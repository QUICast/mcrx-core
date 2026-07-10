//! Optional raw multicast receive support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need
//! complete multicast IP datagrams instead of UDP payloads.
//!
//! Linux and macOS can receive IPv4 and IPv6 raw multicast datagrams. Windows
//! currently supports IPv4 raw receive only. Unsupported modes return a clear
//! error rather than silently degrading to UDP payload receive.

mod config;
mod context;
mod packet;
#[cfg(feature = "raw-shared-capture")]
mod shared;
mod subscription;

pub use config::RawSubscriptionConfig;
pub use context::RawContext;
pub use packet::RawPacket;
#[cfg(all(feature = "raw-shared-capture", feature = "metrics"))]
pub use shared::SharedRawCaptureMetricsSnapshot;
#[cfg(feature = "raw-shared-capture")]
pub use shared::{
    SharedRawContext, SharedRawContextLimits, SharedRawPacket, SharedRawSubscription,
};
pub use subscription::RawSubscription;
