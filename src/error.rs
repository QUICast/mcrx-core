use std::io;
use thiserror::Error;

/// Errors returned by the multicast receiver core.
#[derive(Debug, Error)]
pub enum McrxError {
    /// The configured destination port is invalid.
    #[error("MCRX: invalid destination port")]
    InvalidDestinationPort,

    /// The configured group address is not a valid multicast IPv4 address.
    #[error("MCRX: group must be a multicast IPv4 address")]
    InvalidMulticastGroup,

    /// The configured SSM source address is invalid.
    #[error("MCRX: invalid source address")]
    InvalidSourceAddress,

    /// A subscription with the same configuration already exists.
    #[error("MCRX: subscription already exists")]
    DuplicateSubscription,

    /// No subscription with the requested ID exists.
    #[error("MCRX: subscription not found")]
    SubscriptionNotFound,

    /// The subscription is not currently joined to its multicast group.
    #[error("MCRX: subscription not joined")]
    SubscriptionNotJoined,

    /// The subscription is already joined to its multicast group.
    #[error("MCRX: subscription already joined")]
    SubscriptionAlreadyJoined,

    /// Creating the UDP socket failed.
    #[error("MCRX: failed to create UDP socket: {0}")]
    SocketCreateFailed(io::Error),

    /// Setting a socket option failed.
    #[error("MCRX: failed to set socket option: {0}")]
    SocketOptionFailed(io::Error),

    /// Binding the UDP socket failed.
    #[error("MCRX: failed to bind UDP socket: {0}")]
    SocketBindFailed(io::Error),

    /// Reading the local address of an existing socket failed.
    #[error("MCRX: failed to read local address from existing socket: {0}")]
    SocketLocalAddrFailed(io::Error),

    /// The provided existing socket does not match the current IPv4-only receive model.
    #[error("MCRX: existing socket must be an IPv4 UDP socket")]
    ExistingSocketMustBeIpv4,

    /// The provided existing socket is bound to a different UDP port than the subscription.
    #[error("MCRX: existing socket is bound to UDP port {actual}, expected {expected}")]
    ExistingSocketPortMismatch { expected: u16, actual: u16 },

    /// Joining an IPv4 multicast group failed.
    #[error("MCRX: failed to join IPv4 multicast group: {0}")]
    MulticastJoinFailed(io::Error),

    /// Leaving an IPv4 multicast group failed.
    #[error("MCRX: failed to leave multicast group: {0}")]
    MulticastLeaveFailed(io::Error),

    /// Source-specific multicast is not supported on this platform.
    #[error("MCRX: source-specific multicast is not supported on this platform")]
    SourceSpecificMulticastUnsupported,

    /// Binding the interface probe socket failed.
    #[error("MCRX: failed to bind interface probe socket: {0}")]
    InterfaceProbeBindFailed(io::Error),

    /// Connecting the interface probe socket failed.
    #[error("MCRX: failed to connect interface probe socket: {0}")]
    InterfaceProbeConnectFailed(io::Error),

    /// Reading the local address from the interface probe socket failed.
    #[error("MCRX: failed to read local address from interface probe socket: {0}")]
    InterfaceProbeLocalAddrFailed(io::Error),

    /// Discovering the local network interface failed.
    #[error("MCRX: failed to discover local interface: {0}")]
    InterfaceDiscoveryFailed(String),

    /// The received packet came from a non-IP socket address.
    #[error("MCRX: received packet from non-IP socket address")]
    NonIpSocketAddress,

    /// Receiving a packet failed.
    #[error("MCRX: receive failed: {0}")]
    ReceiveFailed(io::Error),
}
