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

    /// A subscription with the same configuration already exists.
    #[error("MCRX: subscription already exists")]
    DuplicateSubscription,

    /// Creating the UDP socket failed.
    #[error("MCRX: failed to create UDP socket: {0}")]
    SocketCreateFailed(io::Error),

    /// Setting a socket option failed.
    #[error("MCRX: failed to set socket option: {0}")]
    SocketOptionFailed(io::Error),

    /// Binding the UDP socket failed.
    #[error("MCRX: failed to bind UDP socket: {0}")]
    SocketBindFailed(io::Error),

    /// Joining an IPv4 multicast group failed.
    #[error("MCRX: failed to join IPv4 multicast group: {0}")]
    MulticastJoinFailed(io::Error),

    #[error("MCRX: source-specific multicast is not supported on this platform yet")]
    SourceSpecificMulticastUnsupported,

    #[error("MCRX: failed to bind interface probe socket: {0}")]
    InterfaceProbeBindFailed(io::Error),

    #[error("MCRX: failed to connect interface probe socket: {0}")]
    InterfaceProbeConnectFailed(io::Error),

    #[error("MCRX: failed to read local address from interface probe socket: {0}")]
    InterfaceProbeLocalAddrFailed(io::Error),

    #[error("MCRX: Received packet from non IP Socket Address")]
    NonIpSocketAddress,

    #[error("MCRX: failed to discover local interface: {0}")]
    InterfaceDiscoveryFailed(String),

    #[error("MCRX: receive failed: {0}")]
    ReceiveFailed(std::io::Error),
}
