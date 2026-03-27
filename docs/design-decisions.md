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
- `Context::try_recv_any()`
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

1. create and bind socket
2. add subscription
3. join group when ready
4. leave group without destroying the socket
5. remove subscription when done

This is cleaner for testing, metrics, and future lifecycle-sensitive platforms.

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

