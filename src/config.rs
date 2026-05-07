use crate::error::McrxError;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Identifies the IP address family used by a subscription configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionAddressFamily {
    /// IPv4 multicast traffic.
    Ipv4,
    /// IPv6 multicast traffic.
    Ipv6,
}

/// Describes whether packets from any source or only one specific source
/// should be accepted for a multicast group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFilter {
    /// Accept packets from any source (Any-Source Multicast, `(*, G)`).
    Any,
    /// Accept packets only from one specific source (Source-Specific Multicast, `(S, G)`).
    Source(IpAddr),
}

/// Configuration for a multicast receive subscription.
///
/// This defines the multicast group, source filtering mode (ASM or SSM),
/// destination port, and optionally the local interface to join on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionConfig {
    /// The destination multicast group to join.
    pub group: IpAddr,
    /// The source filtering mode for the subscription.
    pub source: SourceFilter,
    /// The destination UDP port to receive on.
    pub dst_port: u16,
    /// The local interface address to join on, if explicitly specified.
    pub interface: Option<IpAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ipv4Membership {
    pub(crate) group: Ipv4Addr,
    pub(crate) source: Option<Ipv4Addr>,
    pub(crate) interface: Option<Ipv4Addr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ipv6Membership {
    pub(crate) group: Ipv6Addr,
    pub(crate) source: Option<Ipv6Addr>,
    pub(crate) interface: Option<Ipv6Addr>,
}

impl SubscriptionConfig {
    /// Validates the configuration and returns an error if it is not usable.
    pub fn validate(&self) -> Result<(), McrxError> {
        if self.dst_port == 0 {
            return Err(McrxError::InvalidDestinationPort);
        }

        if !self.group.is_multicast() {
            return Err(McrxError::InvalidMulticastGroup);
        }

        if let SourceFilter::Source(source) = self.source
        {
            if source.is_multicast() {
                return Err(McrxError::InvalidSourceAddress);
            }

            if !same_family(self.group, source) {
                return Err(McrxError::SourceAddressFamilyMismatch);
            }
        }

        if let Some(interface) = self.interface
            && !same_family(self.group, interface)
        {
            return Err(McrxError::InterfaceAddressFamilyMismatch);
        }

        Ok(())
    }

    /// Returns the configured address family.
    pub fn family(&self) -> SubscriptionAddressFamily {
        match self.group {
            IpAddr::V4(_) => SubscriptionAddressFamily::Ipv4,
            IpAddr::V6(_) => SubscriptionAddressFamily::Ipv6,
        }
    }

    /// Returns `true` when this is an IPv4 subscription.
    pub fn is_ipv4(&self) -> bool {
        matches!(self.family(), SubscriptionAddressFamily::Ipv4)
    }

    /// Returns `true` when this is an IPv6 subscription.
    pub fn is_ipv6(&self) -> bool {
        matches!(self.family(), SubscriptionAddressFamily::Ipv6)
    }

    /// Returns the configured source address, if any.
    pub fn source_addr(&self) -> Option<IpAddr> {
        match self.source {
            SourceFilter::Any => None,
            SourceFilter::Source(source) => Some(source),
        }
    }

    /// Creates an ASM (`(*, G)`) subscription configuration.
    pub fn asm(group: Ipv4Addr, port: u16) -> Self {
        Self::asm_ip(group.into(), port)
    }

    /// Creates an IPv6 ASM (`(*, G)`) subscription configuration.
    pub fn asm_v6(group: Ipv6Addr, port: u16) -> Self {
        Self::asm_ip(group.into(), port)
    }

    /// Creates an ASM (`(*, G)`) subscription configuration from any IP family.
    pub fn asm_ip(group: IpAddr, port: u16) -> Self {
        Self {
            group,
            source: SourceFilter::Any,
            dst_port: port,
            interface: None,
        }
    }

    /// Creates an SSM (`(S, G)`) subscription configuration.
    pub fn ssm(group: Ipv4Addr, source: Ipv4Addr, port: u16) -> Self {
        Self::ssm_ip(group.into(), source.into(), port)
    }

    /// Creates an IPv6 SSM (`(S, G)`) subscription configuration.
    pub fn ssm_v6(group: Ipv6Addr, source: Ipv6Addr, port: u16) -> Self {
        Self::ssm_ip(group.into(), source.into(), port)
    }

    /// Creates an SSM (`(S, G)`) subscription configuration from any IP family.
    pub fn ssm_ip(group: IpAddr, source: IpAddr, port: u16) -> Self {
        Self {
            group,
            source: SourceFilter::Source(source),
            dst_port: port,
            interface: None,
        }
    }

    pub(crate) fn ipv4_membership(&self) -> Option<Ipv4Membership> {
        let group = match self.group {
            IpAddr::V4(group) => group,
            IpAddr::V6(_) => return None,
        };

        let source = match self.source {
            SourceFilter::Any => None,
            SourceFilter::Source(IpAddr::V4(source)) => Some(source),
            SourceFilter::Source(IpAddr::V6(_)) => return None,
        };

        let interface = match self.interface {
            None => None,
            Some(IpAddr::V4(interface)) => Some(interface),
            Some(IpAddr::V6(_)) => return None,
        };

        Some(Ipv4Membership {
            group,
            source,
            interface,
        })
    }

    pub(crate) fn ipv6_membership(&self) -> Option<Ipv6Membership> {
        let group = match self.group {
            IpAddr::V6(group) => group,
            IpAddr::V4(_) => return None,
        };

        let source = match self.source {
            SourceFilter::Any => None,
            SourceFilter::Source(IpAddr::V6(source)) => Some(source),
            SourceFilter::Source(IpAddr::V4(_)) => return None,
        };

        let interface = match self.interface {
            None => None,
            Some(IpAddr::V6(interface)) => Some(interface),
            Some(IpAddr::V4(_)) => return None,
        };

        Some(Ipv6Membership {
            group,
            source,
            interface,
        })
    }
}

fn same_family(left: IpAddr, right: IpAddr) -> bool {
    matches!(
        (left, right),
        (IpAddr::V4(_), IpAddr::V4(_)) | (IpAddr::V6(_), IpAddr::V6(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_multicast_config_passes_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3).into(),
            source: SourceFilter::Any,
            dst_port: 5000,
            interface: None,
        };

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn port_zero_fails_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3).into(),
            source: SourceFilter::Any,
            dst_port: 0,
            interface: None,
        };

        let result = cfg.validate();

        assert!(matches!(result, Err(McrxError::InvalidDestinationPort)));
    }

    #[test]
    fn non_multicast_group_fails_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(192, 168, 1, 10).into(),
            source: SourceFilter::Any,
            dst_port: 5000,
            interface: None,
        };

        let result = cfg.validate();

        assert!(matches!(result, Err(McrxError::InvalidMulticastGroup)));
    }

    #[test]
    fn multicast_source_fails_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(232, 1, 2, 3).into(),
            source: SourceFilter::Source(Ipv4Addr::new(239, 1, 1, 1).into()),
            dst_port: 5000,
            interface: None,
        };

        let result = cfg.validate();

        assert!(matches!(result, Err(McrxError::InvalidSourceAddress)));
    }

    #[test]
    fn ipv6_asm_config_passes_validation() {
        let cfg = SubscriptionConfig::asm_v6("ff3e::1234".parse().unwrap(), 5000);

        assert!(cfg.validate().is_ok());
        assert!(cfg.is_ipv6());
    }

    #[test]
    fn ipv6_ssm_config_passes_validation() {
        let cfg = SubscriptionConfig::ssm_v6(
            "ff3e::1234".parse().unwrap(),
            "2001:db8::10".parse().unwrap(),
            5000,
        );

        assert!(cfg.validate().is_ok());
        assert_eq!(
            cfg.source_addr(),
            Some("2001:db8::10".parse::<IpAddr>().unwrap())
        );
    }

    #[test]
    fn source_family_mismatch_fails_validation() {
        let cfg = SubscriptionConfig::ssm_ip(
            Ipv4Addr::new(232, 1, 2, 3).into(),
            "2001:db8::10".parse().unwrap(),
            5000,
        );

        let result = cfg.validate();

        assert!(matches!(
            result,
            Err(McrxError::SourceAddressFamilyMismatch)
        ));
    }

    #[test]
    fn interface_family_mismatch_fails_validation() {
        let mut cfg = SubscriptionConfig::asm(Ipv4Addr::new(239, 1, 2, 3), 5000);
        cfg.interface = Some("2001:db8::20".parse().unwrap());

        let result = cfg.validate();

        assert!(matches!(
            result,
            Err(McrxError::InterfaceAddressFamilyMismatch)
        ));
    }
}
