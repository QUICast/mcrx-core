# Usage

## Creating a Context

```rust
let mut ctx = Context::new();
```

## Creating Subscription Configurations

### ASM

```rust
let config = SubscriptionConfig::asm(group, port);
```

### SSM

```rust
let config = SubscriptionConfig::ssm(group, source, port);
```

If needed, set the local interface explicitly:

```rust
let mut config = SubscriptionConfig::asm(group, port);
config.interface = Some(interface);
```

## Adding and Joining a Subscription

```rust
let id = ctx.add_subscription(config)?;
ctx.join_subscription(id)?;
```

## Adding a Subscription with an Existing Socket

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

The supplied socket is switched to non-blocking mode, but multicast join/leave
still flows through `join_subscription()` and `leave_subscription()` in this
first integration step.

## Event Loop Integration

There are now two intended integration styles for external event loops.

Borrowing a live socket while the subscription stays inside the `Context`:

```rust
let subscription = ctx.get_subscription(id).unwrap();
let socket = subscription.socket();

#[cfg(unix)]
let raw = subscription.as_raw_fd();

#[cfg(windows)]
let raw = subscription.as_raw_socket();
```

If a registry API requires mutable socket access during registration, use:

```rust
for subscription in ctx.subscriptions_mut() {
    let id = subscription.id();
    let socket = subscription.socket_mut();
    let _ = (id, socket);
}
```

Taking ownership back out of the `Context` for handoff into another runtime:

```rust
let subscription = ctx.take_subscription(id).unwrap();
let parts = subscription.into_parts();

println!("taken subscription {}", parts.id.0);
println!("state at handoff: {:?}", parts.state);

let socket = parts.socket;
```

`take_subscription()` does not implicitly leave the multicast group. It
preserves the current socket ownership and lifecycle state so the caller can
continue receiving immediately in another loop if desired.

## Tokio Integration

Enable the optional `tokio` feature when you want an async wrapper over an
owned subscription:

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- 239.1.2.3 5000
```

The library also exposes a thin adapter type:

```rust
use mcrx_core::TokioSubscription;

let subscription = ctx.take_subscription(id).unwrap();
let subscription = TokioSubscription::new(subscription)?;

let packet = subscription.recv().await?;
let detailed = subscription.recv_with_metadata().await?;
```

On Unix this waits for socket readiness via Tokio's `AsyncFd`. On other
platforms it currently falls back to an async sleep-and-poll loop around the
same non-blocking receive APIs.

## Leaving and Removing a Subscription

```rust
ctx.leave_subscription(id)?;
ctx.remove_subscription(id);
```

If you need the owned subscription back instead of dropping it, use:

```rust
if let Some(subscription) = ctx.take_subscription(id) {
    let socket = subscription.into_socket();
    drop(socket);
}
```

## Receiving from Any Subscription

```rust
if let Some(packet) = ctx.try_recv_any()? {
    println!("received {} bytes", packet.payload.len());
}
```

### Fairness

`try_recv_any()` uses round-robin style fairness across joined subscriptions so repeated calls do not always favor the first subscription.

## Receiving from a Specific Subscription

```rust
let subscription = ctx.get_subscription(id).unwrap();
if let Some(packet) = subscription.try_recv()? {
    println!("received {} bytes", packet.payload.len());
}
```

## Receiving with Richer Metadata

When you need more than source/group/port, use the metadata-aware receive APIs:

```rust
let subscription = ctx.get_subscription(id).unwrap();
if let Some(packet) = subscription.try_recv_with_metadata()? {
    println!("received {} bytes", packet.packet.payload.len());
    println!("socket addr: {:?}", packet.metadata.socket_local_addr);
    println!("configured interface: {:?}", packet.metadata.configured_interface);
    println!("destination ip: {:?}", packet.metadata.destination_local_ip);
    println!("ingress ifindex: {:?}", packet.metadata.ingress_interface_index);
}
```

The `Context` offers matching helpers:

```rust
if let Some(packet) = ctx.try_recv_any_with_metadata()? {
    println!("received on subscription {}", packet.packet.subscription_id.0);
}
```

Today the richer metadata surface exposes:

- the socket's current local bind address
- the configured join interface from `SubscriptionConfig`
- pktinfo-style destination local IP on supported Unix and Windows IPv4 platforms
- pktinfo-style ingress interface index on supported Unix and Windows IPv4 platforms

On platforms where those ancillary control messages are not wired yet, the
pktinfo-derived fields remain `None`.

You can also inspect the local bind address for a subscription:

```rust
let local_addr = subscription.local_addr()?;
println!("bound to {local_addr}");
```

## Batch Receiving

### Bounded batch

```rust
let mut packets = Vec::new();
ctx.try_recv_batch_into(&mut packets, 64)?;
```

### Drain everything currently available

```rust
let mut packets = Vec::new();
ctx.try_recv_all_into(&mut packets)?;
```

### Semantics

All receive APIs are non-blocking:

- `Some(packet)` → packet was available
- `None` → no packet is currently available
- `Err(...)` → actual error

## Multi-subscription Example

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
