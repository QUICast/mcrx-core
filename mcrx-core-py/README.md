# mcrx-core-py

`mcrx-core-py` is the Python binding crate for
[`mcrx-core`](../README.md).

It provides:

- `Context` for multicast context and subscription management
- `Subscription` for `join()`, `leave()`, and non-blocking receive
- `Packet`, `PacketWithMetadata`, and `ReceiveMetadata`
- `AsyncSubscription` for await-style `asyncio` integration
- `add_reader()` for callback-style selector integration

## Build

Install from the repository root:

```bash
pip install ./mcrx-core-py
```

For local development:

```bash
cd mcrx-core-py
maturin develop
```

## Example

```python
from mcrx_core import AsyncSubscription, Context

ctx = Context()
sub = ctx.add_subscription("239.1.2.3", 5000, interface="192.168.1.20")
sub.join()

packet = sub.recv_nowait()

async_sub = AsyncSubscription(sub)
packet = await async_sub.recv_with_metadata()
```

On selector-based event loops, the helper uses `loop.add_reader()` with the
subscription file descriptor. On loops where that API is unavailable, such as
the default Windows asyncio loop, it falls back to a thin async polling layer
over the same non-blocking receive calls.
