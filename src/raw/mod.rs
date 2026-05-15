//! Optional raw multicast receive support.
//!
//! Enable this module with the `raw-packets` Cargo feature when you need
//! complete multicast IP datagrams instead of UDP payloads.
//!
//! The first implementation targets Linux. Other platforms currently return a
//! clear unsupported error rather than silently degrading to UDP behavior.

mod config;
mod context;
mod packet;
mod subscription;

pub use config::RawSubscriptionConfig;
pub use context::RawContext;
pub use packet::RawPacket;
pub use subscription::RawSubscription;
