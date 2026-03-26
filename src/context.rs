use crate::config::SubscriptionConfig;
use crate::error::McrxError;
use crate::packet::Packet;
use crate::platform::{join_multicast_group, leave_multicast_group, open_bound_socket};
use crate::subscription::{Subscription, SubscriptionId};

#[cfg(feature = "metrics")]
use crate::metrics::{ContextMetricsDelta, ContextMetricsSnapshot};
#[cfg(feature = "metrics")]
use std::cell::Cell;
#[cfg(feature = "metrics")]
use std::time::SystemTime;

#[cfg(feature = "metrics")]
#[derive(Debug, Default)]
struct ContextMetricsInner {
    subscriptions_added: Cell<u64>,
    subscriptions_removed: Cell<u64>,
    total_join_count: Cell<u64>,
    total_leave_count: Cell<u64>,
    batch_calls: Cell<u64>,
    batch_packets_received: Cell<u64>,
}

/// Owns and manages the set of active subscriptions.
#[derive(Debug, Default)]
pub struct Context {
    subscriptions: Vec<Subscription>,
    next_subscription_id: u64,
    next_recv_index: usize,
    #[cfg(feature = "metrics")]
    metrics: ContextMetricsInner,
}

impl Context {
    /// Creates an empty context with no subscriptions.
    pub fn new() -> Self {
        Self {
            subscriptions: Vec::new(),
            next_subscription_id: 1,
            next_recv_index: 0,
            #[cfg(feature = "metrics")]
            metrics: ContextMetricsInner::default(),
        }
    }

    /// Returns the number of active subscriptions currently stored in the context.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    #[cfg(feature = "metrics")]
    /// Returns a snapshot of the context's current metrics.
    pub fn metrics_snapshot(&self) -> ContextMetricsSnapshot {
        let mut total_packets_received = 0u64;
        let mut total_bytes_received = 0u64;
        let mut total_would_block_count = 0u64;
        let mut total_receive_errors = 0u64;
        let mut joined_subscriptions = 0usize;

        for subscription in &self.subscriptions {
            let snapshot = subscription.metrics_snapshot();
            total_packets_received += snapshot.packets_received;
            total_bytes_received += snapshot.bytes_received;
            total_would_block_count += snapshot.would_block_count;
            total_receive_errors += snapshot.receive_errors;
            if subscription.is_joined() {
                joined_subscriptions += 1;
            }
        }

        ContextMetricsSnapshot {
            subscriptions_added: self.metrics.subscriptions_added.get(),
            subscriptions_removed: self.metrics.subscriptions_removed.get(),
            active_subscriptions: self.subscriptions.len(),
            joined_subscriptions,
            total_packets_received,
            total_bytes_received,
            total_would_block_count,
            total_receive_errors,
            total_join_count: self.metrics.total_join_count.get(),
            total_leave_count: self.metrics.total_leave_count.get(),
            batch_calls: self.metrics.batch_calls.get(),
            batch_packets_received: self.metrics.batch_packets_received.get(),
            captured_at: SystemTime::now(),
        }
    }

    /// Returns true if a subscription with the given ID exists.
    pub fn contains_subscription(&self, id: SubscriptionId) -> bool {
        self.subscriptions
            .iter()
            .any(|subscription| subscription.id() == id)
    }

    /// Returns a read-only reference to the subscription with the given ID, if it exists.
    pub fn get_subscription(&self, id: SubscriptionId) -> Option<&Subscription> {
        self.subscriptions
            .iter()
            .find(|subscription| subscription.id() == id)
    }

    /// Returns a mutable reference to the subscription with the given ID, if it exists.
    pub fn get_subscription_mut(&mut self, id: SubscriptionId) -> Option<&mut Subscription> {
        self.subscriptions
            .iter_mut()
            .find(|subscription| subscription.id() == id)
    }
    /// Adds a new subscription to the context.
    ///
    /// The configuration is validated before insertion. If an identical subscription
    /// already exists, an error is returned instead of creating a duplicate.
    ///
    /// This function creates and binds the underlying socket, but does not join the
    /// multicast group yet. Call `join_subscription()` to activate multicast reception.
    pub fn add_subscription(
        &mut self,
        config: SubscriptionConfig,
    ) -> Result<SubscriptionId, McrxError> {
        config.validate()?;

        if self
            .subscriptions
            .iter()
            .any(|subscription| subscription.config() == &config)
        {
            return Err(McrxError::DuplicateSubscription);
        }

        let socket = open_bound_socket(&config)?;

        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;

        let subscription = Subscription::new(id, config, socket);
        self.subscriptions.push(subscription);

        #[cfg(feature = "metrics")]
        self.metrics
            .subscriptions_added
            .set(self.metrics.subscriptions_added.get() + 1);

        Ok(id)
    }

    /// Removes the subscription with the given ID.
    ///
    /// Returns true if a subscription was removed and false if no matching
    /// subscription was found.
    ///
    /// This uses `swap_remove`, so subscription order is not preserved.
    pub fn remove_subscription(&mut self, id: SubscriptionId) -> bool {
        if let Some(index) = self
            .subscriptions
            .iter()
            .position(|subscription| subscription.id() == id)
        {
            self.subscriptions.swap_remove(index);

            if self.subscriptions.is_empty() {
                self.next_recv_index = 0;
            } else {
                self.next_recv_index %= self.subscriptions.len();
            }

            #[cfg(feature = "metrics")]
            self.metrics
                .subscriptions_removed
                .set(self.metrics.subscriptions_removed.get() + 1);

            true
        } else {
            false
        }
    }

    /// Joins the multicast group for the given subscription.
    pub fn join_subscription(&mut self, id: SubscriptionId) -> Result<(), McrxError> {
        let subscription = self
            .get_subscription_mut(id)
            .ok_or(McrxError::SubscriptionNotFound)?;

        if subscription.is_joined() {
            return Err(McrxError::SubscriptionAlreadyJoined);
        }

        join_multicast_group(subscription.socket(), subscription.config())?;
        subscription.mark_joined()?;

        #[cfg(feature = "metrics")]
        self.metrics
            .total_join_count
            .set(self.metrics.total_join_count.get() + 1);

        Ok(())
    }

    /// Leaves the multicast group for the given subscription while keeping the socket bound.
    pub fn leave_subscription(&mut self, id: SubscriptionId) -> Result<(), McrxError> {
        let subscription = self
            .get_subscription_mut(id)
            .ok_or(McrxError::SubscriptionNotFound)?;

        if !subscription.is_joined() {
            return Err(McrxError::SubscriptionNotJoined);
        }

        leave_multicast_group(subscription.socket(), subscription.config())?;
        subscription.mark_bound()?;

        #[cfg(feature = "metrics")]
        self.metrics
            .total_leave_count
            .set(self.metrics.total_leave_count.get() + 1);

        Ok(())
    }

    /// Returns a read-only slice of all subscriptions currently stored in the context.
    pub fn subscriptions(&self) -> &[Subscription] {
        &self.subscriptions
    }

    /// Attempts to receive a single packet from any subscription without blocking.
    /// Uses round-robin style fairness across the different subscriptions.
    ///
    /// Returns the first available packet, if any subscription currently has one
    /// ready to be read.
    pub fn try_recv_any(&mut self) -> Result<Option<Packet>, McrxError> {
        let subscription_count = self.subscriptions.len();

        if subscription_count == 0 {
            return Ok(None);
        }

        for offset in 0..subscription_count {
            let index = (self.next_recv_index + offset) % subscription_count;
            let subscription = &self.subscriptions[index];

            if !subscription.is_joined() {
                continue;
            }

            if let Some(packet) = subscription.try_recv()? {
                self.next_recv_index = (index + 1) % subscription_count;
                return Ok(Some(packet));
            }
        }

        Ok(None)
    }

    /// Attempts to receive up to `max_packets` packets from any subscriptions without blocking.
    ///
    /// This method repeatedly calls `try_recv_any()` using the same round-robin fairness logic
    /// and pushes received packets into the provided `out` vector.
    ///
    /// Returns the number of packets that were added to `out`.
    ///
    /// Behavior:
    /// - Stops early if no more packets are available
    /// - Does not block or wait for new packets
    /// - Preserves fairness across subscriptions
    pub fn try_recv_batch_into(
        &mut self,
        out: &mut Vec<Packet>,
        max_packets: usize,
    ) -> Result<usize, McrxError> {
        let mut received = 0;

        #[cfg(feature = "metrics")]
        self.metrics
            .batch_calls
            .set(self.metrics.batch_calls.get() + 1);

        for _ in 0..max_packets {
            match self.try_recv_any()? {
                Some(packet) => {
                    out.push(packet);
                    received += 1;
                }
                None => break,
            }
        }

        #[cfg(feature = "metrics")]
        self.metrics
            .batch_packets_received
            .set(self.metrics.batch_packets_received.get() + received as u64);

        Ok(received)
    }

    /// Attempts to receive all currently available packets without blocking.
    ///
    /// This is a convenience wrapper around `try_recv_batch_into` that continues
    /// draining until no more packets are available.
    ///
    /// Note: this may result in unbounded growth of `out` if a large number of
    /// packets are queued, so callers should ensure capacity if needed.
    pub fn try_recv_all_into(&mut self, out: &mut Vec<Packet>) -> Result<usize, McrxError> {
        let mut total_received = 0;

        loop {
            let received = self.try_recv_batch_into(out, usize::MAX)?;
            total_received += received;

            if received == 0 {
                break;
            }
        }

        Ok(total_received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourceFilter;
    use crate::subscription::SubscriptionState;
    use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
    use std::thread;
    use std::time::{Duration, Instant};

    fn sample_config(port: u16) -> SubscriptionConfig {
        SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3),
            source: SourceFilter::Any,
            dst_port: port,
            interface: None,
        }
    }

    fn recv_next_packet(context: &mut Context, deadline: Instant) -> Packet {
        loop {
            match context.try_recv_any().unwrap() {
                Some(packet) => return packet,
                None if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("timed out waiting for packet"),
            }
        }
    }

    fn send_round_robin_test_packets(
        first_config: &SubscriptionConfig,
        second_config: &SubscriptionConfig,
    ) {
        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        sender.set_multicast_loop_v4(true).unwrap();
        sender.set_multicast_ttl_v4(1).unwrap();

        sender
            .send_to(
                b"first-1",
                SocketAddrV4::new(first_config.group, first_config.dst_port),
            )
            .unwrap();
        sender
            .send_to(
                b"second-1",
                SocketAddrV4::new(second_config.group, second_config.dst_port),
            )
            .unwrap();
        sender
            .send_to(
                b"first-2",
                SocketAddrV4::new(first_config.group, first_config.dst_port),
            )
            .unwrap();
    }

    #[test]
    fn new_context_starts_empty() {
        let context = Context::new();

        assert_eq!(context.subscription_count(), 0);
        assert!(context.subscriptions().is_empty());
    }

    #[test]
    fn add_subscription_returns_id_and_increases_count() {
        let mut context = Context::new();

        let id = context.add_subscription(sample_config(10000)).unwrap();

        assert_eq!(id, SubscriptionId(1));
        assert_eq!(context.subscription_count(), 1);
        assert_eq!(context.subscriptions()[0].id(), id);
    }

    #[test]
    fn adding_two_subscriptions_generates_different_ids() {
        let mut context = Context::new();

        let first = context.add_subscription(sample_config(5000)).unwrap();
        let second = context.add_subscription(sample_config(5001)).unwrap();

        assert_ne!(first, second);
        assert_eq!(first, SubscriptionId(1));
        assert_eq!(second, SubscriptionId(2));
    }

    #[test]
    fn invalid_subscription_is_rejected() {
        let mut context = Context::new();

        let invalid_config = SubscriptionConfig {
            group: Ipv4Addr::new(192, 168, 1, 10),
            source: SourceFilter::Any,
            dst_port: 5000,
            interface: None,
        };

        let result = context.add_subscription(invalid_config);

        assert!(matches!(result, Err(McrxError::InvalidMulticastGroup)));
        assert_eq!(context.subscription_count(), 0);
    }

    #[test]
    fn remove_existing_subscription_returns_true() {
        let mut context = Context::new();

        let id = context.add_subscription(sample_config(5009)).unwrap();

        let removed = context.remove_subscription(id);

        assert!(removed);
        assert_eq!(context.subscription_count(), 0);
    }

    #[test]
    fn remove_missing_subscription_returns_false() {
        let mut context = Context::new();

        let removed = context.remove_subscription(SubscriptionId(999));

        assert!(!removed);
    }

    #[test]
    fn three_subscriotions_have_len_3() {
        let mut context = Context::new();

        context.add_subscription(sample_config(6000)).unwrap();
        context.add_subscription(sample_config(6001)).unwrap();
        context.add_subscription(sample_config(6002)).unwrap();

        assert_eq!(context.subscription_count(), 3);
    }

    #[test]
    fn duplicate_subscription_is_rejected() {
        let mut context = Context::new();
        let config = sample_config(7000);

        let first = context.add_subscription(config.clone());
        let second = context.add_subscription(config);

        assert!(first.is_ok());
        assert!(matches!(second, Err(McrxError::DuplicateSubscription)));
        assert_eq!(context.subscription_count(), 1);
    }

    #[test]
    fn contains_subscription_returns_true_for_existing_id() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(8000)).unwrap();

        assert!(context.contains_subscription(id));
    }

    #[test]
    fn contains_subscription_returns_false_for_missing_id() {
        let context = Context::new();

        assert!(!context.contains_subscription(SubscriptionId(999)));
    }

    #[test]
    fn get_subscription_returns_matching_subscription() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9000)).unwrap();

        let subscription = context.get_subscription(id);

        assert!(subscription.is_some());
        assert_eq!(subscription.unwrap().id(), id);
    }

    #[test]
    fn get_subscription_returns_none_for_missing_id() {
        let context = Context::new();

        let subscription = context.get_subscription(SubscriptionId(999));

        assert!(subscription.is_none());
    }

    #[test]
    fn get_subscription_mut_returns_matching_subscription() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9001)).unwrap();

        let subscription = context.get_subscription_mut(id);

        assert!(subscription.is_some());
        assert_eq!(subscription.unwrap().id(), id);
    }

    #[test]
    fn get_subscription_mut_returns_none_for_missing_id() {
        let mut context = Context::new();

        let subscription = context.get_subscription_mut(SubscriptionId(999));

        assert!(subscription.is_none());
    }

    #[test]
    fn try_recv_any_returns_none_when_no_packet_is_available() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9002)).unwrap();
        context.join_subscription(id).unwrap();

        let result = context.try_recv_any().unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn try_recv_any_returns_packet_from_ready_subscription() {
        let mut context = Context::new();
        let config = sample_config(9003);
        let id = context.add_subscription(config.clone()).unwrap();
        context.join_subscription(id).unwrap();

        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        sender.set_multicast_loop_v4(true).unwrap();
        sender.set_multicast_ttl_v4(1).unwrap();

        let payload = b"context try_recv_any";
        sender
            .send_to(payload, SocketAddrV4::new(config.group, config.dst_port))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match context.try_recv_any().unwrap() {
                Some(packet) => {
                    assert_eq!(packet.group, std::net::IpAddr::V4(config.group));
                    assert_eq!(packet.dst_port, config.dst_port);
                    assert_eq!(&packet.payload[..], payload);
                    break;
                }
                None if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("timed out waiting for packet from context"),
            }
        }
    }

    #[test]
    fn try_recv_any_round_robins_between_ready_subscriptions() {
        let mut context = Context::new();
        let first_config = sample_config(9004);
        let second_config = sample_config(9005);

        let first_id = context.add_subscription(first_config.clone()).unwrap();
        context.join_subscription(first_id).unwrap();
        let second_id = context.add_subscription(second_config.clone()).unwrap();
        context.join_subscription(second_id).unwrap();

        send_round_robin_test_packets(&first_config, &second_config);

        let deadline = Instant::now() + Duration::from_secs(1);

        let first_packet = recv_next_packet(&mut context, deadline);
        let second_packet = recv_next_packet(&mut context, deadline);
        let third_packet = recv_next_packet(&mut context, deadline);

        assert_eq!(
            (first_packet.subscription_id, &first_packet.payload[..]),
            (first_id, b"first-1".as_slice())
        );

        assert_eq!(
            (second_packet.subscription_id, &second_packet.payload[..]),
            (second_id, b"second-1".as_slice())
        );

        assert_eq!(
            (third_packet.subscription_id, &third_packet.payload[..]),
            (first_id, b"first-2".as_slice())
        );
    }

    #[test]
    fn try_recv_batch_into_returns_zero_when_no_packet_is_available() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9006)).unwrap();
        context.join_subscription(id).unwrap();

        let mut packets = Vec::new();
        let received = context.try_recv_batch_into(&mut packets, 8).unwrap();

        assert_eq!(received, 0);
        assert!(packets.is_empty());
    }

    #[test]
    fn try_recv_batch_into_receives_up_to_max_packets() {
        let mut context = Context::new();
        let first_config = sample_config(9007);
        let second_config = sample_config(9008);

        let first_id = context.add_subscription(first_config.clone()).unwrap();
        context.join_subscription(first_id).unwrap();
        let second_id = context.add_subscription(second_config.clone()).unwrap();
        context.join_subscription(second_id).unwrap();

        send_round_robin_test_packets(&first_config, &second_config);

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut packets = Vec::new();

        while packets.len() < 2 && Instant::now() < deadline {
            context.try_recv_batch_into(&mut packets, 2).unwrap();

            if packets.len() < 2 {
                thread::sleep(Duration::from_millis(10));
            }
        }

        assert_eq!(packets.len(), 2);

        assert_eq!(
            (packets[0].subscription_id, &packets[0].payload[..]),
            (first_id, b"first-1".as_slice())
        );

        assert_eq!(
            (packets[1].subscription_id, &packets[1].payload[..]),
            (second_id, b"second-1".as_slice())
        );
    }

    #[test]
    fn try_recv_all_into_drains_all_available_packets() {
        let mut context = Context::new();
        let first_config = sample_config(9009);
        let second_config = sample_config(9010);

        let first_id = context.add_subscription(first_config.clone()).unwrap();
        context.join_subscription(first_id).unwrap();
        let second_id = context.add_subscription(second_config.clone()).unwrap();
        context.join_subscription(second_id).unwrap();

        send_round_robin_test_packets(&first_config, &second_config);

        let deadline = Instant::now() + Duration::from_secs(1);
        let mut packets = Vec::new();

        while packets.len() < 3 && Instant::now() < deadline {
            context.try_recv_all_into(&mut packets).unwrap();

            if packets.len() < 3 {
                thread::sleep(Duration::from_millis(10));
            }
        }

        assert_eq!(packets.len(), 3);

        assert_eq!(
            (packets[0].subscription_id, &packets[0].payload[..]),
            (first_id, b"first-1".as_slice())
        );

        assert_eq!(
            (packets[1].subscription_id, &packets[1].payload[..]),
            (second_id, b"second-1".as_slice())
        );

        assert_eq!(
            (packets[2].subscription_id, &packets[2].payload[..]),
            (first_id, b"first-2".as_slice())
        );
    }

    #[test]
    fn add_subscription_creates_bound_subscription() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9011)).unwrap();

        let subscription = context.get_subscription(id).unwrap();
        assert_eq!(subscription.state(), SubscriptionState::Bound);
    }

    #[test]
    fn join_subscription_transitions_bound_to_joined() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9012)).unwrap();

        context.join_subscription(id).unwrap();

        let subscription = context.get_subscription(id).unwrap();
        assert_eq!(subscription.state(), SubscriptionState::Joined);
    }

    #[test]
    fn leave_subscription_transitions_joined_to_bound() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9013)).unwrap();

        context.join_subscription(id).unwrap();
        context.leave_subscription(id).unwrap();

        let subscription = context.get_subscription(id).unwrap();
        assert_eq!(subscription.state(), SubscriptionState::Bound);
    }

    #[test]
    fn join_subscription_rejects_already_joined_subscription() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9014)).unwrap();

        context.join_subscription(id).unwrap();
        let result = context.join_subscription(id);

        assert!(matches!(result, Err(McrxError::SubscriptionAlreadyJoined)));
    }

    #[test]
    fn leave_subscription_rejects_not_joined_subscription() {
        let mut context = Context::new();
        let id = context.add_subscription(sample_config(9015)).unwrap();

        let result = context.leave_subscription(id);

        assert!(matches!(result, Err(McrxError::SubscriptionNotJoined)));
    }
}
