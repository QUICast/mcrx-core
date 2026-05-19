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
- Optional pktinfo-style receive metadata
- Optional `tokio`, `metrics`, and `raw-packets` features
- Optional Python bindings in the sibling `mcrx-core-py` crate
- Optional C ABI bindings in the sibling `mcrx-core-ffi` crate

## Install

```bash
cargo add mcrx-core
```

Optional feature examples:

```bash
cargo add mcrx-core --features tokio
cargo add mcrx-core --features metrics
cargo add mcrx-core --features raw-packets
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
| macOS   | Yes | Yes | Verified |
| Linux   | Yes | Yes | Verified |
| Windows | Yes | Yes | Verified |

IPv4 and IPv6 ASM/SSM support and pktinfo-style receive metadata are wired into
the UDP receive path on the same platforms.

Raw multicast IP datagram receive is available behind the `raw-packets`
feature. Linux uses packet sockets, macOS uses BPF, and Windows currently
supports IPv4 raw receive. Unsupported raw modes return a clear error instead
of falling back to UDP payload receive.

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

- macOS may temporarily emit IGMPv2 reports in some SSM setups.
- That can break SSM behavior on the network until the host state recovers.

## License

BSD 2-Clause
