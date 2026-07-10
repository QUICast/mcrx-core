# Raw Packet Receive

`mcrx-core` can optionally receive complete multicast IP datagrams instead of
just UDP payloads.

This support is gated behind the `raw-packets` Cargo feature so normal UDP
users do not pay for extra API surface, platform checks, or privileged socket
requirements.

For Linux applications with many raw memberships, the additive
`raw-shared-capture` feature provides a separate shared-socket API. It implies
`raw-packets`; neither feature changes the default UDP API.

## Status

Current first-pass support:

- Linux: implemented
- macOS: implemented through BPF, with explicit interface selection required
- Windows: implemented for IPv4 raw receive, with explicit IPv4 interface
  selection required
- Shared raw capture: implemented on Linux only; macOS and Windows return a
  typed unsupported error rather than falling back to per-subscription capture

The raw API is intentionally separate from the UDP API. Enabling
`raw-packets` does not change the behavior of `Context`, `Subscription`,
`SubscriptionConfig`, or `Packet`.

## Why It Exists

The default receive path is UDP-centric:

- `SubscriptionConfig` includes `dst_port`
- `Packet` exposes source socket address, destination group, destination port,
  and UDP payload bytes
- the platform layer receives on UDP sockets

That is the right shape for ordinary multicast applications. It is not enough
for applications such as AMT relay forwarding that need the original multicast
IP datagram so it can be wrapped or forwarded intact.

## Feature Enablement

```bash
cargo add mcrx-core --features raw-packets
cargo add mcrx-core --features raw-shared-capture
```

## Raw API Types

The raw feature adds a parallel API:

- `RawSubscriptionConfig`
- `RawContext`
- `RawSubscription`
- `RawPacket`

`RawSubscriptionConfig` keeps the same multicast membership model as the UDP
path:

- ASM with `source = Any`
- SSM with `source = Source(ip)`
- optional interface address
- optional IPv6 interface index

Unlike `SubscriptionConfig`, it does not include a UDP port.

## Example

```rust
use mcrx_core::{RawContext, RawSubscriptionConfig};
use std::net::Ipv4Addr;

let mut ctx = RawContext::new();
let id = ctx.add_subscription(RawSubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3)))?;
ctx.join_subscription(id)?;

if let Some(packet) = ctx.try_recv_any()? {
    println!("received {} raw bytes", packet.datagram_len());
    println!("source ip: {:?}", packet.source_ip);
    println!("group ip: {:?}", packet.group);
    println!("ip protocol: {:?}", packet.ip_protocol);
}
```

The feature also includes a small receiver binary for platform testing:

```bash
cargo run --features raw-packets --bin mcrx_raw_recv -- 239.1.2.3 --interface 192.168.1.20
cargo run --features raw-packets --bin mcrx_raw_recv -- ff1e::8000:1234 --interface 7
```

## Shared Raw Capture (Linux)

`RawContext` deliberately keeps its original one-capture-socket-per-logical-
subscription behavior. Applications that need to join hundreds or thousands of
raw memberships can instead opt into `raw-shared-capture` and use
`SharedRawContext`.

The shared backend opens approximately one Linux packet socket per resolved IP
family/interface tuple, installs ASM or SSM memberships on a normal membership
socket for that tuple, and indexes packets by multicast group then source
address. A complete kernel IP datagram is read once. Its `SharedRawPacket`
identifies every matching logical subscription, so duplicate logical
memberships do not cause duplicate capture reads.

```rust
use mcrx_core::{RawSubscriptionConfig, SharedRawContext};
use std::net::Ipv4Addr;

let mut ctx = SharedRawContext::new();
let mut config = RawSubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3));
config.interface = Some(Ipv4Addr::new(192, 168, 1, 20).into());

let id = ctx.add_subscription(config)?;
ctx.join_subscription(id)?;

if let Some(packet) = ctx.try_recv_any()? {
    assert_eq!(packet.packet.subscription_id, id);
    assert_eq!(packet.matching_subscription_ids(), &[id]);
    println!("received {} raw bytes", packet.packet.datagram_len());
}

assert_eq!(ctx.capture_socket_count(), 1);
```

The repository also includes a Linux-oriented runnable example:

```bash
cargo run --example shared_raw_capture --features raw-shared-capture -- \
  239.1.2.3 192.168.1.20
```

`SharedRawContext` provides independent `add_subscription`, `join_subscription`,
`leave_subscription`, and `remove_subscription` operations. Unlike `RawContext`,
it intentionally permits duplicate configs: duplicate handles share a
reference-counted kernel membership, and leaving one handle does not affect the
others. Leaving the final logical membership for a family/interface tuple closes
that capture socket. Its default bounds are 4,096 logical subscriptions and
1,024 queued demultiplexed packets. Use
`SharedRawContext::with_limits(SharedRawContextLimits { .. })` to set explicit
bounds.

With both `raw-shared-capture` and `metrics` enabled,
`SharedRawContext::metrics_snapshot()` reports the current capture-socket and
active-membership counts plus process-lifetime received, unmatched, and
demultiplex-match totals. It also reports the current bounded pending-packet
count.

Receive work is proportional to the number of capture sockets polled plus the
matched subscription IDs, never a scan of every logical subscription. Shared
capture remains nonblocking and does not expose a per-logical-subscription
socket because that socket is intentionally shared.

Resolved interface keys are cached per context and reference-counted by stored
subscriptions, so adding many groups on one explicit interface performs the
platform interface lookup once. Removing the final subscription for a selector
also removes the cache entry.

The common case of one matching logical membership is stored inline without a
separate match-list allocation. Batch receive drains multiple datagrams from a
ready capture before moving on, while preserving round-robin capture fairness
between calls.

### Shared Capture Limits and Tests

Shared capture needs Linux raw-socket permission (`CAP_NET_RAW` or root). It
also does not bypass Linux multicast membership limits: a test with 1,000
logical joins normally needs the namespace-local
`net.ipv4.igmp_max_memberships` sysctl raised first.

The privileged integration test does that in an isolated network namespace. It
also verifies duplicate-membership demultiplexing, leave isolation, batched
receive, and final capture-socket cleanup:

```bash
sudo unshare --user --map-root-user --net \
  cargo test -p mcrx-core --features raw-shared-capture \
  --test shared_raw_capture_linux -- --ignored
```

The test requires a host that permits unprivileged user namespaces and has the
`ip` and `sysctl` utilities. It is ignored by default and is not required for
ordinary CI.

## Raw Packet Shape

`RawPacket` contains:

- `subscription_id`
- full IP datagram bytes as `Bytes`
- parsed source IP when cheaply available
- parsed destination/group IP when cheaply available
- parsed IP protocol or IPv6 next-header when cheaply available
- `ReceiveMetadata`

Captured padding is removed according to the IPv4 total-length or IPv6
payload-length field, and truncated datagrams are discarded. IPv6 jumbograms
are not supported by the bounded raw receive API.

The raw path reuses the same `ReceiveMetadata` struct as the UDP metadata API.
The most useful fields are:

- `configured_interface`
- `configured_interface_index`
- `destination_local_ip`
- `ingress_interface_index`

`socket_local_addr` is typically not meaningful for packet-socket, BPF, or raw
capture receive and should be treated as optional.

## Platform Limitations

### Linux

The current implementation uses Linux packet sockets for receive and a normal
UDP socket for multicast membership management.

That gives a useful property for AMT-style use cases: the receive path returns
the complete IP datagram rather than just the UDP payload.

Practical notes:

- raw packet sockets usually require `CAP_NET_RAW` or root
- the packet socket is opened with `ETH_P_ALL`, matching tcpdump-style behavior
  for same-host outbound multicast paths where the packet may only be visible
  as `PACKET_OUTGOING`
- an attached classic BPF program filters the configured IP family, destination
  group, and optional source in the kernel; parsing verifies them again
- multicast join and leave still reuse the normal multicast membership logic
- interface selection matters, especially for IPv6 and link-local groups
- with no explicit interface, Linux binds the packet socket to all interfaces
  while the membership socket uses the kernel-selected default; use an explicit
  selector when capture must be restricted to one interface

Manual same-host check:

```bash
sudo ./target/debug/mcrx_raw_recv 239.1.2.3 --interface <receiver-ipv4>
tcpdump -Q out -ni <iface> 'dst host 239.1.2.3'
```

When a local sender emits to `239.1.2.3`, both tcpdump and `mcrx_raw_recv`
should see the packet even if `tcpdump -Q in` stays quiet.

For `raw-shared-capture`, the packet socket remains `ETH_P_ALL` and uses a
family-and-multicast-destination BPF filter. Exact group and source filtering
happen in the shared userspace index after the packet is parsed. This preserves
locally emitted multicast packets that Linux exposes only as `PACKET_OUTGOING`.

### macOS

The macOS implementation uses BPF devices for receive and a normal UDP socket
for multicast membership management.

Practical notes:

- opening `/dev/bpf*` usually requires elevated privileges or device
  permissions
- raw receive requires an explicit interface address or interface index
- common BPF datalink types are supported: raw IP, loopback/null, Ethernet,
  and VLAN-tagged Ethernet
- BPF capture blocks are retained across receive calls so every matching record
  in a kernel batch can be returned
- an attached BPF program filters raw/null and ordinary Ethernet traffic in the
  kernel; VLAN traffic is conservatively verified in userspace
- unsupported BPF datalink types return
  `McrxError::RawPacketReceiveUnsupported(...)`

### Windows

The Windows implementation currently supports IPv4 raw receive.

Practical notes:

- raw sockets usually require Administrator privileges
- IPv4 raw receive requires an explicit local IPv4 interface address
- the implementation first tries Windows multicast-only capture and falls back
  to ordinary raw capture if needed
- Windows also opens an IPv4/UDP raw receive socket as a required fallback for
  adapters that expose multicast control traffic through raw capture but do not
  surface UDP multicast data there
- setup and membership failures on either capture path are reported rather than
  silently degrading; UDP is emitted only through the fallback path to avoid
  duplicate delivery
- IPv6 raw receive currently returns
  `McrxError::RawPacketReceiveUnsupported(...)`

The crate does not silently fall back to UDP payload receive on any platform.
That would change the semantics of the API and make AMT-style forwarding easy
to misuse.

## Validation Rules

The raw config path reuses the same multicast validation model as the UDP API:

- group must be multicast
- source family must match group family
- interface family must match group family
- IPv6 interface indexes are supported, IPv4 interface indexes are rejected
- IPv4 SSM requires `232.0.0.0/8`
- IPv6 SSM requires `ff3x::/32`
- SSM-reserved ranges require a source; source-less ASM joins to those ranges
  are rejected

## IPv6 Notes

Raw IPv6 multicast follows the same group and interface guidance as the UDP
receiver. See [IPv6 Multicast](ipv6.md) for the full scoping rules.

- use `ff3x::/32` for IPv6 SSM
- use explicit interface selection when link-local scope is involved
- prefer explicit IPv6 interface indexes for deterministic joins when needed
