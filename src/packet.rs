use crate::subscription::SubscriptionId;
use bytes::Bytes;
use std::net::{IpAddr, SocketAddr};

/// A received packet together with the metadata needed by the receiver core.
#[derive(Debug, Clone)]
pub struct Packet {
    /// The subscription through which this packet was received.
    pub subscription_id: SubscriptionId,
    /// The remote sender's source address and source port.
    pub source: SocketAddr,
    /// The destination multicast group address.
    pub group: IpAddr,
    /// The destination UDP port on which the packet was received.
    pub dst_port: u16,
    /// The raw UDP payload bytes.
    pub payload: Bytes,
}

impl Packet {
    /// Returns the length of the payload in bytes.
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};

    #[test]
    fn packet_payload_len_returns_correct_length() {
        let packet = Packet {
            subscription_id: SubscriptionId(1),
            source: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 12345)),
            group: IpAddr::V4(Ipv4Addr::new(239, 1, 2, 3)),
            dst_port: 5000,
            payload: Bytes::from_static(&[1, 2, 3]),
        };

        assert_eq!(packet.payload_len(), 3);
    }

    #[test]
    fn subscription_id_equality_works() {
        let a = SubscriptionId(7);
        let b = SubscriptionId(7);
        let c = SubscriptionId(8);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
