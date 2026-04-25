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

## Leaving and Removing a Subscription

```rust
ctx.leave_subscription(id)?;
ctx.remove_subscription(id);
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
