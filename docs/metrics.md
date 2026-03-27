# Metrics

## Enabling Metrics

Metrics are optional and enabled with:

```bash
cargo run --features metrics --bin mcrx_recv -- 239.1.2.3 5000
```

## Model

The metrics system is split into three layers:

### Snapshot

Snapshots are cumulative point-in-time values.

Examples:

- total packets received
- total bytes received
- total joins/leaves

### Delta

A delta is computed between two snapshots.

Examples:

- packets received during the interval
- bytes received during the interval
- receive errors during the interval

### Sampler

A sampler stores the previous snapshot and computes deltas across repeated samples.

## Why the First Sampler Call Returns `None`

A delta requires two snapshots.

The first call stores the baseline snapshot and returns `None` so that later samples can be compared against it.

This avoids:

- fake zero-duration intervals
- misleading rates
- hidden assumptions

## Counter vs Gauge Semantics

### Counters

Counters are suitable for deltas and rates:

- packets received
- bytes received
- receive errors
- joins/leaves
- batch calls

### Gauges

Gauges reflect current state and are not converted into deltas:

- active subscriptions
- joined subscriptions

These should be read directly from the latest snapshot.

## Rate Helpers

Delta types expose helpers such as:

- `packets_per_sec()`
- `bytes_per_sec()`
- `would_block_per_sec()`
- `receive_errors_per_sec()`

These compute average rates over the sampled interval.

## CLI Integration

`mcrx_recv` can periodically emit:

- terminal summaries
- JSONL file output

Configured via:

- `MCRX_METRICS_SUMMARY_SECS`
- `MCRX_METRICS_SUMMARY_FILE`
