# Demo Binaries

## Overview

The repository provides four small demo binaries:

- `mcrx_recv`
- `mcrx_send`
- `mcrx_tokio_recv` (with `--features tokio`)
- `mcrx_recv_meta`

These are intended for real-network testing and API validation.

## Receiver

```bash
cargo run --bin mcrx_recv -- <group> <dst_port> [source] [interface]
```

Examples:

```bash
cargo run --bin mcrx_recv -- 239.1.2.3 5000
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10 192.168.1.20
cargo run --bin mcrx_recv -- ff01::1234 5000
cargo run --bin mcrx_recv -- ff01::1234 5000 --interface ::1
cargo run --bin mcrx_recv -- ff31::8000:1234 5000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10 --interface fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
```

Argument meaning:

- `group` → multicast group address
- `dst_port` → destination UDP port
- `source` → optional SSM source address
- `interface` → optional local interface address

## Sender

```bash
cargo run --bin mcrx_send -- <group> <dst_port> <message> [interval_ms] [interface]
```

Examples:

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello 1000
cargo run --bin mcrx_send -- 232.1.2.3 5000 hello 1000 192.168.1.20
cargo run --bin mcrx_send -- ff01::1234 5000 hello 1000 ::1
cargo run --bin mcrx_send -- ff01::1234 5000 hello 1000 1
cargo run --bin mcrx_send -- ff31::8000:1234 5000 hello 1000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
cargo run --bin mcrx_send -- ff3e::8000:1234 5000 hello 1000 fd06:ba51:f296:0:1caf:6b66:e6f7:4b10
```

For IPv6, the optional `interface` argument may be either a local IPv6 address
or a numeric interface index.

When the IPv6 `interface` argument is an IPv6 address, `mcrx_send` binds the
socket to that exact local address and also selects the corresponding outgoing
multicast interface. That is especially important for SSM, because the receive
side filters on the exact sender source IP.

## Tokio Receiver

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- <group> <dst_port> [source] [interface]
```

This variant extracts the joined subscription from the `Context`, wraps it in
`TokioSubscription`, and awaits packets asynchronously.

Example:

```bash
cargo run --features tokio --bin mcrx_tokio_recv -- 239.1.2.3 5000
```

## Metadata-aware Receiver

```bash
cargo run --bin mcrx_recv_meta -- <group> <dst_port> [source] [interface]
```

This variant uses `try_recv_any_with_metadata()` and prints the optional
receive metadata alongside each packet.

Example:

```bash
cargo run --bin mcrx_recv_meta -- 239.1.2.3 5000
cargo run --bin mcrx_recv_meta -- ff01::1234 5000 --interface ::1
cargo run --bin mcrx_recv_meta -- ff3e::8000:1234 5000 <sender-ipv6> --interface <receiver-ipv6>
```

## IPv6 SSM Tips

- Use `ff3x::/32` groups for IPv6 SSM. Good examples are `ff31::8000:1234`
  for same-host tests and `ff3e::8000:1234` for cross-machine testing.
- In receiver commands, `source` means the sender's IP address. `--interface`
  means the receiver's local join interface.
- The mixed form `mcrx_recv_meta <group> <port> <source> --interface <iface>`
  is supported and is often the clearest way to spell IPv6 SSM.
- `ff32::/16` is link-local scope. If you use it, send from a link-local
  `fe80::...` address rather than a ULA or global source.

Cross-machine IPv6 SSM example:

```bash
# sender
cargo run --bin mcrx_send -- ff3e::8000:1234 5000 hello-v6 1000 <sender-ipv6>

# receiver
cargo run --bin mcrx_recv_meta -- ff3e::8000:1234 5000 <sender-ipv6> --interface <receiver-ipv6>
```

## Receiver Metrics

When built with `--features metrics`, `mcrx_recv` can emit periodic delta-based
metrics summaries.

### Environment variables

`MCRX_METRICS_SUMMARY_SECS`

```bash
MCRX_METRICS_SUMMARY_SECS=2
```

`MCRX_METRICS_SUMMARY_FILE`

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

## Notes

- `mcrx_recv` uses a simple non-blocking polling loop
- `mcrx_tokio_recv` demonstrates the optional Tokio adapter and subscription handoff path
- `mcrx_recv_meta` is useful when validating interface-sensitive metadata wiring
