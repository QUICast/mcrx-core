# mcrx-core-py

`mcrx-core-py` is the Python binding crate for
[`mcrx-core`](../README.md).

It provides:

- `Context` for multicast context and subscription management
- `Subscription` for `join()`, `leave()`, and non-blocking receive
- `Packet`, `PacketWithMetadata`, and `ReceiveMetadata`
- `AsyncSubscription` for await-style `asyncio` integration
- `add_reader()` for callback-style selector integration

The bindings currently expose the normal UDP receive API. Raw packet/shared
capture, metrics snapshots, and caller-provided receive sockets remain Rust-only
APIs for now.

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

On selector-based event loops, the helper registers a duplicated subscription
file descriptor with `loop.add_reader()`. A lightweight lifetime check removes
that reader if the logical subscription is removed, including when no packet
readiness event occurs. On loops where `add_reader()` is unavailable, such as
the default Windows asyncio loop, it falls back to a thin async polling layer
over the same non-blocking receive calls.

Keep the returned `ReaderHandle` for as long as callbacks are wanted and call
`close()` to stop them. Closing from inside a callback immediately stops the
current drain. The handle also closes itself when its subscription disappears.

Generated extension modules and wheels are intentionally ignored by Git. Build
a fresh wheel before distribution rather than reusing an artifact already in a
working tree.
