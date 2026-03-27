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
