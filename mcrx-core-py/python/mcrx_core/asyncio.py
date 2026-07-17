from __future__ import annotations

import asyncio
import inspect
import math
import os
from collections.abc import Callable
from typing import Any

from ._mcrx_core import Packet, PacketWithMetadata, Subscription

PacketCallback = Callable[[Packet | PacketWithMetadata], Any]


def _schedule_callback_result(
    loop: asyncio.AbstractEventLoop,
    callback: PacketCallback,
    packet: Packet | PacketWithMetadata,
) -> None:
    result = callback(packet)
    if inspect.isawaitable(result):
        asyncio.ensure_future(result, loop=loop)


def _validate_poll_interval(poll_interval: float) -> float:
    if not math.isfinite(poll_interval) or poll_interval <= 0:
        raise ValueError("poll_interval must be a positive finite number")
    return poll_interval


def _duplicate_reader_fd(subscription: Subscription) -> int | None:
    try:
        return os.dup(subscription.fileno())
    except (AttributeError, OSError):
        return None


def _close_reader_fd(loop: asyncio.AbstractEventLoop, reader_fd: int) -> None:
    try:
        if not loop.is_closed():
            loop.remove_reader(reader_fd)
    except NotImplementedError:
        pass
    except (RuntimeError, ValueError):
        if not loop.is_closed():
            raise
    finally:
        try:
            os.close(reader_fd)
        except OSError:
            pass


class ReaderHandle:
    def __init__(self, close_cb: Callable[[], None]) -> None:
        self._close_cb = close_cb
        self._closed = False

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._close_cb()

    @property
    def closed(self) -> bool:
        return self._closed

    def __enter__(self) -> "ReaderHandle":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        self.close()


class AsyncSubscription:
    def __init__(
        self,
        subscription: Subscription,
        *,
        loop: asyncio.AbstractEventLoop | None = None,
        poll_interval: float = 0.01,
    ) -> None:
        self.subscription = subscription
        self._loop = loop
        self._poll_interval = _validate_poll_interval(poll_interval)

    async def recv(self) -> Packet:
        return await _recv_async(
            self.subscription,
            with_metadata=False,
            loop=self._loop,
            poll_interval=self._poll_interval,
        )

    async def recv_with_metadata(self) -> PacketWithMetadata:
        return await _recv_async(
            self.subscription,
            with_metadata=True,
            loop=self._loop,
            poll_interval=self._poll_interval,
        )

    def add_reader(
        self,
        callback: PacketCallback,
        *,
        with_metadata: bool = False,
    ) -> ReaderHandle:
        return add_reader(
            self.subscription,
            callback,
            with_metadata=with_metadata,
            loop=self._loop,
            poll_interval=self._poll_interval,
        )


async def _recv_async(
    subscription: Subscription,
    *,
    with_metadata: bool,
    loop: asyncio.AbstractEventLoop | None,
    poll_interval: float,
) -> Packet | PacketWithMetadata:
    running_loop = loop or asyncio.get_running_loop()
    poll_interval = _validate_poll_interval(poll_interval)
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    selector_available = hasattr(running_loop, "add_reader") and hasattr(
        subscription, "fileno"
    )

    while True:
        packet = recv_nowait()
        if packet is not None:
            return packet

        if not selector_available:
            await asyncio.sleep(poll_interval)
            continue

        reader_fd = _duplicate_reader_fd(subscription)
        if reader_fd is None:
            selector_available = False
            await asyncio.sleep(poll_interval)
            continue

        future = running_loop.create_future()

        def on_ready() -> None:
            if not future.done():
                future.set_result(None)

        try:
            running_loop.add_reader(reader_fd, on_ready)
        except (AttributeError, NotImplementedError):
            selector_available = False
            _close_reader_fd(running_loop, reader_fd)
            await asyncio.sleep(poll_interval)
            continue

        try:
            await asyncio.wait_for(future, timeout=poll_interval)
        except asyncio.TimeoutError:
            # A duplicated descriptor survives removal of the subscription's
            # original socket. Periodically retry recv_nowait() so removal is
            # observed even when no readiness event arrives.
            pass
        finally:
            _close_reader_fd(running_loop, reader_fd)


def add_reader(
    subscription: Subscription,
    callback: PacketCallback,
    *,
    with_metadata: bool = False,
    loop: asyncio.AbstractEventLoop | None = None,
    poll_interval: float = 0.01,
) -> ReaderHandle:
    running_loop = loop or asyncio.get_running_loop()
    poll_interval = _validate_poll_interval(poll_interval)
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    if hasattr(running_loop, "add_reader") and hasattr(subscription, "fileno"):
        reader_fd = _duplicate_reader_fd(subscription)
        if reader_fd is None:
            return _start_poll_reader(
                running_loop,
                subscription,
                callback,
                with_metadata=with_metadata,
                poll_interval=poll_interval,
            )

        lifetime_timer: asyncio.TimerHandle | None = None
        subscription_state = getattr(subscription, "state", None)

        def close_reader() -> None:
            _close_reader_fd(running_loop, reader_fd)
            if lifetime_timer is not None:
                lifetime_timer.cancel()

        handle = ReaderHandle(close_reader)

        def schedule_lifetime_check() -> None:
            nonlocal lifetime_timer
            if (
                not handle.closed
                and not running_loop.is_closed()
                and callable(subscription_state)
            ):
                lifetime_timer = running_loop.call_later(
                    poll_interval, check_subscription_lifetime
                )

        def check_subscription_lifetime() -> None:
            nonlocal lifetime_timer
            lifetime_timer = None
            if handle.closed:
                return

            state = subscription_state
            if not callable(state):
                return

            try:
                state()
            except LookupError:
                handle.close()
            except Exception as exc:  # pragma: no cover - exercised by loop
                running_loop.call_exception_handler(
                    {
                        "message": "mcrx_core subscription lifetime check failed",
                        "exception": exc,
                    }
                )
                handle.close()
            else:
                schedule_lifetime_check()

        def on_ready() -> None:
            try:
                while not handle.closed:
                    packet = recv_nowait()
                    if packet is None:
                        break
                    _schedule_callback_result(running_loop, callback, packet)
            except LookupError:
                handle.close()
            except Exception as exc:  # pragma: no cover - exercised by loop
                running_loop.call_exception_handler(
                    {
                        "message": "mcrx_core subscription callback failed",
                        "exception": exc,
                    }
                )
                handle.close()

        try:
            running_loop.add_reader(reader_fd, on_ready)
        except (AttributeError, NotImplementedError):
            handle.close()
            return _start_poll_reader(
                running_loop,
                subscription,
                callback,
                with_metadata=with_metadata,
                poll_interval=poll_interval,
            )

        schedule_lifetime_check()
        return handle

    return _start_poll_reader(
        running_loop,
        subscription,
        callback,
        with_metadata=with_metadata,
        poll_interval=poll_interval,
    )


def _start_poll_reader(
    loop: asyncio.AbstractEventLoop,
    subscription: Subscription,
    callback: PacketCallback,
    *,
    with_metadata: bool,
    poll_interval: float,
) -> ReaderHandle:
    timer: asyncio.TimerHandle | None = None

    def close_timer() -> None:
        if timer is not None:
            timer.cancel()

    handle = ReaderHandle(close_timer)
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    def schedule_poll(delay: float) -> None:
        nonlocal timer
        if not handle.closed and not loop.is_closed():
            timer = loop.call_later(delay, poll)

    def poll() -> None:
        nonlocal timer
        timer = None
        if handle.closed:
            return

        try:
            while not handle.closed:
                packet = recv_nowait()
                if packet is None:
                    schedule_poll(poll_interval)
                    return
                _schedule_callback_result(loop, callback, packet)
        except LookupError:
            handle.close()
        except Exception as exc:  # pragma: no cover - exercised by loop
            loop.call_exception_handler(
                {
                    "message": "mcrx_core subscription callback failed",
                    "exception": exc,
                }
            )
            handle.close()

    schedule_poll(0)
    return handle
