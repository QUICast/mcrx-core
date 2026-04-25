pub mod config;
pub mod context;
pub mod error;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod packet;
mod platform;
pub mod subscription;
#[cfg(test)]
mod test_support;
#[cfg(feature = "tokio")]
pub mod tokio_adapter;

pub use config::{SourceFilter, SubscriptionConfig};
pub use context::Context;
pub use error::McrxError;
pub use packet::{Packet, PacketWithMetadata, ReceiveMetadata};
pub use subscription::{Subscription, SubscriptionId, SubscriptionParts, SubscriptionState};
#[cfg(feature = "tokio")]
pub use tokio_adapter::{TokioReceiveError, TokioSubscription};

#[cfg(feature = "metrics")]
pub use metrics::{
    ContextMetricsDelta, ContextMetricsSampler, ContextMetricsSnapshot, HardwareMetricsDelta,
    HardwareMetricsSampler, HardwareMetricsSnapshot, SubscriptionMetricsDelta,
    SubscriptionMetricsSampler, SubscriptionMetricsSnapshot,
};
