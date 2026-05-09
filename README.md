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
```

## Documentation

- [Usage Guide](docs/usage.md)
- [Architecture](docs/architecture.md)
- [Demo Binaries](docs/demo.md)
- [Metrics](docs/metrics.md)
- [Design Decisions](docs/design-decisions.md)

## Platform Support

| OS      | ASM | SSM | Notes                                    |
|---------|-----|-----|------------------------------------------|
| macOS   | ✅   | ✅   | Verified                                 |
| Linux   | ✅   | ✅   | Verified                                 |
| Windows | ✅   | ✅   | Build-checked (`x86_64-pc-windows-msvc`) |

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
