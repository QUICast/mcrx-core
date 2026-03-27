# mcrx-core

A portable multicast receiver library with support for ASM (*Any-Source Multicast*) and SSM (*Source-Specific Multicast*).

Designed to be:

- lightweight
- runtime-agnostic
- embeddable into larger systems (e.g. QUIC / quiche integrations)
- extensible via FFI (C, Rust, Python bindings planned)

---

## ✨ Features

- IPv4 ASM (`(*, G)`) support
- IPv4 SSM (`(S, G)`) support
- Non-blocking receive API
- Multiple concurrent subscriptions
- Zero-copy-friendly payload handling via `bytes::Bytes`
- Cross-platform design
- Explicit subscription lifecycle (`add`, `join`, `leave`, `remove`)
- Optional metrics with snapshots, deltas, and rate helpers

---

## 🚀 Quick Example

```rust
use mcrx_core::{Context, SubscriptionConfig};
use std::net::Ipv4Addr;

let mut ctx = Context::new();

let config = SubscriptionConfig::asm(
    Ipv4Addr::new(239, 1, 2, 3),
    5000,
);

let id = ctx.add_subscription(config)?;
ctx.join_subscription(id)?;

if let Some(packet) = ctx.try_recv_any()? {
    println!("Received {} bytes", packet.payload.len());
}
```

---

## 📚 Documentation

- [Architecture](docs/architecture.md)
- [Usage Guide](docs/usage.md)
- [Demo Binaries](docs/demo.md)
- [Metrics](docs/metrics.md)
- [Design Decisions](docs/design-decisions.md)

---

## 🧪 Demo Binaries

Receiver:

```bash
cargo run --bin mcrx_recv -- 239.1.2.3 5000
```

Sender:

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello
```

See [docs/demo.md](docs/demo.md) for full CLI and metrics documentation.

---

## 🧪 Platform Support

| OS      | ASM | SSM | Notes    |
|---------|-----|-----|----------|
| macOS   | ✅   | ✅   | Verified |
| Linux   | ✅   | ✅   | Verified |
| Windows | ✅   | ✅   | Verified |

---

## 🔁 ASM Cross-Platform Compatibility

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Windows           | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Linux             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Android           | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| iOS               | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |

---

## 🔁 SSM Cross-Platform Compatibility

| Sender / Receiver | macOS | Windows | Linux | Android | iOS |
|-------------------|-------|---------|-------|---------|-----|
| macOS             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Windows           | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Linux             | ✅     | ✅       | ✅     | ⏳       | ⏳   |
| Android           | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| iOS               | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |

---

## ⚠️ Notes on macOS (SSM)

- macOS supports IGMPv3 but may temporarily emit IGMPv2 reports
- This can break SSM behavior on the network
- A system reboot may restore correct behavior

---

## 📄 License

BSD 2-Clause
