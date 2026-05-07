#![cfg(test)]

use crate::{Context, Packet, SourceFilter, SubscriptionConfig};
use socket2::SockRef;
use std::net::IpAddr;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

/// Creates a standard ASM test subscription configuration on the given port.
pub(crate) fn sample_config(port: u16) -> SubscriptionConfig {
    SubscriptionConfig {
        group: IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3)),
        source: SourceFilter::Any,
        dst_port: port,
        interface: None,
    }
}

/// Creates a standard IPv6 ASM test subscription configuration on the given port.
pub(crate) fn sample_config_v6(port: u16) -> SubscriptionConfig {
    let mut config = SubscriptionConfig::asm_v6("ff01::1234".parse().unwrap(), port);
    config.interface = Some(IpAddr::V6(Ipv6Addr::LOCALHOST));
    config
}

/// Receives the next packet from the context before the given deadline.
pub(crate) fn recv_next_packet(context: &mut Context, deadline: Instant) -> Packet {
    loop {
        match context.try_recv_any().unwrap() {
            Some(packet) => return packet,
            None if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            None => panic!("timed out waiting for packet from context"),
        }
    }
}

/// Creates a multicast-capable UDP sender socket for tests.
pub(crate) fn make_multicast_sender() -> UdpSocket {
    let sender = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).unwrap();
    sender.set_multicast_loop_v4(true).unwrap();
    sender.set_multicast_ttl_v4(1).unwrap();
    sender
}

/// Creates an IPv6 multicast-capable UDP sender socket for tests.
pub(crate) fn make_multicast_sender_v6(interface: Ipv6Addr) -> UdpSocket {
    let sender = UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0)).unwrap();
    sender.set_multicast_loop_v6(true).unwrap();

    let socket = SockRef::from(&sender);
    socket.set_multicast_hops_v6(1).unwrap();

    let ifindex = crate::platform::resolve_ipv6_interface_index(interface).unwrap();
    socket.set_multicast_if_v6(ifindex).unwrap();

    sender
}

pub(crate) fn ipv6_group(config: &SubscriptionConfig) -> Ipv6Addr {
    config.ipv6_membership().unwrap().group
}

pub(crate) fn ipv6_group_socket_addr(config: &SubscriptionConfig) -> SocketAddrV6 {
    let interface = match config.interface {
        Some(IpAddr::V6(interface)) => interface,
        _ => Ipv6Addr::LOCALHOST,
    };
    let ifindex = crate::platform::resolve_ipv6_interface_index(interface).unwrap();
    SocketAddrV6::new(ipv6_group(config), config.dst_port, 0, ifindex)
}
