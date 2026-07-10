use crate::error::McrxError;
use crate::packet::ReceiveMetadata;
use crate::platform::{
    RawCapturedDatagram, RawReceiveSocket, RawSharedCaptureKey, join_raw_multicast_group,
    leave_raw_multicast_group, open_shared_raw_socket, recv_shared_raw_datagram,
    shared_raw_capture_key,
};
use crate::raw::{RawPacket, RawSubscriptionConfig};
use crate::subscription::{SubscriptionId, SubscriptionState};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// Limits for a [`SharedRawContext`].
///
/// The pending-packet limit bounds userspace memory when the caller is slower
/// than the capture sockets. The subscription limit bounds the logical
/// membership index, not the number of operating-system capture sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedRawContextLimits {
    /// Maximum number of logical raw subscriptions stored in the context.
    pub max_subscriptions: usize,
    /// Maximum number of demultiplexed packets buffered for the caller.
    pub max_pending_packets: usize,
}

impl Default for SharedRawContextLimits {
    fn default() -> Self {
        Self {
            max_subscriptions: 4_096,
            max_pending_packets: 1_024,
        }
    }
}

/// A logical raw multicast subscription managed by [`SharedRawContext`].
///
/// Multiple joined subscriptions can share one underlying capture socket when
/// they use the same IP family and resolved interface. Duplicate logical
/// subscriptions share a reference-counted kernel membership while each handle
/// keeps its own lifecycle state.
#[derive(Debug, Clone)]
pub struct SharedRawSubscription {
    id: SubscriptionId,
    config: RawSubscriptionConfig,
    capture_key: RawSharedCaptureKey,
    state: SubscriptionState,
}

impl SharedRawSubscription {
    fn new(
        id: SubscriptionId,
        config: RawSubscriptionConfig,
        capture_key: RawSharedCaptureKey,
    ) -> Self {
        Self {
            id,
            config,
            capture_key,
            state: SubscriptionState::Bound,
        }
    }

    /// Returns the subscription's ID.
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    /// Returns the immutable subscription configuration.
    pub fn config(&self) -> &RawSubscriptionConfig {
        &self.config
    }

    /// Returns the current lifecycle state.
    pub fn state(&self) -> SubscriptionState {
        self.state
    }

    /// Returns `true` when this logical subscription has joined its group.
    pub fn is_joined(&self) -> bool {
        matches!(self.state, SubscriptionState::Joined)
    }
}

/// One raw IP datagram and every joined logical subscription that matched it.
///
/// The kernel datagram is captured once and the [`RawPacket`] uses the first
/// matching ID in deterministic ascending order as its primary subscription.
/// Use [`Self::matching_subscription_ids`] when overlapping logical
/// memberships need to be distinguished.
#[derive(Debug, Clone)]
pub struct SharedRawPacket {
    /// The received complete IP datagram and primary matching subscription ID.
    pub packet: RawPacket,
    matching_subscription_ids: MatchingSubscriptionIds,
}

impl SharedRawPacket {
    /// Returns the captured complete IP datagram.
    pub fn packet(&self) -> &RawPacket {
        &self.packet
    }

    /// Returns every joined logical subscription matched by this datagram.
    pub fn matching_subscription_ids(&self) -> &[SubscriptionId] {
        self.matching_subscription_ids.as_slice()
    }
}

#[derive(Debug, Clone)]
enum MatchingSubscriptionIds {
    One([SubscriptionId; 1]),
    Many(Vec<SubscriptionId>),
}

impl MatchingSubscriptionIds {
    fn from_sorted_slices(first: &[SubscriptionId], second: &[SubscriptionId]) -> Option<Self> {
        match first.len() + second.len() {
            0 => None,
            1 => {
                let id = match first.first() {
                    Some(id) => *id,
                    None => second[0],
                };
                Some(Self::One([id]))
            }
            total => Some(Self::Many(merge_ids(first, second, total))),
        }
    }

    fn as_slice(&self) -> &[SubscriptionId] {
        match self {
            Self::One(id) => id,
            Self::Many(ids) => ids,
        }
    }

    fn first(&self) -> SubscriptionId {
        self.as_slice()[0]
    }

    fn remove_existing(&mut self, index: usize) {
        let Self::Many(ids) = self else {
            unreachable!("a single matching subscription is removed with its queued packet");
        };

        ids.remove(index);
        if ids.len() == 1 {
            *self = Self::One([ids[0]]);
        }
    }
}

#[derive(Debug, Default)]
struct GroupDemultiplexer {
    any_source: Vec<SubscriptionId>,
    sources: HashMap<IpAddr, Vec<SubscriptionId>>,
}

impl GroupDemultiplexer {
    fn insert(&mut self, id: SubscriptionId, source: Option<IpAddr>) {
        let ids = match source {
            Some(source) => self.sources.entry(source).or_default(),
            None => &mut self.any_source,
        };
        insert_id(ids, id);
    }

    fn remove(&mut self, id: SubscriptionId, source: Option<IpAddr>) {
        match source {
            Some(source) => {
                if let Some(ids) = self.sources.get_mut(&source) {
                    remove_id(ids, id);
                    if ids.is_empty() {
                        self.sources.remove(&source);
                    }
                }
            }
            None => remove_id(&mut self.any_source, id),
        }
    }

    fn is_empty(&self) -> bool {
        self.any_source.is_empty() && self.sources.is_empty()
    }

    fn matches(&self, source: IpAddr) -> Option<MatchingSubscriptionIds> {
        let source_matches = self.sources.get(&source).map_or(&[][..], Vec::as_slice);
        MatchingSubscriptionIds::from_sorted_slices(&self.any_source, source_matches)
    }
}

#[derive(Debug, Default)]
struct CaptureDemultiplexer {
    groups: HashMap<IpAddr, GroupDemultiplexer>,
}

impl CaptureDemultiplexer {
    fn insert(&mut self, id: SubscriptionId, config: &RawSubscriptionConfig) {
        self.groups
            .entry(config.group)
            .or_default()
            .insert(id, config.source_addr());
    }

    fn remove(&mut self, id: SubscriptionId, config: &RawSubscriptionConfig) {
        let remove_group = if let Some(group) = self.groups.get_mut(&config.group) {
            group.remove(id, config.source_addr());
            group.is_empty()
        } else {
            false
        };

        if remove_group {
            self.groups.remove(&config.group);
        }
    }

    fn matches(&self, group: IpAddr, source: IpAddr) -> Option<MatchingSubscriptionIds> {
        self.groups.get(&group)?.matches(source)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct KernelMembershipKey {
    group: IpAddr,
    source: Option<IpAddr>,
}

impl From<&RawSubscriptionConfig> for KernelMembershipKey {
    fn from(config: &RawSubscriptionConfig) -> Self {
        Self {
            group: config.group,
            source: config.source_addr(),
        }
    }
}

#[derive(Debug)]
struct KernelMembership {
    references: usize,
    joined_config: RawSubscriptionConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CaptureSelector {
    family: crate::SubscriptionAddressFamily,
    interface: Option<IpAddr>,
    interface_index: Option<u32>,
}

impl From<&RawSubscriptionConfig> for CaptureSelector {
    fn from(config: &RawSubscriptionConfig) -> Self {
        Self {
            family: config.family(),
            interface: config.interface,
            interface_index: config.interface_index,
        }
    }
}

#[derive(Debug)]
struct ResolvedCaptureKey {
    key: RawSharedCaptureKey,
    references: usize,
}

#[derive(Debug)]
struct SharedCaptureSocket {
    socket: RawReceiveSocket,
    memberships: usize,
    kernel_memberships: HashMap<KernelMembershipKey, KernelMembership>,
    demultiplexer: CaptureDemultiplexer,
}

impl SharedCaptureSocket {
    fn new(socket: RawReceiveSocket) -> Self {
        Self {
            socket,
            memberships: 0,
            kernel_memberships: HashMap::new(),
            demultiplexer: CaptureDemultiplexer::default(),
        }
    }
}

#[cfg(feature = "metrics")]
#[derive(Debug, Default)]
struct SharedRawCaptureMetrics {
    received_packets_total: u64,
    unmatched_packets_total: u64,
    demultiplex_matches_total: u64,
}

/// A point-in-time snapshot of shared raw capture activity.
#[cfg(feature = "metrics")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedRawCaptureMetricsSnapshot {
    /// Number of open capture sockets, normally one per family/interface tuple.
    pub capture_socket_count: usize,
    /// Number of joined logical multicast memberships.
    pub active_memberships: usize,
    /// Complete IP datagrams read from capture sockets since context creation.
    pub received_packets_total: u64,
    /// Captured datagrams whose source/group matched no logical membership.
    pub unmatched_packets_total: u64,
    /// Sum of logical membership matches across received datagrams.
    pub demultiplex_matches_total: u64,
    /// Demultiplexed packets currently queued for the caller.
    pub pending_packets: usize,
}

/// A raw multicast context that shares one capture socket per family/interface.
///
/// This Linux-only opt-in transport primitive is intended for applications
/// with many logical memberships. It manages each logical lifecycle
/// independently, reference-counts duplicate kernel memberships, captures each
/// kernel IP datagram once, and looks up matching subscriptions through a
/// `(group, source)` index. macOS and Windows return
/// [`McrxError::RawPacketReceiveUnsupported`] instead of falling back to the
/// per-subscription backend.
#[derive(Debug)]
pub struct SharedRawContext {
    subscriptions: HashMap<SubscriptionId, SharedRawSubscription>,
    resolved_capture_keys: HashMap<CaptureSelector, ResolvedCaptureKey>,
    captures: HashMap<RawSharedCaptureKey, SharedCaptureSocket>,
    capture_order: Vec<RawSharedCaptureKey>,
    pending_packets: VecDeque<SharedRawPacket>,
    limits: SharedRawContextLimits,
    next_subscription_id: u64,
    next_capture_index: usize,
    #[cfg(feature = "metrics")]
    metrics: SharedRawCaptureMetrics,
}

impl Default for SharedRawContext {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedRawContext {
    /// Creates an empty shared raw capture context with default bounds.
    pub fn new() -> Self {
        Self::with_limits(SharedRawContextLimits::default())
            .expect("default shared raw capture limits are valid")
    }

    /// Creates an empty shared raw capture context with explicit bounds.
    pub fn with_limits(limits: SharedRawContextLimits) -> Result<Self, McrxError> {
        if limits.max_subscriptions == 0 || limits.max_pending_packets == 0 {
            return Err(McrxError::InvalidSharedRawCaptureLimits);
        }

        Ok(Self {
            subscriptions: HashMap::new(),
            resolved_capture_keys: HashMap::new(),
            captures: HashMap::new(),
            capture_order: Vec::new(),
            pending_packets: VecDeque::new(),
            limits,
            next_subscription_id: 1,
            next_capture_index: 0,
            #[cfg(feature = "metrics")]
            metrics: SharedRawCaptureMetrics::default(),
        })
    }

    /// Returns the configured shared raw capture bounds.
    pub fn limits(&self) -> SharedRawContextLimits {
        self.limits
    }

    /// Returns the number of stored logical subscriptions.
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// Returns the number of currently open shared capture sockets.
    pub fn capture_socket_count(&self) -> usize {
        self.captures.len()
    }

    /// Returns the number of joined logical memberships.
    pub fn active_membership_count(&self) -> usize {
        self.captures
            .values()
            .map(|capture| capture.memberships)
            .sum()
    }

    /// Returns the number of demultiplexed packets waiting for the caller.
    pub fn pending_packet_count(&self) -> usize {
        self.pending_packets.len()
    }

    /// Returns true if a logical subscription with the given ID exists.
    pub fn contains_subscription(&self, id: SubscriptionId) -> bool {
        self.subscriptions.contains_key(&id)
    }

    /// Returns an immutable logical subscription, if present.
    pub fn get_subscription(&self, id: SubscriptionId) -> Option<&SharedRawSubscription> {
        self.subscriptions.get(&id)
    }

    /// Adds a bounded logical raw subscription without joining it yet.
    ///
    /// Unlike [`crate::RawContext`], this context permits duplicate configs.
    /// Joined duplicates share one kernel membership and are returned as
    /// separate IDs by [`SharedRawPacket::matching_subscription_ids`].
    ///
    /// On supported platforms this operation resolves the capture interface
    /// key, but does not allocate a raw capture socket until [`Self::join_subscription`].
    pub fn add_subscription(
        &mut self,
        config: RawSubscriptionConfig,
    ) -> Result<SubscriptionId, McrxError> {
        config.validate()?;
        if self.subscriptions.len() >= self.limits.max_subscriptions {
            return Err(McrxError::SharedRawCaptureSubscriptionLimitExceeded {
                limit: self.limits.max_subscriptions,
            });
        }

        let capture_key = self.resolve_capture_key(&config)?;
        let id = SubscriptionId(self.next_subscription_id);
        self.next_subscription_id += 1;
        self.subscriptions
            .insert(id, SharedRawSubscription::new(id, config, capture_key));
        Ok(id)
    }

    /// Joins one logical multicast membership.
    ///
    /// The membership is installed on the capture socket matching its resolved
    /// family/interface key. An identical joined membership increments its
    /// kernel reference count instead of repeating the socket option. Other
    /// memberships on that socket remain intact.
    pub fn join_subscription(&mut self, id: SubscriptionId) -> Result<(), McrxError> {
        let (capture_key, config) = {
            let subscription = self
                .subscriptions
                .get(&id)
                .ok_or(McrxError::SubscriptionNotFound)?;
            if subscription.is_joined() {
                return Err(McrxError::SubscriptionAlreadyJoined);
            }
            (subscription.capture_key, subscription.config.clone())
        };
        let membership_key = KernelMembershipKey::from(&config);

        let created_capture = match self.captures.entry(capture_key) {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                let socket = open_shared_raw_socket(capture_key)?;
                entry.insert(SharedCaptureSocket::new(socket));
                self.capture_order.push(capture_key);
                true
            }
        };

        let needs_kernel_join = !self
            .captures
            .get(&capture_key)
            .expect("shared capture was inserted or already existed")
            .kernel_memberships
            .contains_key(&membership_key);
        let join_result = if needs_kernel_join {
            let capture = self
                .captures
                .get(&capture_key)
                .expect("shared capture was inserted or already existed");
            join_raw_multicast_group(&capture.socket, &config)
        } else {
            Ok(())
        };
        if let Err(error) = join_result {
            if created_capture {
                self.remove_capture(capture_key);
            }
            return Err(error);
        }

        let capture = self
            .captures
            .get_mut(&capture_key)
            .expect("joined shared capture exists");
        match capture.kernel_memberships.entry(membership_key) {
            Entry::Occupied(mut entry) => entry.get_mut().references += 1,
            Entry::Vacant(entry) => {
                entry.insert(KernelMembership {
                    references: 1,
                    joined_config: config.clone(),
                });
            }
        }
        capture.demultiplexer.insert(id, &config);
        capture.memberships += 1;
        self.subscriptions
            .get_mut(&id)
            .expect("joined subscription exists")
            .state = SubscriptionState::Joined;
        Ok(())
    }

    /// Leaves one logical membership without affecting other shared memberships.
    pub fn leave_subscription(&mut self, id: SubscriptionId) -> Result<(), McrxError> {
        let (capture_key, config) = {
            let subscription = self
                .subscriptions
                .get(&id)
                .ok_or(McrxError::SubscriptionNotFound)?;
            if !subscription.is_joined() {
                return Err(McrxError::SubscriptionNotJoined);
            }
            (subscription.capture_key, subscription.config.clone())
        };
        let membership_key = KernelMembershipKey::from(&config);

        let should_remove_capture = {
            let capture = self
                .captures
                .get_mut(&capture_key)
                .expect("joined subscription has a shared capture");
            let joined_config = {
                let membership = capture
                    .kernel_memberships
                    .get_mut(&membership_key)
                    .expect("joined subscription has a kernel membership");
                if membership.references == 1 {
                    Some(membership.joined_config.clone())
                } else {
                    membership.references -= 1;
                    None
                }
            };
            if let Some(joined_config) = joined_config {
                leave_raw_multicast_group(&capture.socket, &joined_config)?;
                capture.kernel_memberships.remove(&membership_key);
            }
            capture.demultiplexer.remove(id, &config);
            capture.memberships = capture
                .memberships
                .checked_sub(1)
                .expect("membership count");
            debug_assert_eq!(
                capture.memberships == 0,
                capture.kernel_memberships.is_empty()
            );
            capture.memberships == 0
        };

        self.subscriptions
            .get_mut(&id)
            .expect("joined subscription exists")
            .state = SubscriptionState::Bound;
        self.remove_pending_membership(id);
        if should_remove_capture {
            self.remove_capture(capture_key);
        }
        Ok(())
    }

    /// Removes one logical subscription and leaves it first when necessary.
    ///
    /// Returns `false` when no such subscription exists.
    pub fn remove_subscription(&mut self, id: SubscriptionId) -> Result<bool, McrxError> {
        if !self.subscriptions.contains_key(&id) {
            return Ok(false);
        }
        if self
            .subscriptions
            .get(&id)
            .expect("subscription existence checked")
            .is_joined()
        {
            self.leave_subscription(id)?;
        }
        let removed = self
            .subscriptions
            .remove(&id)
            .expect("subscription existence checked");
        self.release_capture_key(&removed.config);
        Ok(true)
    }

    /// Attempts to receive one complete IP datagram without blocking.
    ///
    /// Capture sockets are visited in round-robin order. A packet is read once
    /// and returned with all matching logical subscription IDs.
    pub fn try_recv_any(&mut self) -> Result<Option<SharedRawPacket>, McrxError> {
        if let Some(packet) = self.pending_packets.pop_front() {
            return Ok(Some(packet));
        }

        self.fill_pending(1)?;
        Ok(self.pending_packets.pop_front())
    }

    /// Receives up to `max_packets` complete IP datagrams without blocking.
    ///
    /// Returns the number of packets appended to `packets`.
    pub fn try_recv_batch_into(
        &mut self,
        packets: &mut Vec<SharedRawPacket>,
        max_packets: usize,
    ) -> Result<usize, McrxError> {
        if max_packets == 0 {
            return Ok(0);
        }

        let mut received = self.drain_pending_into(packets, max_packets);
        while received < max_packets {
            let queued = self.fill_pending(max_packets - received)?;
            if queued == 0 {
                break;
            }
            received += self.drain_pending_into(packets, max_packets - received);
        }
        Ok(received)
    }

    /// Returns a metrics snapshot for the shared capture backend.
    #[cfg(feature = "metrics")]
    pub fn metrics_snapshot(&self) -> SharedRawCaptureMetricsSnapshot {
        SharedRawCaptureMetricsSnapshot {
            capture_socket_count: self.capture_socket_count(),
            active_memberships: self.active_membership_count(),
            received_packets_total: self.metrics.received_packets_total,
            unmatched_packets_total: self.metrics.unmatched_packets_total,
            demultiplex_matches_total: self.metrics.demultiplex_matches_total,
            pending_packets: self.pending_packet_count(),
        }
    }

    fn fill_pending(&mut self, max_new_packets: usize) -> Result<usize, McrxError> {
        let target = max_new_packets.min(
            self.limits
                .max_pending_packets
                .saturating_sub(self.pending_packets.len()),
        );
        if self.capture_order.is_empty() || target == 0 {
            return Ok(0);
        }

        let capture_count = self.capture_order.len();
        let mut queued_packets = 0;
        let mut captured_datagrams = 0;
        let capture_budget = target.saturating_add(capture_count);

        'captures: for _ in 0..capture_count {
            let capture_key = self.capture_order[self.next_capture_index];
            self.next_capture_index = (self.next_capture_index + 1) % capture_count;

            for _ in 0..target {
                if queued_packets == target || captured_datagrams == capture_budget {
                    break 'captures;
                }

                let captured_and_matches = {
                    let capture = self
                        .captures
                        .get(&capture_key)
                        .expect("capture order and capture map stay in sync");
                    let Some(captured) = recv_shared_raw_datagram(&capture.socket, capture_key)?
                    else {
                        break;
                    };
                    let matches = capture
                        .demultiplexer
                        .matches(captured.group, captured.source_ip);
                    (captured, matches)
                };
                captured_datagrams += 1;
                self.record_received_packet();

                let (captured, Some(matching_ids)) = captured_and_matches else {
                    self.record_unmatched_packet();
                    continue;
                };

                self.record_demultiplex_matches(matching_ids.as_slice().len());
                let primary_id = matching_ids.first();
                let packet = {
                    let primary_config = &self
                        .subscriptions
                        .get(&primary_id)
                        .expect("demultiplexer only contains registered subscriptions")
                        .config;
                    raw_packet_from_captured(primary_id, primary_config, captured)
                };
                self.pending_packets.push_back(SharedRawPacket {
                    packet,
                    matching_subscription_ids: matching_ids,
                });
                queued_packets += 1;
            }
        }
        Ok(queued_packets)
    }

    fn drain_pending_into(
        &mut self,
        packets: &mut Vec<SharedRawPacket>,
        max_packets: usize,
    ) -> usize {
        let drained = max_packets.min(self.pending_packets.len());
        packets.extend(self.pending_packets.drain(..drained));
        drained
    }

    fn remove_capture(&mut self, capture_key: RawSharedCaptureKey) {
        self.captures.remove(&capture_key);
        let index = self
            .capture_order
            .iter()
            .position(|key| *key == capture_key)
            .expect("capture order contains every capture map key");
        self.capture_order.remove(index);
        if self.capture_order.is_empty() {
            self.next_capture_index = 0;
        } else {
            self.next_capture_index %= self.capture_order.len();
        }
    }

    fn resolve_capture_key(
        &mut self,
        config: &RawSubscriptionConfig,
    ) -> Result<RawSharedCaptureKey, McrxError> {
        let selector = CaptureSelector::from(config);
        match self.resolved_capture_keys.entry(selector) {
            Entry::Occupied(mut entry) => {
                let resolved = entry.get_mut();
                resolved.references += 1;
                Ok(resolved.key)
            }
            Entry::Vacant(entry) => {
                let key = shared_raw_capture_key(config)?;
                entry.insert(ResolvedCaptureKey { key, references: 1 });
                Ok(key)
            }
        }
    }

    fn release_capture_key(&mut self, config: &RawSubscriptionConfig) {
        let selector = CaptureSelector::from(config);
        let remove = {
            let resolved = self
                .resolved_capture_keys
                .get_mut(&selector)
                .expect("stored subscription has a resolved capture key");
            resolved.references = resolved
                .references
                .checked_sub(1)
                .expect("resolved capture key reference count");
            resolved.references == 0
        };
        if remove {
            self.resolved_capture_keys.remove(&selector);
        }
    }

    fn remove_pending_membership(&mut self, id: SubscriptionId) {
        let subscriptions = &self.subscriptions;
        self.pending_packets.retain_mut(|shared_packet| {
            let Some(index) = shared_packet
                .matching_subscription_ids
                .as_slice()
                .iter()
                .position(|matching_id| *matching_id == id)
            else {
                return true;
            };

            if shared_packet.matching_subscription_ids.as_slice().len() == 1 {
                return false;
            }

            shared_packet
                .matching_subscription_ids
                .remove_existing(index);
            let primary_id = shared_packet.matching_subscription_ids.first();
            if shared_packet.packet.subscription_id != primary_id {
                let primary_config = &subscriptions
                    .get(&primary_id)
                    .expect("queued packet only references registered subscriptions")
                    .config;
                shared_packet.packet.subscription_id = primary_id;
                shared_packet.packet.metadata.configured_interface = primary_config.interface;
                shared_packet.packet.metadata.configured_interface_index =
                    primary_config.interface_index;
            }
            true
        });
    }

    #[cfg(not(feature = "metrics"))]
    fn record_received_packet(&mut self) {}

    #[cfg(feature = "metrics")]
    fn record_received_packet(&mut self) {
        self.metrics.received_packets_total = self.metrics.received_packets_total.saturating_add(1);
    }

    #[cfg(not(feature = "metrics"))]
    fn record_unmatched_packet(&mut self) {}

    #[cfg(feature = "metrics")]
    fn record_unmatched_packet(&mut self) {
        self.metrics.unmatched_packets_total =
            self.metrics.unmatched_packets_total.saturating_add(1);
    }

    #[cfg(not(feature = "metrics"))]
    fn record_demultiplex_matches(&mut self, _matches: usize) {}

    #[cfg(feature = "metrics")]
    fn record_demultiplex_matches(&mut self, matches: usize) {
        self.metrics.demultiplex_matches_total = self
            .metrics
            .demultiplex_matches_total
            .saturating_add(matches as u64);
    }
}

fn raw_packet_from_captured(
    subscription_id: SubscriptionId,
    config: &RawSubscriptionConfig,
    captured: RawCapturedDatagram,
) -> RawPacket {
    RawPacket {
        subscription_id,
        datagram: captured.datagram,
        source_ip: Some(captured.source_ip),
        group: Some(captured.group),
        ip_protocol: Some(captured.ip_protocol),
        metadata: ReceiveMetadata {
            socket_local_addr: None,
            configured_interface: config.interface,
            configured_interface_index: config.interface_index,
            destination_local_ip: Some(captured.group),
            ingress_interface_index: captured.ingress_interface_index,
        },
    }
}

fn insert_id(ids: &mut Vec<SubscriptionId>, id: SubscriptionId) {
    match ids.binary_search_by_key(&id.0, |existing| existing.0) {
        Ok(_) => {}
        Err(index) => ids.insert(index, id),
    }
}

fn remove_id(ids: &mut Vec<SubscriptionId>, id: SubscriptionId) {
    if let Ok(index) = ids.binary_search_by_key(&id.0, |existing| existing.0) {
        ids.remove(index);
    }
}

fn merge_ids(
    first: &[SubscriptionId],
    second: &[SubscriptionId],
    total: usize,
) -> Vec<SubscriptionId> {
    let mut matches = Vec::with_capacity(total);
    let (mut first_index, mut second_index) = (0, 0);
    while first_index < first.len() && second_index < second.len() {
        if first[first_index].0 < second[second_index].0 {
            matches.push(first[first_index]);
            first_index += 1;
        } else if first[first_index].0 > second[second_index].0 {
            matches.push(second[second_index]);
            second_index += 1;
        } else {
            matches.push(first[first_index]);
            first_index += 1;
            second_index += 1;
        }
    }
    matches.extend_from_slice(&first[first_index..]);
    matches.extend_from_slice(&second[second_index..]);
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceFilter;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn asm(group: Ipv4Addr) -> RawSubscriptionConfig {
        RawSubscriptionConfig::asm(group)
    }

    fn ssm(group: Ipv4Addr, source: Ipv4Addr) -> RawSubscriptionConfig {
        RawSubscriptionConfig::ssm(group, source)
    }

    #[test]
    fn demultiplexer_matches_duplicate_logical_memberships_once_each() {
        let group = Ipv4Addr::new(239, 1, 2, 3);
        let source = Ipv4Addr::new(192, 0, 2, 10);
        let config = asm(group);
        let mut demux = CaptureDemultiplexer::default();
        demux.insert(SubscriptionId(9), &config);
        demux.insert(SubscriptionId(3), &config);

        assert_eq!(
            demux
                .matches(group.into(), source.into())
                .unwrap()
                .as_slice(),
            &[SubscriptionId(3), SubscriptionId(9)]
        );
    }

    #[test]
    fn demultiplexer_uses_group_then_source_without_cross_group_matches() {
        let first_group = Ipv4Addr::new(239, 1, 2, 3);
        let second_group = Ipv4Addr::new(239, 1, 2, 4);
        let source = Ipv4Addr::new(192, 0, 2, 10);
        let mut demux = CaptureDemultiplexer::default();
        demux.insert(SubscriptionId(1), &ssm(first_group, source));
        demux.insert(SubscriptionId(2), &asm(second_group));

        assert!(
            demux
                .matches(first_group.into(), Ipv4Addr::new(192, 0, 2, 11).into())
                .is_none()
        );
        assert_eq!(
            demux
                .matches(second_group.into(), source.into())
                .unwrap()
                .as_slice(),
            &[SubscriptionId(2)]
        );
        assert!(matches!(
            demux.matches(second_group.into(), source.into()),
            Some(MatchingSubscriptionIds::One(_))
        ));
    }

    #[test]
    fn removing_one_duplicate_does_not_disturb_the_other_logical_membership() {
        let group = Ipv4Addr::new(239, 1, 2, 3);
        let source = Ipv4Addr::new(192, 0, 2, 10);
        let config = asm(group);
        let mut demux = CaptureDemultiplexer::default();
        demux.insert(SubscriptionId(1), &config);
        demux.insert(SubscriptionId(2), &config);
        demux.remove(SubscriptionId(1), &config);

        assert_eq!(
            demux
                .matches(group.into(), source.into())
                .unwrap()
                .as_slice(),
            &[SubscriptionId(2)]
        );
        assert_eq!(
            demux
                .matches(group.into(), Ipv4Addr::new(192, 0, 2, 11).into())
                .unwrap()
                .as_slice(),
            &[SubscriptionId(2)]
        );
    }

    #[test]
    fn kernel_membership_keys_track_group_and_source_within_one_capture() {
        let group = Ipv4Addr::new(239, 1, 2, 3);
        let first = asm(group);
        let mut duplicate = first.clone();
        duplicate.interface = Some(Ipv4Addr::new(192, 0, 2, 10).into());

        assert_eq!(
            KernelMembershipKey::from(&first),
            KernelMembershipKey::from(&duplicate)
        );
        assert_ne!(
            KernelMembershipKey::from(&first),
            KernelMembershipKey::from(&asm(Ipv4Addr::new(239, 1, 2, 4)))
        );

        let ssm_group = Ipv4Addr::new(232, 1, 2, 3);
        assert_ne!(
            KernelMembershipKey::from(&ssm(ssm_group, Ipv4Addr::new(192, 0, 2, 10))),
            KernelMembershipKey::from(&ssm(ssm_group, Ipv4Addr::new(192, 0, 2, 11)))
        );
    }

    #[test]
    fn resolved_capture_key_cache_is_bounded_by_stored_selectors() {
        let config = asm(Ipv4Addr::new(239, 1, 2, 3));
        let selector = CaptureSelector::from(&config);
        let capture_key = RawSharedCaptureKey {
            family: crate::SubscriptionAddressFamily::Ipv4,
            interface_index: 2,
        };
        let mut context = SharedRawContext::new();
        context.resolved_capture_keys.insert(
            selector,
            ResolvedCaptureKey {
                key: capture_key,
                references: 2,
            },
        );

        context.release_capture_key(&config);
        assert_eq!(context.resolved_capture_keys[&selector].references, 1);

        context.release_capture_key(&config);
        assert!(context.resolved_capture_keys.is_empty());
    }

    #[test]
    fn capture_keys_separate_address_families_and_interfaces() {
        let ipv4_eth0 = RawSharedCaptureKey {
            family: crate::SubscriptionAddressFamily::Ipv4,
            interface_index: 2,
        };
        let ipv4_eth1 = RawSharedCaptureKey {
            family: crate::SubscriptionAddressFamily::Ipv4,
            interface_index: 3,
        };
        let ipv6_eth0 = RawSharedCaptureKey {
            family: crate::SubscriptionAddressFamily::Ipv6,
            interface_index: 2,
        };

        assert_ne!(ipv4_eth0, ipv4_eth1);
        assert_ne!(ipv4_eth0, ipv6_eth0);
    }

    #[test]
    fn limits_reject_zero_and_default_is_bounded() {
        assert!(matches!(
            SharedRawContext::with_limits(SharedRawContextLimits {
                max_subscriptions: 0,
                max_pending_packets: 1,
            }),
            Err(McrxError::InvalidSharedRawCaptureLimits)
        ));
        assert!(matches!(
            SharedRawContext::with_limits(SharedRawContextLimits {
                max_subscriptions: 1,
                max_pending_packets: 0,
            }),
            Err(McrxError::InvalidSharedRawCaptureLimits)
        ));
        assert!(SharedRawContextLimits::default().max_pending_packets > 0);
    }

    #[test]
    fn subscription_limit_is_enforced_before_platform_socket_setup() {
        let first_config = asm(Ipv4Addr::new(239, 1, 2, 3));
        let mut context = SharedRawContext::with_limits(SharedRawContextLimits {
            max_subscriptions: 1,
            max_pending_packets: 1,
        })
        .unwrap();
        context.subscriptions.insert(
            SubscriptionId(1),
            SharedRawSubscription::new(
                SubscriptionId(1),
                first_config,
                RawSharedCaptureKey {
                    family: crate::SubscriptionAddressFamily::Ipv4,
                    interface_index: 2,
                },
            ),
        );

        let error = context
            .add_subscription(asm(Ipv4Addr::new(239, 1, 2, 4)))
            .unwrap_err();
        assert!(matches!(
            error,
            McrxError::SharedRawCaptureSubscriptionLimitExceeded { limit: 1 }
        ));
    }

    #[test]
    fn raw_packet_from_capture_preserves_complete_datagram_and_metadata() {
        let config = RawSubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3));
        let packet = raw_packet_from_captured(
            SubscriptionId(7),
            &config,
            RawCapturedDatagram {
                datagram: bytes::Bytes::from_static(&[0x45, 0, 0, 20]),
                source_ip: Ipv4Addr::new(192, 0, 2, 10).into(),
                group: config.group,
                ip_protocol: 17,
                ingress_interface_index: Some(2),
            },
        );

        assert_eq!(packet.datagram.as_ref(), &[0x45, 0, 0, 20]);
        assert_eq!(packet.source_ip, Some(Ipv4Addr::new(192, 0, 2, 10).into()));
        assert_eq!(packet.metadata.ingress_interface_index, Some(2));
    }

    #[test]
    fn removing_a_pending_overlap_keeps_the_remaining_membership() {
        let first_id = SubscriptionId(1);
        let second_id = SubscriptionId(2);
        let first_config = asm(Ipv4Addr::new(239, 1, 2, 3));
        let second_config = first_config.clone();
        let capture_key = RawSharedCaptureKey {
            family: crate::SubscriptionAddressFamily::Ipv4,
            interface_index: 2,
        };
        let mut context = SharedRawContext::new();
        context.subscriptions.insert(
            first_id,
            SharedRawSubscription::new(first_id, first_config.clone(), capture_key),
        );
        context.subscriptions.insert(
            second_id,
            SharedRawSubscription::new(second_id, second_config.clone(), capture_key),
        );
        context.pending_packets.push_back(SharedRawPacket {
            packet: raw_packet_from_captured(
                first_id,
                &first_config,
                RawCapturedDatagram {
                    datagram: bytes::Bytes::from_static(&[0x45, 0, 0, 20]),
                    source_ip: Ipv4Addr::new(192, 0, 2, 10).into(),
                    group: first_config.group,
                    ip_protocol: 17,
                    ingress_interface_index: Some(2),
                },
            ),
            matching_subscription_ids: MatchingSubscriptionIds::Many(vec![first_id, second_id]),
        });

        context.remove_pending_membership(first_id);

        let packet = context.pending_packets.front().expect("remaining overlap");
        assert_eq!(packet.packet.subscription_id, second_id);
        assert_eq!(packet.matching_subscription_ids(), &[second_id]);
    }

    #[test]
    fn batch_receive_drains_queued_packets_up_to_the_requested_limit() {
        fn queued_packet(id: SubscriptionId) -> SharedRawPacket {
            SharedRawPacket {
                packet: RawPacket {
                    subscription_id: id,
                    datagram: bytes::Bytes::from_static(&[0x45, 0, 0, 20]),
                    source_ip: Some(Ipv4Addr::new(192, 0, 2, 10).into()),
                    group: Some(Ipv4Addr::new(239, 1, 2, 3).into()),
                    ip_protocol: Some(17),
                    metadata: ReceiveMetadata::empty(),
                },
                matching_subscription_ids: MatchingSubscriptionIds::One([id]),
            }
        }

        let mut context = SharedRawContext::new();
        context
            .pending_packets
            .push_back(queued_packet(SubscriptionId(1)));
        context
            .pending_packets
            .push_back(queued_packet(SubscriptionId(2)));
        let mut packets = Vec::new();

        let received = context.try_recv_batch_into(&mut packets, 1).unwrap();

        assert_eq!(received, 1);
        assert_eq!(packets[0].packet.subscription_id, SubscriptionId(1));
        assert_eq!(context.pending_packet_count(), 1);
    }

    #[cfg(feature = "metrics")]
    #[test]
    fn metrics_snapshot_reports_shared_capture_counters() {
        let mut context = SharedRawContext::new();
        context.record_received_packet();
        context.record_received_packet();
        context.record_unmatched_packet();
        context.record_demultiplex_matches(3);

        let snapshot = context.metrics_snapshot();
        assert_eq!(snapshot.capture_socket_count, 0);
        assert_eq!(snapshot.active_memberships, 0);
        assert_eq!(snapshot.received_packets_total, 2);
        assert_eq!(snapshot.unmatched_packets_total, 1);
        assert_eq!(snapshot.demultiplex_matches_total, 3);
        assert_eq!(snapshot.pending_packets, 0);
    }

    #[test]
    fn source_filter_enum_remains_usable_for_shared_configs() {
        let config = RawSubscriptionConfig {
            group: Ipv6Addr::from(0xff3e_0000_0000_0000_0000_0000_0000_1234u128).into(),
            source: SourceFilter::Source("2001:db8::1".parse().unwrap()),
            interface: Some("fe80::1".parse().unwrap()),
            interface_index: Some(2),
        };
        assert!(config.validate().is_ok());
    }
}
