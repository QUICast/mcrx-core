#![cfg(test)]

use crate::{Context, Packet, SourceFilter, SubscriptionConfig};
use std::net::IpAddr;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
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
