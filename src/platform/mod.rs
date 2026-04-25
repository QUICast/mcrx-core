use crate::config::{SourceFilter, SubscriptionConfig};
use crate::error::McrxError;
use crate::packet::Packet;
use crate::subscription::SubscriptionId;
use bytes::Bytes;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

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

    prepare_existing_socket(socket, config)
}

/// Validates and prepares a caller-provided socket for use with the subscription.
///
/// The socket must already be bound to the destination port from `config`.
/// This helper preserves caller-controlled bind/socket setup while still enforcing
/// the non-blocking contract used by the receive APIs.
pub(crate) fn prepare_existing_socket(
    socket: Socket,
    config: &SubscriptionConfig,
) -> Result<Socket, McrxError> {
    let local_addr = socket_local_addr(&socket)?;

    match local_addr {
        SocketAddr::V4(addr) => {
            if addr.port() != config.dst_port {
                return Err(McrxError::ExistingSocketPortMismatch {
                    expected: config.dst_port,
                    actual: addr.port(),
                });
            }
        }
        SocketAddr::V6(_) => {
            return Err(McrxError::ExistingSocketMustBeIpv4);
        }
    }

    socket
        .set_nonblocking(true)
        .map_err(McrxError::SocketOptionFailed)?;

    Ok(socket)
}

/// Returns the local IP socket address for the given socket.
pub(crate) fn socket_local_addr(socket: &Socket) -> Result<SocketAddr, McrxError> {
    socket
        .local_addr()
        .map_err(McrxError::SocketLocalAddrFailed)?
        .as_socket()
        .ok_or(McrxError::NonIpSocketAddress)
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

/// Attempts to receive a packet from the given socket without blocking.
pub(crate) fn recv_packet(
    socket: &Socket,
    subscription_id: SubscriptionId,
    config: &SubscriptionConfig,
) -> Result<Option<Packet>, McrxError> {
    let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 65535];

    match socket.recv_from(&mut buf) {
        Ok((len, addr)) => {
            let source = addr.as_socket().ok_or(McrxError::NonIpSocketAddress)?;

            // SAFETY: `recv_from` initialized exactly the first `len` bytes of `buf`.
            // We only create a slice over that initialized prefix, and copy it immediately.
            let payload_bytes =
                unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

            Ok(Some(Packet {
                subscription_id,
                source,
                group: IpAddr::V4(config.group),
                dst_port: config.dst_port,
                payload: Bytes::copy_from_slice(payload_bytes),
            }))
        }
        Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(McrxError::ReceiveFailed(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SubscriptionConfig;
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

    #[test]
    fn prepare_existing_socket_rejects_wrong_port() {
        let config = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 55010);

        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).unwrap();
        socket.set_reuse_address(true).unwrap();
        socket
            .bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
            .unwrap();

        let result = prepare_existing_socket(socket, &config);

        assert!(matches!(
            result,
            Err(McrxError::ExistingSocketPortMismatch {
                expected: 55010,
                ..
            })
        ));
    }
}
