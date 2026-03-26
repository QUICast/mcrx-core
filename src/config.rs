use crate::error::McrxError;
use std::net::Ipv4Addr;

/// Describes whether packets from any source or only one specific source
/// should be accepted for a multicast group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFilter {
    /// Accept packets from any source (Any-Source Multicast, `(*, G)`).
    Any,
    /// Accept packets only from one specific source (Source-Specific Multicast, `(S, G)`).
    Source(Ipv4Addr),
}

/// Configuration for a multicast receive subscription.
///
/// This defines the multicast group, source filtering mode (ASM or SSM),
/// destination port, and optionally the local interface to join on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionConfig {
    /// The destination multicast group to join.
    pub group: Ipv4Addr,
    /// The source filtering mode for the subscription.
    pub source: SourceFilter,
    /// The destination UDP port to receive on.
    pub dst_port: u16,
    /// The local interface address to join on, if explicitly specified.
    pub interface: Option<Ipv4Addr>,
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
            && source.is_multicast()
        {
            return Err(McrxError::InvalidSourceAddress);
        }

        Ok(())
    }

    /// Creates an ASM (`(*, G)`) subscription configuration.
    pub fn asm(group: Ipv4Addr, port: u16) -> Self {
        Self {
            group,
            source: SourceFilter::Any,
            dst_port: port,
            interface: None,
        }
    }

    /// Creates an SSM (`(S, G)`) subscription configuration.
    pub fn ssm(group: Ipv4Addr, source: Ipv4Addr, port: u16) -> Self {
        Self {
            group,
            source: SourceFilter::Source(source),
            dst_port: port,
            interface: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_multicast_config_passes_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3),
            source: SourceFilter::Any,
            dst_port: 5000,
            interface: None,
        };

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn port_zero_fails_validation() {
        let cfg = SubscriptionConfig {
            group: Ipv4Addr::new(239, 1, 2, 3),
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
            group: Ipv4Addr::new(192, 168, 1, 10),
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
            group: Ipv4Addr::new(232, 1, 2, 3),
            source: SourceFilter::Source(Ipv4Addr::new(239, 1, 1, 1)),
            dst_port: 5000,
            interface: None,
        };

        let result = cfg.validate();

        assert!(matches!(result, Err(McrxError::InvalidSourceAddress)));
    }
}
