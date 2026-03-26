use crate::config::{SourceFilter, SubscriptionConfig};
use crate::error::McrxError;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4};

fn resolve_interface(config: &SubscriptionConfig) -> Ipv4Addr {
    config.interface.unwrap_or(Ipv4Addr::UNSPECIFIED)
}

/// Opens and binds a UDP socket for the given subscription configuration.
///
/// The socket is bound to `0.0.0.0:dst_port` so it can receive multicast traffic
/// destined for the configured UDP port. The socket is configured as non-blocking.
pub(crate) fn open_bound_socket(config: &SubscriptionConfig) -> Result<Socket, McrxError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .map_err(McrxError::SocketCreateFailed)?;

    socket
        .set_reuse_address(true)
        .map_err(McrxError::SocketOptionFailed)?;

    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, config.dst_port);

    socket
        .bind(&SockAddr::from(bind_addr))
        .map_err(McrxError::SocketBindFailed)?;

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    Ok(socket)
}

/// Joins the configured multicast group for this subscription on an already bound socket.
///
/// Uses ASM (`(*, G)`) or SSM (`(S, G)`) depending on the configured source filter.
pub(crate) fn join_multicast_group(
    socket: &Socket,
    config: &SubscriptionConfig,
) -> Result<(), McrxError> {
    let interface = resolve_interface(config);

    match config.source {
        SourceFilter::Any => socket
            .join_multicast_v4(&config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),

        SourceFilter::Source(source) => socket
            .join_ssm_v4(&source, &config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),
    }
}

/// Leaves the configured multicast group for this subscription on the given socket.
///
/// Uses ASM (`(*, G)`) or SSM (`(S, G)`) depending on the configured source filter.
pub(crate) fn leave_multicast_group(
    socket: &Socket,
    config: &SubscriptionConfig,
) -> Result<(), McrxError> {
    let interface = resolve_interface(config);

    match config.source {
        SourceFilter::Any => socket
            .leave_multicast_v4(&config.group, &interface)
            .map_err(McrxError::MulticastLeaveFailed),

        SourceFilter::Source(source) => socket
            .leave_ssm_v4(&source, &config.group, &interface)
            .map_err(McrxError::MulticastLeaveFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SourceFilter, SubscriptionConfig};
    use std::net::Ipv4Addr;

    #[test]
    fn open_and_join_socket_succeeds_for_valid_asm_config() {
        let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 55000);

        let socket = open_bound_socket(&config);
        assert!(socket.is_ok());

        let socket = socket.unwrap();
        let result = join_multicast_group(&socket, &config);
        assert!(result.is_ok());
    }

    #[test]
    fn open_and_join_socket_succeeds_for_valid_ssm_config() {
        let config = SubscriptionConfig::ssm(
            Ipv4Addr::new(232, 1, 2, 3),
            Ipv4Addr::new(192, 168, 188, 50),
            55009,
        );

        let socket = open_bound_socket(&config);
        assert!(socket.is_ok());

        let socket = socket.unwrap();
        let result = join_multicast_group(&socket, &config);
        assert!(result.is_ok());
    }
}
