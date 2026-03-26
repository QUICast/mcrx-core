use std::net::SocketAddr;
use std::time::SystemTime;

/// Computes an average per-second rate for a monotonically increasing counter.
fn rate_per_sec(count: u64, interval_secs: f64) -> f64 {
    if interval_secs > 0.0 {
        count as f64 / interval_secs
    } else {
        0.0
    }
}

/// A point-in-time snapshot of cumulative subscription metrics.
///
/// Counter fields in this snapshot are cumulative from the lifetime of the
/// subscription and can be compared against an earlier snapshot to compute
/// deltas and rates.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone)]
pub struct SubscriptionMetricsSnapshot {
    pub packets_received: u64,
    pub bytes_received: u64,
    pub would_block_count: u64,
    pub receive_errors: u64,
    pub join_count: u64,
    pub leave_count: u64,
    pub last_payload_len: Option<usize>,
    pub last_source: Option<SocketAddr>,
    pub last_receive_at: Option<SystemTime>,
    pub captured_at: SystemTime,
}

/// The difference between two cumulative subscription metrics snapshots.
///
/// This contains only counter-based deltas over the sampled interval.
/// Last-seen values such as source address or payload length are intentionally
/// not included here; callers should inspect those directly from the latest
/// snapshot instead.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone)]
pub struct SubscriptionMetricsDelta {
    pub interval_secs: f64,
    pub packets_received: u64,
    pub bytes_received: u64,
    pub would_block_count: u64,
    pub receive_errors: u64,
    pub join_count: u64,
    pub leave_count: u64,
}

#[cfg(feature = "metrics")]
impl SubscriptionMetricsSnapshot {
    /// Computes the counter deltas between this snapshot and an earlier one.
    ///
    /// Returns `None` if:
    /// - `earlier` was captured after `self`
    /// - any cumulative counter appears to have moved backwards
    pub fn delta_since(&self, earlier: &Self) -> Option<SubscriptionMetricsDelta> {
        let duration = self.captured_at.duration_since(earlier.captured_at).ok()?;
        let interval_secs = duration.as_secs_f64();

        Some(SubscriptionMetricsDelta {
            interval_secs,
            packets_received: self
                .packets_received
                .checked_sub(earlier.packets_received)?,
            bytes_received: self.bytes_received.checked_sub(earlier.bytes_received)?,
            would_block_count: self
                .would_block_count
                .checked_sub(earlier.would_block_count)?,
            receive_errors: self.receive_errors.checked_sub(earlier.receive_errors)?,
            join_count: self.join_count.checked_sub(earlier.join_count)?,
            leave_count: self.leave_count.checked_sub(earlier.leave_count)?,
        })
    }
}

#[cfg(feature = "metrics")]
impl SubscriptionMetricsDelta {
    /// Returns the average packets received per second over the sampled interval.
    pub fn packets_per_sec(&self) -> f64 {
        rate_per_sec(self.packets_received, self.interval_secs)
    }

    /// Returns the average bytes received per second over the sampled interval.
    pub fn bytes_per_sec(&self) -> f64 {
        rate_per_sec(self.bytes_received, self.interval_secs)
    }

    /// Returns the average would-block count per second over the sampled interval.
    pub fn would_block_per_sec(&self) -> f64 {
        rate_per_sec(self.would_block_count, self.interval_secs)
    }

    /// Returns the average receive error count per second over the sampled interval.
    pub fn receive_errors_per_sec(&self) -> f64 {
        rate_per_sec(self.receive_errors, self.interval_secs)
    }
}

/// Tracks successive subscription metrics snapshots and computes deltas between them.
///
/// The first call to `sample()` stores the provided snapshot and returns `None`,
/// because there is no earlier sample to compare against yet.
///
/// Each subsequent call compares the new snapshot against the previous one,
/// updates the stored baseline, and returns the computed delta.
#[cfg(feature = "metrics")]
#[derive(Debug, Default, Clone)]
pub struct SubscriptionMetricsSampler {
    previous: Option<SubscriptionMetricsSnapshot>,
}

#[cfg(feature = "metrics")]
impl SubscriptionMetricsSampler {
    /// Creates a new sampler with no previous snapshot.
    pub fn new() -> Self {
        Self { previous: None }
    }

    /// Samples a new cumulative subscription metrics snapshot and returns the
    /// delta since the previous sample, if any.
    ///
    /// On the first call, this stores `current` and returns `None`.
    ///
    /// On later calls, this returns the delta between `current` and the previous
    /// snapshot, then stores `current` as the new baseline.
    pub fn sample(
        &mut self,
        current: SubscriptionMetricsSnapshot,
    ) -> Option<SubscriptionMetricsDelta> {
        let delta = match &self.previous {
            Some(previous) => current.delta_since(previous),
            None => None,
        };

        self.previous = Some(current);
        delta
    }

    /// Clears the stored baseline snapshot.
    ///
    /// After calling this, the next call to `sample()` will return `None` again.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    /// Returns the currently stored baseline snapshot, if any.
    pub fn previous(&self) -> Option<&SubscriptionMetricsSnapshot> {
        self.previous.as_ref()
    }
}

/// A point-in-time snapshot of cumulative context metrics.
///
/// Counter fields in this snapshot are cumulative from the lifetime of the
/// context and can be compared against an earlier snapshot to compute deltas
/// and rates.
///
/// Gauge-like fields such as `active_subscriptions` and `joined_subscriptions`
/// represent the current state at the moment the snapshot was taken and should
/// not be interpreted as cumulative counters.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone)]
pub struct ContextMetricsSnapshot {
    pub subscriptions_added: u64,
    pub subscriptions_removed: u64,
    pub active_subscriptions: usize,
    pub joined_subscriptions: usize,
    pub total_packets_received: u64,
    pub total_bytes_received: u64,
    pub total_would_block_count: u64,
    pub total_receive_errors: u64,
    pub total_join_count: u64,
    pub total_leave_count: u64,
    pub batch_calls: u64,
    pub batch_packets_received: u64,
    pub captured_at: SystemTime,
}

/// The difference between two cumulative context metrics snapshots.
///
/// This contains only counter-based deltas over the sampled interval.
/// Gauge-like values such as active subscription counts are intentionally not
/// included here; callers should inspect those directly from the latest
/// snapshot instead.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone)]
pub struct ContextMetricsDelta {
    pub interval_secs: f64,
    pub packets_received: u64,
    pub bytes_received: u64,
    pub would_block_count: u64,
    pub receive_errors: u64,
    pub join_count: u64,
    pub leave_count: u64,
    pub batch_calls: u64,
    pub batch_packets_received: u64,
}

#[cfg(feature = "metrics")]
impl ContextMetricsSnapshot {
    /// Computes the counter deltas between this snapshot and an earlier one.
    ///
    /// Returns `None` if:
    /// - `earlier` was captured after `self`
    /// - any cumulative counter appears to have moved backwards
    ///
    /// The resulting delta contains only counter-based values and the elapsed
    /// interval in seconds. Gauge-like values such as active subscription counts
    /// should be read directly from the latest snapshot instead.
    pub fn delta_since(&self, earlier: &Self) -> Option<ContextMetricsDelta> {
        let duration = self.captured_at.duration_since(earlier.captured_at).ok()?;
        let interval_secs = duration.as_secs_f64();

        Some(ContextMetricsDelta {
            interval_secs,
            packets_received: self
                .total_packets_received
                .checked_sub(earlier.total_packets_received)?,
            bytes_received: self
                .total_bytes_received
                .checked_sub(earlier.total_bytes_received)?,
            would_block_count: self
                .total_would_block_count
                .checked_sub(earlier.total_would_block_count)?,
            receive_errors: self
                .total_receive_errors
                .checked_sub(earlier.total_receive_errors)?,
            join_count: self
                .total_join_count
                .checked_sub(earlier.total_join_count)?,
            leave_count: self
                .total_leave_count
                .checked_sub(earlier.total_leave_count)?,
            batch_calls: self.batch_calls.checked_sub(earlier.batch_calls)?,
            batch_packets_received: self
                .batch_packets_received
                .checked_sub(earlier.batch_packets_received)?,
        })
    }
}

#[cfg(feature = "metrics")]
impl ContextMetricsDelta {
    /// Returns the average packets received per second over the sampled interval.
    pub fn packets_per_sec(&self) -> f64 {
        rate_per_sec(self.packets_received, self.interval_secs)
    }

    /// Returns the average bytes received per second over the sampled interval.
    pub fn bytes_per_sec(&self) -> f64 {
        rate_per_sec(self.bytes_received, self.interval_secs)
    }

    /// Returns the average would-block count per second over the sampled interval.
    pub fn would_block_per_sec(&self) -> f64 {
        rate_per_sec(self.would_block_count, self.interval_secs)
    }

    /// Returns the average receive error count per second over the sampled interval.
    pub fn receive_errors_per_sec(&self) -> f64 {
        rate_per_sec(self.receive_errors, self.interval_secs)
    }
}

/// Tracks successive context metrics snapshots and computes deltas between them.
///
/// The first call to `sample()` stores the provided snapshot and returns `None`,
/// because there is no earlier sample to compare against yet.
///
/// Each subsequent call compares the new snapshot against the previous one,
/// updates the stored baseline, and returns the computed delta.
#[cfg(feature = "metrics")]
#[derive(Debug, Default, Clone)]
pub struct ContextMetricsSampler {
    previous: Option<ContextMetricsSnapshot>,
}

#[cfg(feature = "metrics")]
impl ContextMetricsSampler {
    /// Creates a new sampler with no previous snapshot.
    pub fn new() -> Self {
        Self { previous: None }
    }

    /// Samples a new cumulative context metrics snapshot and returns the delta
    /// since the previous sample, if any.
    ///
    /// On the first call, this stores `current` and returns `None`.
    ///
    /// On later calls, this returns the delta between `current` and the previous
    /// snapshot, then stores `current` as the new baseline.
    pub fn sample(&mut self, current: ContextMetricsSnapshot) -> Option<ContextMetricsDelta> {
        let delta = match &self.previous {
            Some(previous) => current.delta_since(previous),
            None => None,
        };

        self.previous = Some(current);
        delta
    }

    /// Clears the stored baseline snapshot.
    ///
    /// After calling this, the next call to `sample()` will return `None` again.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    /// Returns the currently stored baseline snapshot, if any.
    pub fn previous(&self) -> Option<&ContextMetricsSnapshot> {
        self.previous.as_ref()
    }
}

#[cfg(all(test, feature = "metrics"))]
mod tests {
    use super::*;
    use crate::Context;
    use crate::test_support::{make_multicast_sender, recv_next_packet, sample_config};

    use std::net::SocketAddrV4;
    use std::thread;
    use std::time::{Duration, Instant};

    // Test helper: uses fixed non-zero values for unrelated fields.
    fn make_context_snapshot(
        total_packets_received: u64,
        total_bytes_received: u64,
        total_would_block_count: u64,
        total_receive_errors: u64,
        batch_calls: u64,
        batch_packets_received: u64,
    ) -> ContextMetricsSnapshot {
        ContextMetricsSnapshot {
            subscriptions_added: 1,
            subscriptions_removed: 0,
            active_subscriptions: 1,
            joined_subscriptions: 1,
            total_packets_received,
            total_bytes_received,
            total_would_block_count,
            total_receive_errors,
            total_join_count: 1,
            total_leave_count: 0,
            batch_calls,
            batch_packets_received,
            captured_at: SystemTime::now(),
        }
    }

    // Test helper: uses fixed non-zero values for unrelated fields.
    fn make_subscription_snapshot(
        packets_received: u64,
        bytes_received: u64,
        would_block_count: u64,
        receive_errors: u64,
        last_payload_len: Option<usize>,
    ) -> SubscriptionMetricsSnapshot {
        SubscriptionMetricsSnapshot {
            packets_received,
            bytes_received,
            would_block_count,
            receive_errors,
            join_count: 1,
            leave_count: 0,
            last_payload_len,
            last_source: None,
            last_receive_at: None,
            captured_at: SystemTime::now(),
        }
    }

    #[test]
    fn context_metrics_sampler_returns_none_on_first_sample() {
        let snapshot = make_context_snapshot(10, 1000, 2, 0, 3, 10);

        let mut sampler = ContextMetricsSampler::new();
        let delta = sampler.sample(snapshot);

        assert!(delta.is_none());
    }

    #[test]
    fn context_metrics_sampler_returns_delta_on_second_sample() {
        let earlier = make_context_snapshot(10, 1000, 2, 0, 3, 10);

        thread::sleep(Duration::from_millis(10));

        let later = make_context_snapshot(15, 1600, 3, 1, 5, 15);

        let mut sampler = ContextMetricsSampler::new();
        assert!(sampler.sample(earlier).is_none());

        let delta = sampler.sample(later).unwrap();

        assert_eq!(delta.packets_received, 5);
        assert_eq!(delta.bytes_received, 600);
        assert_eq!(delta.would_block_count, 1);
        assert_eq!(delta.receive_errors, 1);
        assert_eq!(delta.join_count, 0);
        assert_eq!(delta.leave_count, 0);
        assert_eq!(delta.batch_calls, 2);
        assert_eq!(delta.batch_packets_received, 5);
        assert!(delta.interval_secs > 0.0);
    }

    #[test]
    fn subscription_metrics_sampler_returns_none_on_first_sample() {
        let snapshot = make_subscription_snapshot(10, 1000, 2, 0, Some(100));

        let mut sampler = SubscriptionMetricsSampler::new();
        let delta = sampler.sample(snapshot);

        assert!(delta.is_none());
    }

    #[test]
    fn subscription_metrics_sampler_returns_delta_on_second_sample() {
        let earlier = make_subscription_snapshot(10, 1000, 2, 0, Some(100));

        thread::sleep(Duration::from_millis(10));

        let later = make_subscription_snapshot(12, 1300, 3, 1, Some(150));

        let mut sampler = SubscriptionMetricsSampler::new();
        assert!(sampler.sample(earlier).is_none());

        let delta = sampler.sample(later).unwrap();

        assert_eq!(delta.packets_received, 2);
        assert_eq!(delta.bytes_received, 300);
        assert_eq!(delta.would_block_count, 1);
        assert_eq!(delta.receive_errors, 1);
        assert_eq!(delta.join_count, 0);
        assert_eq!(delta.leave_count, 0);
        assert!(delta.interval_secs > 0.0);
    }

    #[test]
    fn metrics_snapshot_tracks_join_and_leave_counts() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9016)).unwrap();

        context.join_subscription(id).unwrap();
        context.leave_subscription(id).unwrap();

        let metrics = context.metrics_snapshot();
        let subscription_metrics = context.get_subscription(id).unwrap().metrics_snapshot();

        assert_eq!(metrics.subscriptions_added, 1);
        assert_eq!(metrics.subscriptions_removed, 0);
        assert_eq!(metrics.active_subscriptions, 1);
        assert_eq!(metrics.joined_subscriptions, 0);
        assert_eq!(metrics.total_join_count, 1);
        assert_eq!(metrics.total_leave_count, 1);

        assert_eq!(subscription_metrics.join_count, 1);
        assert_eq!(subscription_metrics.leave_count, 1);
    }

    #[test]
    fn metrics_snapshot_tracks_received_packets_and_bytes() {
        let mut context = Context::new();
        let config = sample_config(9017);
        let id = context.add_subscription(config.clone()).unwrap();
        context.join_subscription(id).unwrap();

        let sender = make_multicast_sender();

        let payload = b"metrics-packet";
        sender
            .send_to(payload, SocketAddrV4::new(config.group, config.dst_port))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        let packet = recv_next_packet(&mut context, deadline);
        assert_eq!(&packet.payload[..], payload);

        let metrics = context.metrics_snapshot();
        let subscription_metrics = context.get_subscription(id).unwrap().metrics_snapshot();

        assert_eq!(metrics.total_packets_received, 1);
        assert_eq!(metrics.total_bytes_received, payload.len() as u64);
        assert_eq!(subscription_metrics.packets_received, 1);
        assert_eq!(subscription_metrics.bytes_received, payload.len() as u64);
        assert_eq!(subscription_metrics.last_payload_len, Some(payload.len()));
        assert!(subscription_metrics.last_source.is_some());
        assert!(subscription_metrics.last_receive_at.is_some());
    }

    #[test]
    fn metrics_snapshot_delta_tracks_counter_changes() {
        let mut context = Context::new();
        let config = sample_config(9018);
        let id = context.add_subscription(config.clone()).unwrap();
        context.join_subscription(id).unwrap();

        let earlier = context.metrics_snapshot();

        let sender = make_multicast_sender();

        let payload = b"delta-metrics";
        sender
            .send_to(payload, SocketAddrV4::new(config.group, config.dst_port))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        let packet = recv_next_packet(&mut context, deadline);
        assert_eq!(&packet.payload[..], payload);

        let later = context.metrics_snapshot();
        let delta = later.delta_since(&earlier).unwrap();

        assert_eq!(delta.packets_received, 1);
        assert_eq!(delta.bytes_received, payload.len() as u64);
        assert_eq!(delta.join_count, 0);
        assert_eq!(delta.leave_count, 0);
    }
}
