# mcrx-core

`mcrx-core` is a runtime-agnostic multicast receiver library for IPv4 and IPv6
ASM/SSM.

The default API receives UDP multicast payloads through explicit
`add`/`join`/`leave` subscription lifecycle calls. Optional features add Tokio
integration, metrics, raw IP datagram receive, and Python bindings without
changing the default UDP receive path. Sibling crates provide Python and C ABI
bindings for non-Rust consumers.

## Highlights

- IPv4 and IPv6 ASM/SSM receive support
- Non-blocking receive API
- Multiple subscriptions with fair receive across them
- Caller-provided socket support
- Socket borrowing and extraction for event-loop integration
- Pktinfo-verified destination filtering, with optional richer receive metadata
- Optional `tokio`, `metrics`, `raw-packets`, and Linux shared raw capture
- Optional Python bindings in the sibling `mcrx-core-py` crate
- Optional C ABI bindings in the sibling `mcrx-core-ffi` crate

## Install

`mcrx-core` requires Rust 1.88 or newer.

```bash
cargo add mcrx-core
```

Optional feature examples:

```bash
cargo add mcrx-core --features tokio
cargo add mcrx-core --features metrics
cargo add mcrx-core --features raw-packets
cargo add mcrx-core --features raw-shared-capture
```

## Quick Start

```rust
use mcrx_core::{Context, SubscriptionConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();
let id = ctx.add_subscription(SubscriptionConfig::asm(
    Ipv4Addr::new(239, 1, 2, 3),
    5000,
))?;
ctx.join_subscription(id)?;

if let Some(packet) = ctx.try_recv_any()? {
    println!("received {} bytes", packet.payload_len());
}
```

## Feature Map

- `tokio`: async receive wrapper for extracted subscriptions.
- `metrics`: snapshots, deltas, samplers, and JSONL helpers.
- `raw-packets`: complete multicast IP datagram receive for AMT-style use cases.
- `raw-shared-capture`: Linux-only shared raw capture for many logical raw
  memberships on one interface; implies `raw-packets`.
- `mcrx-core-py`: sibling workspace crate with Python and asyncio bindings.
- `mcrx-core-ffi`: sibling workspace crate with a small C ABI for Swift/C/C++.

## Documentation

- [Usage Guide](docs/usage.md): core Rust API flow.
- [IPv6 Guide](docs/ipv6.md): IPv6 scopes, SSM groups, and interface selection.
- [Raw Packet Receive](docs/raw-packets.md): `raw-packets` API and platform limits.
- [Demo Binaries](docs/demo.md): receiver CLI commands and metrics examples.
- [Metrics](docs/metrics.md): snapshot, delta, and JSONL semantics.
- [Python Bindings](docs/python.md): Python API and asyncio helper.
- [C FFI Bindings](mcrx-core-ffi/README.md): C ABI and iOS XCFramework sketch.
- [Architecture](docs/architecture.md): main types and module layout.
- [Design Decisions](docs/design-decisions.md): why the API is shaped this way.

## Platform Support

| OS      | ASM | SSM | Notes    |
|---------|-----|-----|----------|
| macOS   | Yes | Yes | Verified; see IPv4 SSM note |
| Linux   | Yes | Yes | Verified |
| Windows | Yes | Yes | Verified |

IPv4 and IPv6 ASM/SSM support and pktinfo-style receive metadata are wired into
the UDP receive path on the same platforms.

Raw multicast IP datagram receive is available behind the `raw-packets`
feature. Linux uses packet sockets, macOS uses BPF, and Windows currently
supports IPv4 raw receive. Unsupported raw modes return a clear error instead
of falling back to UDP payload receive.

For large raw membership sets on Linux, the opt-in `raw-shared-capture` feature
adds `SharedRawContext`. It shares one packet socket per resolved
family/interface and demultiplexes full IP datagrams in userspace. See
[Raw Packet Receive](docs/raw-packets.md#shared-raw-capture-linux) for its
different platform and kernel-membership limits.

## Compatibility

ASM cross-platform compatibility:

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | Yes   | Yes     | Yes   | TBD     | TBD |
| Windows           | Yes   | Yes     | Yes   | TBD     | TBD |
| Linux             | Yes   | Yes     | Yes   | TBD     | TBD |
| Android           | TBD   | TBD     | TBD   | TBD     | TBD |
| iOS               | TBD   | TBD     | TBD   | TBD     | TBD |

SSM cross-platform compatibility:

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | Yes   | Yes     | Yes   | TBD     | TBD |
| Windows           | Yes   | Yes     | Yes   | TBD     | TBD |
| Linux             | Yes   | Yes     | Yes   | TBD     | TBD |
| Android           | TBD   | TBD     | TBD   | TBD     | TBD |
| iOS               | TBD   | TBD     | TBD   | TBD     | TBD |

## Notes

- SSM subscriptions must use SSM group ranges: IPv4 `232.0.0.0/8` or IPv6
  `ff3x::/32`, and those ranges require a source filter. ASM-range groups such
  as `239.x.x.x` or `ff1x::/16` must be joined without a source.
- macOS IPv4 SSM is wired through the IPv4-specific source-membership API. If a
  valid `232/8` SSM join still emits IGMPv2 reports on the wire, macOS may have
  stale IGMP compatibility state; see the troubleshooting note in the
  [Usage Guide](docs/usage.md).

## License

BSD 2-Clause
