use crate::config::SubscriptionConfig;
use crate::error::McrxError;
use crate::packet::Packet;
use bytes::Bytes;
use socket2::Socket;
use std::io::ErrorKind;

/// Identifies a subscription within a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub u64);

/// Represents a registered subscription stored inside a context.
#[derive(Debug)]
pub struct Subscription {
    id: SubscriptionId,
    config: SubscriptionConfig,
    socket: Socket,
}

impl Subscription {
    /// Creates a new subscription from an ID and configuration.
    pub fn new(id: SubscriptionId, config: SubscriptionConfig, socket: Socket) -> Self {
        Self { id, config, socket }
    }

    /// Returns the subscription's ID.
    pub fn id(&self) -> SubscriptionId {
        self.id
    }

    /// Returns a read-only reference to the subscription's configuration.
    pub fn config(&self) -> &SubscriptionConfig {
        &self.config
    }

    /// Returns a read-only reference to the subscription's socket.
    pub fn socket(&self) -> &Socket {
        &self.socket
    }

    /// Uses a `MaybeUninit<u8>` buffer because `socket2::Socket::recv_from` may
    /// write into uninitialized memory.
    ///
    /// After `recv_from` returns `len`, only the first `len` bytes are assumed to
    /// be initialized by the OS. Those bytes are then reinterpreted as `&[u8]`
    /// for copying into the packet payload.
    ///
    /// Attempts to receive a single packet without blocking.
    ///
    /// Returns:
    /// - `Ok(Some(packet))` if a packet was received,
    /// - `Ok(None)` if no packet is currently available,
    /// - `Err(...)` on an actual receive failure.
    pub fn try_recv(&self) -> Result<Option<Packet>, McrxError> {
        let mut buf = [std::mem::MaybeUninit::<u8>::uninit(); 65535];

        match self.socket.recv_from(&mut buf) {
            Ok((len, addr)) => {
                let source = addr.as_socket().ok_or(McrxError::NonIpSocketAddress)?;

                // SAFETY: `recv_from` initialized exactly the first `len` bytes of `buf`.
                // We only create a slice over that initialized prefix, and copy it immediately.
                let payload_bytes =
                    unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) };

                let packet = Packet {
                    subscription_id: self.id,
                    source,
                    group: std::net::IpAddr::V4(self.config.group),
                    dst_port: self.config.dst_port,
                    payload: Bytes::copy_from_slice(payload_bytes),
                };

                Ok(Some(packet))
            }

            Err(err) if err.kind() == ErrorKind::WouldBlock => Ok(None),

            Err(err) => Err(McrxError::ReceiveFailed(err)),
        }
    }

    /// Returns the raw Unix file descriptor of the underlying socket.
    ///
    /// This is useful for integrating subscriptions into external event loops.
    #[cfg(unix)]
    pub fn as_raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.socket.as_raw_fd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SourceFilter, SubscriptionConfig};
    use crate::platform;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
    use std::thread;
    use std::time::{Duration, Instant};

    fn test_config(port: u16) -> SubscriptionConfig {
        SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3),
            source: SourceFilter::Any,
            dst_port: port,
            interface: None,
        }
    }

    fn test_ssm_config(port: u16, interface: Ipv4Addr) -> SubscriptionConfig {
        SubscriptionConfig {
            group: Ipv4Addr::new(232, 1, 2, 3),
            source: SourceFilter::Source(interface),
            dst_port: port,
            interface: Some(interface),
        }
    }

    fn primary_ipv4() -> Ipv4Addr {
        let probe = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        probe
            .connect(SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), 9))
            .unwrap();

        match probe.local_addr().unwrap() {
            SocketAddr::V4(addr) => *addr.ip(),
            SocketAddr::V6(_) => panic!("expected an IPv4 local address for SSM test"),
        }
    }

    #[test]
    fn try_recv_returns_none_when_no_packet_is_available() {
        let config = test_config(55020);
        let socket = platform::open_and_join_socket(&config).unwrap();
        let subscription = Subscription::new(SubscriptionId(1), config, socket);

        let result = subscription.try_recv().unwrap();

        assert!(result.is_none());
    }

    #[test]
    fn try_recv_receives_packet_sent_to_bound_port() {
        let config = test_config(55021);
        let socket = platform::open_and_join_socket(&config).unwrap();
        let subscription = Subscription::new(SubscriptionId(1), config.clone(), socket);

        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let payload = b"hello multicast core";

        sender
            .send_to(
                payload,
                SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.dst_port),
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match subscription.try_recv().unwrap() {
                Some(packet) => {
                    assert_eq!(packet.subscription_id, SubscriptionId(1));
                    assert_eq!(packet.group, std::net::IpAddr::V4(config.group));
                    assert_eq!(packet.dst_port, config.dst_port);
                    assert_eq!(&packet.payload[..], payload);
                    assert_eq!(
                        packet.source.ip(),
                        std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
                    );
                    break;
                }
                None if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("timed out waiting for packet"),
            }
        }
    }

    #[test]
    fn try_recv_receives_multicast_packet_from_joined_group() {
        let config = test_config(55022);
        let socket = platform::open_and_join_socket(&config).unwrap();
        let subscription = Subscription::new(SubscriptionId(1), config.clone(), socket);

        let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        sender.set_multicast_loop_v4(true).unwrap();
        sender.set_multicast_ttl_v4(1).unwrap();

        let sender_port = sender.local_addr().unwrap().port();
        let payload = b"hello real asm multicast";

        sender
            .send_to(payload, SocketAddrV4::new(config.group, config.dst_port))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match subscription.try_recv().unwrap() {
                Some(packet) => {
                    assert_eq!(packet.subscription_id, SubscriptionId(1));
                    assert_eq!(packet.group, std::net::IpAddr::V4(config.group));
                    assert_eq!(packet.dst_port, config.dst_port);
                    assert_eq!(&packet.payload[..], payload);
                    assert_eq!(packet.source.port(), sender_port);
                    break;
                }
                None if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("timed out waiting for multicast packet"),
            }
        }
    }

    #[test]
    fn try_recv_receives_ssm_packet_from_allowed_source() {
        let interface = primary_ipv4();
        let config = test_ssm_config(55023, interface);
        let socket = platform::open_and_join_socket(&config).unwrap();
        let subscription = Subscription::new(SubscriptionId(1), config.clone(), socket);

        let sender = UdpSocket::bind(SocketAddrV4::new(interface, 0)).unwrap();
        sender.set_multicast_loop_v4(true).unwrap();
        sender.set_multicast_ttl_v4(1).unwrap();

        let sender_port = sender.local_addr().unwrap().port();
        let payload = b"hello real ssm multicast";

        sender
            .send_to(payload, SocketAddrV4::new(config.group, config.dst_port))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match subscription.try_recv().unwrap() {
                Some(packet) => {
                    assert_eq!(packet.subscription_id, SubscriptionId(1));
                    assert_eq!(packet.group, IpAddr::V4(config.group));
                    assert_eq!(packet.dst_port, config.dst_port);
                    assert_eq!(&packet.payload[..], payload);
                    assert_eq!(packet.source.port(), sender_port);
                    assert_eq!(packet.source.ip(), IpAddr::V4(interface));
                    break;
                }
                None if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                None => panic!("timed out waiting for SSM packet"),
            }
        }
    }
}
