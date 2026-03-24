use crate::config::{SourceFilter, SubscriptionConfig};
use crate::error::McrxError;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::net::{Ipv4Addr, SocketAddrV4};

/// Opens, binds, and joins a UDP multicast socket for the given subscription.
///
/// This currently supports IPv4 ASM (`(*, G)`) subscriptions.
/// SSM (`(S, G)`) is not yet implemented.
pub(crate) fn open_and_join_socket(config: &SubscriptionConfig) -> Result<Socket, McrxError> {
    let socket = open_bound_socket(config)?;

    join_multicast_group(&socket, config)?;

    Ok(socket)
}

/// Opens and binds a UDP socket for the given subscription configuration.
///
/// The socket is bound to `0.0.0.0:dst_port` so it can receive multicast traffic
/// destined for the configured UDP port.
fn open_bound_socket(config: &SubscriptionConfig) -> Result<Socket, McrxError> {
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

/// Joins the configured multicast group on the given socket.
fn join_multicast_group(socket: &Socket, config: &SubscriptionConfig) -> Result<(), McrxError> {
    let interface = config.interface.unwrap_or(Ipv4Addr::UNSPECIFIED);

    match config.source {
        SourceFilter::Any => socket
            .join_multicast_v4(&config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),

        SourceFilter::Source(source) => socket
            .join_ssm_v4(&source, &config.group, &interface)
            .map_err(McrxError::MulticastJoinFailed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SourceFilter, SubscriptionConfig};
    use std::net::Ipv4Addr;

    #[test]
    fn open_and_join_socket_succeeds_for_valid_asm_config() {
        let config = SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3),
            source: SourceFilter::Any,
            dst_port: 55000,
            interface: None,
        };

        let socket = open_and_join_socket(&config);

        assert!(socket.is_ok());
    }

    #[test]
    fn open_and_join_socket_succeeds_for_valid_ssm_config() {
        let config = SubscriptionConfig {
            group: Ipv4Addr::new(232, 1, 2, 3),
            source: SourceFilter::Source(Ipv4Addr::new(192, 168, 188, 50)),
            dst_port: 55001,
            interface: None,
        };

        let socket = open_and_join_socket(&config);

        assert!(socket.is_ok());
    }
}
