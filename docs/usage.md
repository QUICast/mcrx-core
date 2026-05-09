# Usage

## Core Flow

```rust
let mut ctx = Context::new();
let config = SubscriptionConfig::asm(group, port);
let id = ctx.add_subscription(config)?;
ctx.join_subscription(id)?;

if let Some(packet) = ctx.try_recv_any()? {
    println!("received {} bytes", packet.payload.len());
}
```

For SSM:

```rust
let config = SubscriptionConfig::ssm(group, source, port);
```

If needed, set the local interface explicitly:

```rust
let mut config = SubscriptionConfig::asm(group, port);
config.interface = Some(interface.into());
```

The demo receiver binaries also support an explicit ASM interface override via
`--interface`, for example `mcrx_recv_meta ff01::1234 5000 --interface ::1`.

`SubscriptionConfig` can represent either IPv4 or IPv6 addresses. The active
receive path supports IPv4 and IPv6 ASM/SSM. The metadata-aware receive APIs
also expose pktinfo-style metadata for both families on platforms that provide
it.

## IPv6 Group and Interface Guidance

IPv6 multicast is stricter than IPv4 about scope and interface selection.

For IPv6 SSM, use `ff3x::/32` groups:

- `ff31::/16` for interface-local tests on one host
- `ff32::/16` for link-local tests on one L2 link
- `ff35::/16` for site-local tests
- `ff38::/16` for organization-local tests
- `ff3e::/16` for global scope

Prefer dynamic SSM group IDs such as `ff31::8000:1234` or `ff3e::8000:1234`.

For IPv6 SSM, the two important addresses are:

- `source` → the sender's IP address, which the receiver will admit
- `interface` → the receiver's local join interface

On one machine they may be the same. Across machines they usually differ.

Example:

```rust
let mut config = SubscriptionConfig::ssm_v6(group, source, port);
config.interface = Some(interface.into());
```

The receiver binaries accept the same split explicitly:

```bash
cargo run --bin mcrx_recv_meta -- ff3e::8000:1234 5000 <sender-ipv6> --interface <receiver-ipv6>
```

For IPv6 senders, the demo binary treats an IPv6 interface address as both:

- the local source address to bind to
- the interface to use for multicast transmission

That behavior is intentional for SSM, because the receiver filters on the exact
packet source IP. If the sender only chose an interface and let the kernel pick
another source address, the packet could be dropped by the SSM filter.

One practical rule:

- for link-local multicast groups such as `ff32::/16`, send from a link-local
  `fe80::...` source
- for wider-scope groups such as `ff35::/16` or `ff3e::/16`, use a ULA or
  global IPv6 source that is valid on that network

## Existing Sockets

When an integration needs to create or bind the socket itself, pass it into the
context directly. The socket must already be bound to `config.dst_port`.

```rust
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4};

let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 5000);

let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
socket.set_reuse_address(true)?;
socket.bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 5000)))?;

let id = ctx.add_subscription_with_socket(config, socket)?;
ctx.join_subscription(id)?;
```

The supplied socket is switched to non-blocking mode, while multicast
join/leave still flows through `join_subscription()` and `leave_subscription()`.

## Receiving

From any joined subscription:

```rust
if let Some(packet) = ctx.try_recv_any()? {
    println!("received {} bytes", packet.payload.len());
}
```

From one specific subscription:

```rust
let subscription = ctx.get_subscription(id).unwrap();
if let Some(packet) = subscription.try_recv()? {
    println!("received {} bytes", packet.payload.len());
}
```

`try_recv_any()` uses round-robin style fairness across joined subscriptions so
repeated calls do not always favor the first subscription.

## Event Loop Integration

Borrow a live socket while the subscription stays inside the `Context`:

```rust
let subscription = ctx.get_subscription(id).unwrap();
let socket = subscription.socket();

#[cfg(unix)]
let raw = subscription.as_raw_fd();

#[cfg(windows)]
let raw = subscription.as_raw_socket();
```

If a registry API needs mutable socket access during registration:

```rust
for subscription in ctx.subscriptions_mut() {
    let id = subscription.id();
    let socket = subscription.socket_mut();
    let _ = (id, socket);
}
```

Extract ownership for handoff into another runtime:

```rust
let subscription = ctx.take_subscription(id).unwrap();
let parts = subscription.into_parts();

println!("taken subscription {}", parts.id.0);
println!("state at handoff: {:?}", parts.state);

let socket = parts.socket;
```

`take_subscription()` does not implicitly leave the multicast group. It
preserves the current lifecycle state so the caller can continue receiving
immediately in another loop if desired.

## Tokio Integration

Enable the optional `tokio` feature when you want an async wrapper over an
owned subscription:

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- 239.1.2.3 5000
```

The library also exposes `TokioSubscription`:

```rust
use mcrx_core::TokioSubscription;

let subscription = ctx.take_subscription(id).unwrap();
let mut subscription = TokioSubscription::new(subscription)?;

let packet = subscription.recv().await?;
let detailed = subscription.recv_with_metadata().await?;
```

`TokioSubscription` is an owned single-consumer receive handle, so its async
receive methods take `&mut self`.

On Unix this waits for socket readiness via Tokio's `AsyncFd`. On other
platforms it currently falls back to an async sleep-and-poll loop around the
same non-blocking receive APIs.

## Optional Receive Metadata

When you need more delivery context than source, group, port, and payload, use
the metadata-aware receive APIs:

```rust
let subscription = ctx.get_subscription(id).unwrap();
if let Some(packet) = subscription.try_recv_with_metadata()? {
    println!("socket addr: {:?}", packet.metadata.socket_local_addr);
    println!("destination ip: {:?}", packet.metadata.destination_local_ip);
}
```

The `Context` offers matching helpers:

```rust
if let Some(packet) = ctx.try_recv_any_with_metadata()? {
    println!("received on subscription {}", packet.packet.subscription_id.0);
}
```

Today the optional metadata surface exposes:

- the socket's current local bind address
- the configured join interface from `SubscriptionConfig`
- pktinfo-style destination local IP on supported Unix and Windows IPv4 platforms
- pktinfo-style ingress interface index on supported Unix and Windows IPv4 platforms

On platforms where those ancillary control messages are not wired yet, the
pktinfo-derived fields remain `None`.

You can also inspect the local bind address directly:

```rust
let local_addr = subscription.local_addr()?;
println!("bound to {local_addr}");
```

## Batch Receiving

Bounded batch:

```rust
let mut packets = Vec::new();
ctx.try_recv_batch_into(&mut packets, 64)?;
```

Drain everything currently available:

```rust
let mut packets = Vec::new();
ctx.try_recv_all_into(&mut packets)?;
```

All receive APIs are non-blocking:

- `Some(packet)` means a packet was available
- `None` means no packet is currently available
- `Err(...)` means an actual receive failure occurred

## Leaving, Removing, and Taking Ownership

```rust
ctx.leave_subscription(id)?;
ctx.remove_subscription(id);
```

If you need the owned subscription back instead of dropping it:

```rust
if let Some(subscription) = ctx.take_subscription(id) {
    let socket = subscription.into_socket();
    drop(socket);
}
```

## Multiple Subscriptions

```rust
let mut ctx = Context::new();

let id1 = ctx.add_subscription(SubscriptionConfig::asm(group1, 5000))?;
let id2 = ctx.add_subscription(SubscriptionConfig::asm(group2, 5001))?;

ctx.join_subscription(id1)?;
ctx.join_subscription(id2)?;

if let Some(packet) = ctx.try_recv_any()? {
    println!("received on subscription {}", packet.subscription_id.0);
}
```
