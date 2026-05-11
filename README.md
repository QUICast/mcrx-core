# mcrx-core

`mcrx-core` is a runtime-agnostic and portable multicast receiver library for
IPv4 and IPv6 ASM/SSM.

It is built for applications and integrations that want a small multicast
receive core with explicit lifecycle and socket ownership control.

The receive path supports IPv4 and IPv6 ASM/SSM, including pktinfo-style
receive metadata on platforms that expose it.

## Highlights

- IPv4 ASM and SSM receive support
- IPv6 ASM and SSM receive support
- Non-blocking receive API
- Explicit subscription lifecycle: `add`, `join`, `leave`, `remove`
- Multiple concurrent subscriptions with fair receive across them
- Caller-provided socket support
- Event-loop friendly socket borrowing and extraction APIs
- Optional Tokio adapter via the `tokio` feature
- Optional Python bindings with an asyncio helper via the sibling `mcrx-core-py` crate
- Optional receive metadata on platforms that expose it
- Optional metrics via the `metrics` feature

## Install

```bash
cargo add mcrx-core
```

With the optional Tokio adapter:

```bash
cargo add mcrx-core --features tokio
```

With optional metrics:

```bash
cargo add mcrx-core --features metrics
```

## Python Bindings

Python bindings live in the sibling workspace crate
[`mcrx-core-py`](mcrx-core-py/README.md).

That split keeps `mcrx-core` as a pure Rust crate while still shipping a
Python-friendly API with:

- `Context` and `Subscription`
- packet and receive metadata objects
- `AsyncSubscription`
- `add_reader()` for callback-style asyncio integration

## Quick Start

```rust
use mcrx_core::{Context, SubscriptionConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();

let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 5000);
let id = ctx.add_subscription(config) ?;
ctx.join_subscription(id) ?;

if let Some(packet) = ctx.try_recv_any() ? {
println ! ("received {} bytes", packet.payload.len());
}
```

## Existing Sockets

Use `add_subscription_with_socket()` when you need to create or bind the socket
yourself:

```rust
use mcrx_core::{Context, SubscriptionConfig};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4};

let mut ctx = Context::new();
let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 5000);

let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) ?;
socket.set_reuse_address(true) ?;
socket.bind( & SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 5000))) ?;

let id = ctx.add_subscription_with_socket(config, socket) ?;
ctx.join_subscription(id) ?;
```

## Event Loop Integration

Borrow the live socket from a subscription:

```rust
let subscription = ctx.get_subscription(id).unwrap();
let socket = subscription.socket();

#[cfg(unix)]
let raw = subscription.as_raw_fd();
```

Or extract the subscription and move it into another loop or runtime:

```rust
let subscription = ctx.take_subscription(id).unwrap();
let parts = subscription.into_parts();
let socket = parts.socket;
```

## Tokio Integration

With the `tokio` feature enabled, you can wrap an extracted subscription and
await packets asynchronously:

```rust
use mcrx_core::TokioSubscription;

let subscription = ctx.take_subscription(id).unwrap();
let mut subscription = TokioSubscription::new(subscription) ?;
let packet = subscription.recv().await?;
```

`TokioSubscription` is an owned single-consumer receive handle, so its async
receive methods take `&mut self`.

Run the Tokio example with:

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- 239.1.2.3 5000
```

## Optional Receive Metadata

If you need more delivery context than source, group, port, and payload, use
the metadata-aware receive APIs:

```rust
let subscription = ctx.get_subscription(id).unwrap();
if let Some(packet) = subscription.try_recv_with_metadata() ? {
println ! ("socket addr: {:?}", packet.metadata.socket_local_addr);
println ! ("destination ip: {:?}", packet.metadata.destination_local_ip);
}
```

## Demo Binaries

Basic receiver:

```bash
cargo run --bin mcrx_recv -- 239.1.2.3 5000
cargo run --bin mcrx_recv -- ff01::1234 5000 --interface ::1
cargo run --bin mcrx_recv -- ff32::8000:1234 5000 --interface fe80::1%en0
cargo run --bin mcrx_recv -- ff3e::8000:1234 5000 --interface 7
```

Sender:

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello
cargo run --bin mcrx_send -- ff01::1234 5000 hello 1000 ::1
```

Tokio receiver:

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- 239.1.2.3 5000
```

Metadata inspection receiver:

```bash
cargo run --bin mcrx_recv_meta -- 239.1.2.3 5000
cargo run --bin mcrx_recv_meta -- ff01::1234 5000 --interface ::1
cargo run --bin mcrx_recv_meta -- ff32::8000:1234 5000 --interface fe80::1%en0
cargo run --bin mcrx_recv_meta -- ff31::8000:1234 5000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10 --interface fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
```

## IPv6 SSM Notes

For IPv6 SSM, use `ff3x::/32` groups. The `x` nibble is the multicast scope:

- `ff31::/16` → interface-local, good for same-host tests
- `ff32::/16` → link-local, only for the local L2 link
- `ff35::/16` → site-local
- `ff38::/16` → organization-local
- `ff3e::/16` → global scope

Prefer dynamic SSM group IDs such as `ff31::8000:1234` or `ff3e::8000:1234`.

For receivers:

- the SSM `source` is the sender's IP address
- the `interface` is the receiver's local join interface
- on one machine those may be the same
- across machines they usually differ
- for IPv6 ASM or SSM, the receiver CLIs accept `--interface ::1`,
  `--interface fe80::1%7`, `--interface fe80::1%en0`, or a bare interface
  index such as `--interface 7`

For senders:

- when you pass an IPv6 address to `mcrx_send`, the sender binds to that exact
  local IPv6 address and also selects the corresponding multicast interface
- this matters for SSM, because the receiver filters on the exact packet source
- for link-local SSM groups such as `ff32::/16`, send from a link-local
  `fe80::...` source
- for wider-scope groups such as `ff35::/16` or `ff3e::/16`, use a ULA or
  global IPv6 source that is valid on that network

Same-host IPv6 SSM example:

```bash
cargo run --bin mcrx_recv_meta -- ff31::8000:1234 5000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10 --interface fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
cargo run --bin mcrx_send -- ff31::8000:1234 5000 hello-v6 1000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
```

Cross-machine IPv6 SSM example on the same network:

```bash
# sender host
cargo run --bin mcrx_send -- ff3e::8000:1234 5000 hello-v6 1000 <sender-ipv6>

# receiver host
cargo run --bin mcrx_recv_meta -- ff3e::8000:1234 5000 <sender-ipv6> --interface <receiver-ipv6>
```

## Documentation

- [Usage Guide](docs/usage.md)
- [Architecture](docs/architecture.md)
- [Demo Binaries](docs/demo.md)
- [Metrics](docs/metrics.md)
- [Python Bindings](docs/python.md)
- [Design Decisions](docs/design-decisions.md)

## Platform Support

| OS      | ASM | SSM | Notes                                    |
|---------|-----|-----|------------------------------------------|
| macOS   | ✅   | ✅   | Verified                                 |
| Linux   | ✅   | ✅   | Verified                                 |
| Windows | ✅   | ✅   | Verified                                 |

IPv6 ASM/SSM support and pktinfo-style receive metadata are wired into the
receive path on the same platforms.

## Compatibility

ASM cross-platform compatibility:

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Windows           | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Linux             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Android           | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| iOS               | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |

SSM cross-platform compatibility:

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Windows           | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Linux             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Android           | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| iOS               | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |

## Notes

- macOS may temporarily emit IGMPv2 reports in some SSM setups
- that can break SSM behavior on the network until the host state recovers

## License

BSD 2-Clause
