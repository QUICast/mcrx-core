# Design Decisions

## Context vs Subscription

A single `Subscription` models one multicast receive path.

A `Context` exists to coordinate multiple subscriptions and provide behavior that does not belong to a single
subscription, including:

- fairness across subscriptions
- batch receive
- aggregation of metrics
- centralized lifecycle management
- a stable top-level handle for integrations

This means callers can still think in terms of individual subscriptions, while higher-level integrations can work with
one context object.

## Non-blocking API

The library uses a pull-based, non-blocking API:

- `Subscription::try_recv()`
- `Subscription::try_recv_with_metadata()`
- `Context::try_recv_any()`
- `Context::try_recv_any_with_metadata()`
- `Context::try_recv_batch_into()`
- `Context::try_recv_all_into()`

Reasons:

- no runtime dependency
- works in sync and async environments
- easy integration into custom event loops
- suitable for desktop, server, and mobile use

## Explicit Join / Leave Lifecycle

Adding a subscription does not automatically join the multicast group.

This separation makes the lifecycle explicit:

1. create and bind socket, or provide an existing bound socket
2. add subscription
3. join group when ready
4. leave group without destroying the socket
5. remove subscription when done

This is cleaner for testing, metrics, and future lifecycle-sensitive platforms.

## Socket Ownership Boundary

`Context::add_subscription()` remains the convenience path that creates and binds
the socket internally.

`Context::add_subscription_with_socket()` is the lower-level integration path for
embedders that need to control socket creation themselves. The current step keeps
join/leave/receive behavior inside `mcrx-core`, while routing raw socket
operations through the `platform` module so future IPv6, richer receive metadata,
and alternate backends can plug in with less churn.

For event-loop integration, the library now supports two ownership modes:

- borrow the socket from a live `Subscription` via `socket()`, `socket_mut()`,
  `as_raw_fd()`, or `as_raw_socket()`
- extract the whole `Subscription` from a `Context` via `take_subscription()`
  and move its owned socket into another loop or runtime

This keeps the default API simple while making ownership transfer explicit
instead of forcing callers to rebuild subscription state around a raw socket.

An optional Tokio layer now builds on top of that ownership model instead of
changing the core API. `TokioSubscription` wraps an owned `Subscription` after
`take_subscription()`, so the async path reuses the same join/leave/receive
logic rather than introducing a second socket management model.

## Staged Receive Metadata

The original `Packet` type stays small and stable for callers that only need the
core addressing tuple plus payload.

The richer path uses `PacketWithMetadata`, which wraps a `Packet` plus a
non-exhaustive `ReceiveMetadata` struct. The first step intentionally exposes
metadata in layers:

- socket local address
- configured join interface
- pktinfo-style destination local IP on supported Unix and Windows IPv4 platforms
- pktinfo-style ingress interface index on supported Unix and Windows IPv4 platforms

Where the platform layer does not provide those ancillary messages yet, the
pktinfo-derived fields remain `None`. This lets integrators adopt the richer
type now without forcing a breaking redesign each time a new OS-specific
metadata source gets wired in.

## Metrics Model

The metrics model is intentionally split into:

- cumulative snapshots
- deltas between snapshots
- sampler helpers for repeated sampling

This avoids hidden mutable state inside snapshots while still making interval-based analysis easy.

### Feature-gated Metrics

Metrics are behind the `metrics` Cargo feature so that:

- default builds have zero metrics overhead
- core receive paths stay lean
- users opt in only when needed

### Why First Sampler Call Returns `None`

A delta needs a previous sample.

Returning `None` on the first sampler call avoids:

- fake zero-duration intervals
- misleading rates
- hidden baseline assumptions
