from __future__ import annotations

import asyncio
import inspect
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
        loop.create_task(result)


def _duplicate_reader_fd(subscription: Subscription) -> int | None:
    try:
        return os.dup(subscription.fileno())
    except (AttributeError, OSError):
        return None


def _close_reader_fd(loop: asyncio.AbstractEventLoop, reader_fd: int) -> None:
    try:
        if not loop.is_closed():
            loop.remove_reader(reader_fd)
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
        self._poll_interval = poll_interval

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
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    if hasattr(running_loop, "add_reader") and hasattr(subscription, "fileno"):
        while True:
            packet = recv_nowait()
            if packet is not None:
                return packet

            reader_fd = _duplicate_reader_fd(subscription)
            if reader_fd is None:
                await asyncio.sleep(poll_interval)
                continue

            future = running_loop.create_future()

            def on_ready() -> None:
                if not future.done():
                    future.set_result(None)

            running_loop.add_reader(reader_fd, on_ready)
            try:
                await future
            finally:
                _close_reader_fd(running_loop, reader_fd)
    else:
        while True:
            packet = recv_nowait()
            if packet is not None:
                return packet
            await asyncio.sleep(poll_interval)


def add_reader(
    subscription: Subscription,
    callback: PacketCallback,
    *,
    with_metadata: bool = False,
    loop: asyncio.AbstractEventLoop | None = None,
    poll_interval: float = 0.01,
) -> ReaderHandle:
    running_loop = loop or asyncio.get_running_loop()
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    if hasattr(running_loop, "add_reader") and hasattr(subscription, "fileno"):
        reader_fd = _duplicate_reader_fd(subscription)
        if reader_fd is None:
            task = running_loop.create_task(
                _poll_reader_loop(
                    subscription,
                    callback,
                    with_metadata=with_metadata,
                    poll_interval=poll_interval,
                )
            )
            return ReaderHandle(task.cancel)

        closed = False

        def close_reader() -> None:
            nonlocal closed

            if closed:
                return

            closed = True
            _close_reader_fd(running_loop, reader_fd)

        def on_ready() -> None:
            try:
                while True:
                    packet = recv_nowait()
                    if packet is None:
                        break
                    _schedule_callback_result(running_loop, callback, packet)
            except LookupError:
                close_reader()
            except Exception as exc:  # pragma: no cover - exercised by loop
                running_loop.call_exception_handler(
                    {
                        "message": "mcrx_core subscription callback failed",
                        "exception": exc,
                    }
                )

        running_loop.add_reader(reader_fd, on_ready)
        return ReaderHandle(close_reader)

    task = running_loop.create_task(
        _poll_reader_loop(
            subscription,
            callback,
            with_metadata=with_metadata,
            poll_interval=poll_interval,
        )
    )
    return ReaderHandle(task.cancel)


async def _poll_reader_loop(
    subscription: Subscription,
    callback: PacketCallback,
    *,
    with_metadata: bool,
    poll_interval: float,
) -> None:
    recv_nowait = (
        subscription.recv_with_metadata_nowait
        if with_metadata
        else subscription.recv_nowait
    )

    try:
        while True:
            packet = recv_nowait()
            if packet is None:
                await asyncio.sleep(poll_interval)
                continue

            _schedule_callback_result(asyncio.get_running_loop(), callback, packet)
    except LookupError:
        return
    except asyncio.CancelledError:
        raise
