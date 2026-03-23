# mcrx-core

A portable multicast receiver library with support for ASM (*Any-Source Multicast*) and SSM (*Source-Specific
Multicast*).

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
- Cross-platform design (platform quirks handled internally)

---

## 📐 Architecture Overview

```
Context
 └── Subscriptions (Vec)
       └── Subscription
             └── Socket (OS / socket2)
                   └── Packet (output)
```

### Flow

1. A `Context` manages multiple multicast subscriptions
2. Each `Subscription` owns a socket joined to a multicast group
3. Incoming UDP packets are received via `try_recv()`
4. Packets are returned as `Packet` structs with metadata + payload

---

## 🚀 Quick Example

```rust
use mcrx_core::{Context, SubscriptionConfig, SourceFilter};
use std::net::Ipv4Addr;

let mut ctx = Context::new();

let config = SubscriptionConfig {
group: Ipv4Addr::new(239, 1, 2, 3),
source: SourceFilter::Any,
dst_port: 5000,
interface: None,
};

let _id = ctx.add_subscription(config) ?;

// Non-blocking receive
if let Some(packet) = ctx.try_recv_any() ? {
println ! ("Received {} bytes from {}", packet.payload.len(), packet.source);
}
```

---

---

## 🧪 Demo Binaries

### Receiver

```bash
cargo run --bin mcrx_recv -- 239.1.2.3 5000
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10
cargo run --bin mcrx_recv -- 232.1.2.3 5000 192.168.1.10 192.168.1.20
```

- omit `source` for ASM
- provide `source` for SSM
- `interface` is optional and selects the local join interface

---

### Sender

```bash
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello
cargo run --bin mcrx_send -- 239.1.2.3 5000 hello 1000
cargo run --bin mcrx_send -- 232.1.2.3 5000 hello 1000 192.168.1.20
```

- optional `interval_ms` enables periodic sending
- optional `interface` selects the outgoing interface

---

## 🔄 Receive Model

- `try_recv()` → per subscription
- `try_recv_any()` → across all subscriptions

Returns:

- `Ok(Some(packet))` → packet received
- `Ok(None)` → no packet available
- `Err(...)` → actual error

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
| macOS             | ✅     | ✅       | ⏳     | ⏳       | ⏳   |
| Windows           | ✅     | ✅       | ⏳     | ⏳       | ⏳   |
| Linux             | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| Android           | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |
| iOS               | ⏳     | ⏳       | ⏳     | ⏳       | ⏳   |

---

## ⚠️ Notes on macOS (SSM)

- macOS supports IGMPv3 but may temporarily emit IGMPv2 reports
- This can break SSM behavior on the network
- A system reboot may restore correct behavior

---

## 🧰 Planned Additions

- Linux validation
- Windows validation
- FFI bindings (C, Python)
- IPv6 support (MLDv2 / SSM)

---

## 🎯 Design Goals

- No runtime dependency
- Minimal unsafe usage (well-contained)
- Clean architecture for integration

---

## 📄 License

This project is licensed under the BSD 2-Clause License.
