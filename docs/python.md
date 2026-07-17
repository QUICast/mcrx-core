# Python Bindings

The Python bindings live in the sibling workspace crate `mcrx-core-py`.

## Build

From the repository root:

```bash
pip install ./mcrx-core-py
```

For local development:

```bash
cd mcrx-core-py
maturin develop
```

## Binding Shape

The Python API is intentionally centered on the multicast use case rather than
on a direct transliteration of the Rust ownership model.

Core objects:

- `Context`
- `Subscription`
- `Packet`
- `PacketWithMetadata`
- `ReceiveMetadata`

Async helpers:

- `AsyncSubscription`
- `add_reader()`

That gives Python callers the four pieces that tend to matter most:

1. create and manage a multicast context
2. join and leave subscriptions
3. receive packets with source, group, port, and optional pktinfo-style metadata
4. integrate those subscriptions into `asyncio`

The current binding scope is the normal UDP receive API. Raw packet and shared
raw capture contexts, metrics snapshots, and caller-provided receive sockets
remain available through the Rust API only.

## Basic Example

```python
from mcrx_core import Context

ctx = Context()
sub = ctx.add_subscription("239.1.2.3", 5000, interface="192.168.1.20")
sub.join()

packet = sub.recv_nowait()
if packet is not None:
    print(packet.source_addr, packet.source_port, packet.group, packet.payload)
```

For SSM:

```python
sub = ctx.add_subscription(
    "ff3e::8000:1234",
    5000,
    source="fd00::10",
    interface="fd00::20",
)
sub.join()
```

IPv4 SSM uses `232.0.0.0/8` groups. IPv6 SSM uses `ff3x::/32` groups. Passing
a source with an ASM-range group such as `239.x.x.x` is rejected before join.

## Asyncio

For direct await-style use:

```python
from mcrx_core import AsyncSubscription

async_sub = AsyncSubscription(sub)
packet = await async_sub.recv()
detailed = await async_sub.recv_with_metadata()
```

For callback-style use:

```python
from mcrx_core import add_reader

handle = add_reader(sub, lambda packet: print(packet.payload))

# Later, or from inside the callback:
handle.close()
```

### Event Loop Behavior

On selector-based loops, the helper gives `loop.add_reader()` a duplicated
subscription file descriptor. The duplicate prevents stale descriptor reuse,
while a lightweight lifetime check closes it when the logical subscription is
removed even if no further readiness event arrives.

On platforms or loops where `add_reader()` is not available, such as the
default Windows asyncio loop, it falls back to a thin polling timer over
the same non-blocking `recv_nowait()` methods.

That keeps the Rust core runtime-agnostic while still giving Python users an
event-loop story out of the box.

The returned `ReaderHandle` owns the registration. Calling `close()` is
idempotent, may be done inside a packet callback, and stops the current drain
before another callback is invoked. Reader handles also close themselves when
their subscription is removed.

## Notes

- `mcrx-core` remains a pure Rust crate with no PyO3 or `cdylib` packaging.
- `mcrx-core-py` depends on `mcrx-core` by path inside the same workspace.
- The Python bindings are layered on top of the same non-blocking receive path
  as the Rust API.
- `Context.recv_any_nowait()` and per-subscription receive methods are both
  exposed.
- `Subscription.fileno()` is available on Unix for applications that want to
  register their own selector or callback integrations.
- Generated extension modules and wheels are not source artifacts. Build a
  fresh wheel before distribution rather than relying on an ignored local file.
