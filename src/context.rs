use crate::config::SubscriptionConfig;
use crate::error::McrxError;
use crate::packet::Packet;
use crate::platform::open_and_join_socket;
use crate::subscription::{Subscription, SubscriptionId};

/// Owns and manages the set of active subscriptions.
#[derive(Debug, Default)]
pub struct Context {
    subscriptions: Vec<Subscription>,
    next_subscription_id: u64,
}

impl Context {
    /// Creates an empty context with no subscriptions.
    pub fn new() -> Self {
        Self {
            subscriptions: Vec::new(),
            next_subscription_id: 1,
        }
    }

    /// Returns the number of active subscriptions currently stored in the context.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
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
    /// This function creates the socket, binds it, and attempts to join the multicast group
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

        let socket = open_and_join_socket(&config)?;

        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;

        let subscription = Subscription::new(id, config, socket);
        self.subscriptions.push(subscription);

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
            true
        } else {
            false
        }
    }

    /// Returns a read-only slice of all subscriptions currently stored in the context.
    pub fn subscriptions(&self) -> &[Subscription] {
        &self.subscriptions
    }

    /// Attempts to receive a single packet from any subscription without blocking.
    ///
    /// Returns the first available packet, if any subscription currently has one
    /// ready to be read.
    pub fn try_recv_any(&self) -> Result<Option<Packet>, McrxError> {
        for subscription in &self.subscriptions {
            if let Some(packet) = subscription.try_recv()? {
                return Ok(Some(packet));
            }
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourceFilter;
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
        context.add_subscription(sample_config(9002)).unwrap();

        let result = context.try_recv_any().unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn try_recv_any_returns_packet_from_ready_subscription() {
        let mut context = Context::new();
        let config = sample_config(9003);
        context.add_subscription(config.clone()).unwrap();

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
}
