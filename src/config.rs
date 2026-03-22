use crate::error::McrxError;
use std::net::Ipv4Addr;

/// Describes whether packets from any source or only one specific source
/// should be accepted for a multicast group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFilter {
    /// Accept packets from any source. This corresponds to ASM `(*, G)`.
    Any,
    /// Accept packets only from one specific source. This corresponds to SSM `(S, G)`.
    Source(Ipv4Addr),
}

/// Configuration used to create a multicast receive subscription.
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

        Ok(())
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
}
