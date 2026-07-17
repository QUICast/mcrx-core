from __future__ import annotations

import asyncio
import os
import socket
import time
import unittest

from mcrx_core import AsyncSubscription, Context, add_reader
from mcrx_core import asyncio as mcrx_asyncio


def _send_ipv4_multicast(group: str, port: int, payload: bytes) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.setsockopt(
            socket.IPPROTO_IP,
            socket.IP_MULTICAST_IF,
            socket.inet_aton("127.0.0.1"),
        )
        sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_LOOP, 1)
        sock.sendto(payload, (group, port))

def _existing_interface_name() -> str:
    names = socket.if_nameindex()
    if not names:
        raise unittest.SkipTest("no network interfaces available")

    for _index, name in names:
        lowered = name.lower()
        if name in ("lo", "lo0") or "loopback" in lowered:
            return name

    return names[0][1]


class _PipeSubscription:
    def __init__(self, packets: list[object] | None = None) -> None:
        self._reader, self._writer = os.pipe()
        self.packets = packets or []
        self.removed = False

    def fileno(self) -> int:
        return self._reader

    def state(self) -> str:
        if self.removed:
            raise LookupError("subscription removed")
        return "joined"

    def recv_nowait(self):
        if self.removed:
            raise LookupError("subscription removed")
        if self.packets:
            return self.packets.pop(0)
        return None

    def signal(self) -> None:
        os.write(self._writer, b"x")

    def close(self) -> None:
        os.close(self._writer)
        os.close(self._reader)


class BindingsTest(unittest.TestCase):
    def test_reader_fd_is_closed_even_after_event_loop_shutdown(self) -> None:
        class ClosedLoop:
            def is_closed(self) -> bool:
                return True

            def remove_reader(self, _fd: int) -> None:
                raise AssertionError("remove_reader must not run on a closed loop")

        reader, writer = os.pipe()
        try:
            mcrx_asyncio._close_reader_fd(ClosedLoop(), reader)  # type: ignore[arg-type,attr-defined]
            with self.assertRaises(OSError):
                os.fstat(reader)
        finally:
            os.close(writer)

    def test_reader_handle_closes_after_event_loop_shutdown(self) -> None:
        loop = asyncio.new_event_loop()
        sub = _PipeSubscription()

        async def setup_reader():
            return add_reader(sub, lambda _packet: None)

        try:
            handle = loop.run_until_complete(setup_reader())
            loop.close()
            handle.close()
            self.assertTrue(handle.closed)
        finally:
            if not loop.is_closed():
                loop.close()
            sub.close()

    def test_add_subscription_parses_numeric_ipv6_interface_index(self) -> None:
        ctx = Context()
        sub = ctx.add_subscription("ff1e::8000:1234", 55129, interface="7")

        self.assertIsNone(sub.interface)
        self.assertEqual(sub.interface_index, 7)

    def test_add_subscription_parses_scoped_ipv6_interface_name(self) -> None:
        ctx = Context()
        interface_name = _existing_interface_name()
        sub = ctx.add_subscription(
            "ff12::8000:1234",
            55128,
            interface=f"fe80::1%{interface_name}",
        )

        self.assertEqual(sub.interface, "fe80::1")
        self.assertEqual(sub.interface_index, socket.if_nametoindex(interface_name))

    def test_context_subscription_receives_packet(self) -> None:
        ctx = Context()
        sub = ctx.add_subscription("239.1.2.3", 55130, interface="127.0.0.1")
        sub.join()

        payload = b"python-binding-packet"
        _send_ipv4_multicast("239.1.2.3", 55130, payload)

        deadline = time.time() + 1.0
        packet = None
        while time.time() < deadline:
            packet = sub.recv_nowait()
            if packet is not None:
                break
            time.sleep(0.01)

        self.assertIsNotNone(packet)
        assert packet is not None
        self.assertEqual(packet.group, "239.1.2.3")
        self.assertEqual(packet.dst_port, 55130)
        self.assertEqual(packet.payload, payload)
        self.assertEqual(sub.join_mode, "asm")
        if hasattr(sub, "fileno"):
            self.assertTrue(sub.fileno() >= 0)
        else:
            self.assertTrue(sub.socket_handle() > 0)

    def test_async_subscription_recv(self) -> None:
        async def run() -> None:
            ctx = Context()
            sub = ctx.add_subscription("239.1.2.4", 55131, interface="127.0.0.1")
            sub.join()

            async_sub = AsyncSubscription(sub)

            loop = asyncio.get_running_loop()
            loop.call_later(
                0.05,
                _send_ipv4_multicast,
                "239.1.2.4",
                55131,
                b"async-python-binding-packet",
            )

            packet = await asyncio.wait_for(async_sub.recv_with_metadata(), timeout=1.0)
            self.assertEqual(packet.packet.group, "239.1.2.4")
            self.assertEqual(packet.packet.payload, b"async-python-binding-packet")

        asyncio.run(run())

    def test_add_reader_callback(self) -> None:
        async def run() -> None:
            ctx = Context()
            sub = ctx.add_subscription("239.1.2.5", 55132, interface="127.0.0.1")
            sub.join()

            received = asyncio.Event()
            payloads: list[bytes] = []

            def on_packet(packet) -> None:
                payloads.append(packet.payload)
                received.set()

            handle = add_reader(sub, on_packet)
            try:
                _send_ipv4_multicast("239.1.2.5", 55132, b"callback-packet")
                await asyncio.wait_for(received.wait(), timeout=1.0)
            finally:
                handle.close()

            self.assertEqual(payloads, [b"callback-packet"])

        asyncio.run(run())

    def test_polling_reader_closes_when_subscription_is_removed(self) -> None:
        class RemovedSubscription:
            def recv_nowait(self):
                raise LookupError("subscription removed")

        async def run() -> None:
            callback_count = 0

            def on_packet(_packet) -> None:
                nonlocal callback_count
                callback_count += 1

            handle = add_reader(
                RemovedSubscription(),
                on_packet,
                poll_interval=0.001,
            )
            await asyncio.sleep(0.01)

            self.assertEqual(callback_count, 0)
            self.assertTrue(handle.closed)

        asyncio.run(run())

    def test_async_recv_exits_when_subscription_is_removed_without_readiness(self) -> None:
        async def run() -> None:
            sub = _PipeSubscription()
            task = asyncio.create_task(
                mcrx_asyncio._recv_async(  # type: ignore[attr-defined]
                    sub,
                    with_metadata=False,
                    loop=None,
                    poll_interval=0.001,
                )
            )

            try:
                await asyncio.sleep(0.02)
                sub.removed = True
                with self.assertRaises(LookupError):
                    await asyncio.wait_for(task, timeout=0.2)
            finally:
                if not task.done():
                    task.cancel()
                sub.close()

        asyncio.run(run())

    def test_add_reader_closes_after_removal_without_readiness(self) -> None:
        async def run() -> None:
            sub = _PipeSubscription()
            handle = add_reader(sub, lambda _packet: None, poll_interval=0.001)

            try:
                sub.removed = True
                deadline = asyncio.get_running_loop().time() + 0.2
                while not handle.closed and asyncio.get_running_loop().time() < deadline:
                    await asyncio.sleep(0.001)
                self.assertTrue(handle.closed)
            finally:
                handle.close()
                sub.close()

        asyncio.run(run())

    def test_closing_reader_from_callback_stops_current_drain(self) -> None:
        async def run() -> None:
            sub = _PipeSubscription([object(), object()])
            calls = 0
            handle = None

            def on_packet(_packet) -> None:
                nonlocal calls
                calls += 1
                assert handle is not None
                handle.close()

            handle = add_reader(sub, on_packet, poll_interval=0.001)
            try:
                sub.signal()
                await asyncio.sleep(0.05)
                self.assertEqual(calls, 1)
                self.assertTrue(handle.closed)
            finally:
                handle.close()
                sub.close()

        asyncio.run(run())

    def test_async_subscription_rejects_invalid_poll_interval(self) -> None:
        with self.assertRaises(ValueError):
            AsyncSubscription(object(), poll_interval=0)  # type: ignore[arg-type]
        with self.assertRaises(ValueError):
            AsyncSubscription(object(), poll_interval=float("nan"))  # type: ignore[arg-type]

    def test_callback_may_return_an_existing_future(self) -> None:
        async def run() -> None:
            loop = asyncio.get_running_loop()
            future = loop.create_future()
            future.set_result(None)
            mcrx_asyncio._schedule_callback_result(  # type: ignore[attr-defined]
                loop,
                lambda _packet: future,
                object(),
            )
            await asyncio.sleep(0)

        asyncio.run(run())

    def test_add_reader_closes_itself_when_subscription_is_removed(self) -> None:
        async def run() -> None:
            loop = asyncio.get_running_loop()
            errors: list[dict[str, object]] = []
            previous_handler = loop.get_exception_handler()
            loop.set_exception_handler(lambda _loop, context: errors.append(context))

            sub = _PipeSubscription()
            sub.removed = True
            payloads: list[bytes] = []
            handle = add_reader(sub, lambda packet: payloads.append(packet.payload))

            try:
                sub.signal()
                await asyncio.sleep(0.05)
            finally:
                handle.close()
                sub.close()
                loop.set_exception_handler(previous_handler)

            self.assertEqual(payloads, [])
            self.assertEqual(errors, [])

        asyncio.run(run())


if __name__ == "__main__":
    unittest.main()
