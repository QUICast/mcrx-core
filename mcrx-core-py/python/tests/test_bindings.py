from __future__ import annotations

import asyncio
import socket
import time
import unittest

from mcrx_core import AsyncSubscription, Context, add_reader


def _send_ipv4_multicast(group: str, port: int, payload: bytes) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP) as sock:
        sock.sendto(payload, (group, port))


class BindingsTest(unittest.TestCase):
    def test_context_subscription_receives_packet(self) -> None:
        ctx = Context()
        sub = ctx.add_subscription("239.1.2.3", 55130)
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
        self.assertTrue(sub.fileno() >= 0)

    def test_async_subscription_recv(self) -> None:
        async def run() -> None:
            ctx = Context()
            sub = ctx.add_subscription("239.1.2.4", 55131)
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
            sub = ctx.add_subscription("239.1.2.5", 55132)
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


if __name__ == "__main__":
    unittest.main()
