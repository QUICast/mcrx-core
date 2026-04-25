# Demo Binaries

## Overview

The repository provides three small demo binaries:

- `mcrx_recv`
- `mcrx_recv_meta`
- `mcrx_send`

These are intended for real-network testing and API validation across devices.

## Receiver

```bash
cargo run --bin mcrx_recv -- <group> <dst_port> [source] [interface]
```

### Examples

ASM:

```bash
cargo run --bin mcrx_recv -- 239.1.2.3 5000
```

SSM:

```bash
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10
```

SSM with explicit interface:

```bash
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10 192.168.1.20
```

### Argument meaning

- `group` → multicast group address
- `dst_port` → destination UDP port
- `source` → optional SSM source address
- `interface` → optional local interface address

Rules:

- omit `source` for ASM
- provide `source` for SSM
- `interface` is optional and selects the local join interface

## Metadata-aware Receiver

```bash
cargo run --bin mcrx_recv_meta -- <group> <dst_port> [source] [interface]
```

This variant uses `try_recv_any_with_metadata()` and prints the richer receive
metadata alongside each packet, including:

- socket local bind address
- configured join interface
- pktinfo-style destination local IP when the platform reports it
- pktinfo-style ingress interface index when the platform reports it

Example:

```bash
cargo run --bin mcrx_recv_meta -- 239.1.2.3 5000
```

## Sender

```bash
cargo run --bin mcrx_send -- <group> <dst_port> <message> [interval_ms] [interface]
```

### Examples

Single send:

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello
```

Repeated send:

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello 1000
```

Repeated send with explicit interface:

```bash
cargo run --bin mcrx_send -- 232.1.2.3 5000 hello 1000 192.168.1.20
```

## Receiver Metrics

When built with `--features metrics`, `mcrx_recv` can emit periodic delta-based metrics summaries.

### Environment variables

#### `MCRX_METRICS_SUMMARY_SECS`

Emit a delta metrics summary every `n` seconds:

```bash
MCRX_METRICS_SUMMARY_SECS=2
```

#### `MCRX_METRICS_SUMMARY_FILE`

Append JSONL delta summaries to a file:

```bash
MCRX_METRICS_SUMMARY_FILE=metrics.jsonl
```

### Example usage

Print summaries to the terminal:

```bash
MCRX_METRICS_SUMMARY_SECS=2 cargo run --features metrics --bin mcrx_recv -- 239.1.2.3 5000
```

Write summaries to a file:

```bash
MCRX_METRICS_SUMMARY_SECS=2 MCRX_METRICS_SUMMARY_FILE=metrics.jsonl cargo run --features metrics --bin mcrx_recv -- 239.1.2.3 5000
```

## Example CLI Metrics Output

```text
[metrics]
  interval_secs:         2.000
  active_subscriptions:  1
  joined_subscriptions:  1
  packets_received:      120
  bytes_received:        144000
  would_block_count:     18
  receive_errors:        0
  join_count:            0
  leave_count:           0
  batch_calls:           0
  batch_packets:         0
  packets_per_sec:       60.000
  bytes_per_sec:         72000.000
  would_block_per_sec:   9.000
  recv_errors_per_sec:   0.000
```

## Example JSONL Output

```json
{"ts":1711387565.123,"interval_secs":2.0,"active_subscriptions":1,"joined_subscriptions":1,"packets_received":120,"bytes_received":144000,"would_block_count":18,"receive_errors":0,"join_count":0,"leave_count":0,"batch_calls":0,"batch_packets_received":0,"packets_per_sec":60.0,"bytes_per_sec":72000.0,"would_block_per_sec":9.0,"receive_errors_per_sec":0.0}
```

## Notes

- receiver uses a simple non-blocking polling loop
- `mcrx_recv_meta` is useful when validating interface-sensitive metadata wiring
- summaries are delta-based, not cumulative
- file output is append-only during a run
- the JSONL file is cleared once at program start before summaries are appended
