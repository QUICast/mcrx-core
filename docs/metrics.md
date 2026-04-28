# Metrics

Metrics are optional and sit outside the core receive API.

## Enabling Metrics

```bash
cargo run --features metrics --bin mcrx_recv -- 239.1.2.3 5000
```

## Model

The metrics system is split into three layers:

### Snapshot

A snapshot is a point-in-time view.

Counter fields in a snapshot are cumulative.

For `ContextMetricsSnapshot`, packet, byte, would-block, receive-error, join,
leave, and batch counters are true context-lifetime totals. They are not
recomputed from the currently active subscriptions, and they do not decrease
when a subscription is removed.

Gauge-like fields in a snapshot reflect current state only:

- `active_subscriptions`
- `joined_subscriptions`

### Delta

A delta is computed between two snapshots of the same metric type.

Delta fields represent only the change over the sampled interval:

- packets received during the interval
- bytes received during the interval
- receive errors during the interval
- joins, leaves, and batch activity during the interval

### Sampler

A sampler stores the previous snapshot and computes deltas across repeated
samples.

The first call returns `None` because a delta requires two snapshots.

## Cumulative Totals

At the context level, these snapshot fields are cumulative totals:

- `total_packets_received`
- `total_bytes_received`
- `total_would_block_count`
- `total_receive_errors`
- `total_join_count`
- `total_leave_count`
- `batch_calls`
- `batch_packets_received`

At the subscription level, the per-subscription snapshot counters remain
cumulative for the lifetime of that subscription object.

## Rates

Delta types expose average interval rates such as:

- `packets_per_sec()`
- `bytes_per_sec()`
- `would_block_per_sec()`
- `receive_errors_per_sec()`

These are computed from delta counters divided by the sampled interval.

## CLI Integration

`mcrx_recv` can periodically emit:

- terminal summaries
- JSONL file output

Configured via:

- `MCRX_METRICS_SUMMARY_SECS`
- `MCRX_METRICS_SUMMARY_FILE`

## JSONL Schema

The JSONL output now uses explicit names for counter totals and interval deltas.

Packet and byte fields:

- `packets_received_total`
- `bytes_received_total`
- `packets_received_delta`
- `bytes_received_delta`
- `packets_per_sec`
- `bytes_per_sec`

Other counter-style fields follow the same explicit pattern where emitted:

- `would_block_count_total`
- `would_block_count_delta`
- `receive_errors_total`
- `receive_errors_delta`
- `join_count_total`
- `join_count_delta`
- `leave_count_total`
- `leave_count_delta`
- `batch_calls_total`
- `batch_calls_delta`
- `batch_packets_received_total`
- `batch_packets_received_delta`

Current-state gauge fields remain:

- `active_subscriptions`
- `joined_subscriptions`

## Breaking Schema Change

For downstream JSONL consumers, the old ambiguous counter keys such as
`packets_received`, `bytes_received`, `would_block_count`, `receive_errors`,
`join_count`, `leave_count`, `batch_calls`, and `batch_packets_received` are no
longer emitted by `mcrx_recv`.

Consumers should switch to the explicit `*_total` and `*_delta` fields.
