//! Iroh infrastructure selection for zterm endpoints.

use iroh::{
    Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    address_lookup::{
        AddrFilter, DnsAddressLookup, N0_DNS_ENDPOINT_ORIGIN_PROD, N0_DNS_PKARR_RELAY_PROD,
        PkarrPublisher, PkarrResolver,
    },
    endpoint::{Builder, presets},
};

use crate::config::ValidatedInfrastructure;

/// Product protocol identifier for wire major one.
pub const ZTERM_ALPN: &[u8] = b"zterm/1";

/// Short-lived pairing protocol identifier for format/protocol major one.
pub const ZTERM_PAIR_ALPN: &[u8] = b"zterm-pair/1";

/// Effective configuration of one Relay entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayProfileSummary {
    /// Relay URL advertised and dialed by the endpoint.
    pub url: RelayUrl,
    /// Whether this entry enables QUIC address discovery.
    pub quic_address_discovery: bool,
}

/// Read-only effective infrastructure choices exposed for diagnostics and tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InfrastructureProfileSummary {
    /// All configured Relay entries.
    pub relays: Vec<RelayProfileSummary>,
    /// Production Pkarr URL used by the publisher.
    pub pkarr_publisher_url: RelayUrl,
    /// Production Pkarr URL used by the resolver.
    pub pkarr_resolver_url: RelayUrl,
    /// Production DNS origin used by endpoint lookup.
    pub dns_lookup_origin: String,
    /// Whether direct IP addresses may be published by address lookup services.
    pub publishes_direct_addresses: bool,
    /// Whether Iroh's router port-mapping client is compiled into this profile.
    pub portmapper_enabled: bool,
    /// Accepted protocol identifiers.
    pub alpns: Vec<Vec<u8>>,
}

/// The infrastructure services used to build an Iroh endpoint.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum InfrastructureProfile {
    /// Iroh's pinned official n0 production map.
    #[default]
    OfficialN0,
    /// One explicit self-hosted Relay with QAD disabled.
    SelfHosted {
        /// Relay-only HTTPS URL.
        relay_url: RelayUrl,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProductionAddressLookups {
    pkarr_relay_url: RelayUrl,
    dns_origin: String,
}

impl ProductionAddressLookups {
    fn from_iroh_constants() -> Self {
        let pkarr_relay_url = N0_DNS_PKARR_RELAY_PROD
            .parse()
            .expect("Iroh's production Pkarr constant is a valid URL");

        Self {
            pkarr_relay_url,
            dns_origin: N0_DNS_ENDPOINT_ORIGIN_PROD.to_owned(),
        }
    }

    fn apply(self, builder: Builder) -> Builder {
        builder
            .address_lookup(PkarrPublisher::builder(self.pkarr_relay_url.clone().into()))
            .address_lookup(PkarrResolver::builder(self.pkarr_relay_url.into()))
            .address_lookup(DnsAddressLookup::builder(self.dns_origin))
    }
}

impl InfrastructureProfile {
    /// Creates the official n0 production infrastructure profile.
    ///
    /// The Relay map is intentionally obtained from Iroh's pinned production
    /// default rather than copied into zterm. Address lookup uses Iroh's public
    /// production constants explicitly so ambient staging configuration cannot
    /// change this product profile.
    #[must_use]
    pub const fn zterm() -> Self {
        Self::OfficialN0
    }

    /// Constructs the endpoint profile selected by validated config.
    #[must_use]
    pub fn from_validated(config: &ValidatedInfrastructure) -> Self {
        match config {
            ValidatedInfrastructure::OfficialN0 => Self::OfficialN0,
            ValidatedInfrastructure::SelfHosted(relay_url) => Self::SelfHosted {
                relay_url: relay_url.clone(),
            },
        }
    }

    /// Returns a read-only projection of the effective Relay and lookup configuration.
    #[must_use]
    pub fn summary(&self) -> InfrastructureProfileSummary {
        let lookups = ProductionAddressLookups::from_iroh_constants();
        let relays = self
            .relay_mode()
            .relay_map()
            .relays::<Vec<_>>()
            .into_iter()
            .map(|relay| RelayProfileSummary {
                url: relay.url.clone(),
                quic_address_discovery: relay.quic.is_some(),
            })
            .collect();

        InfrastructureProfileSummary {
            relays,
            pkarr_publisher_url: lookups.pkarr_relay_url.clone(),
            pkarr_resolver_url: lookups.pkarr_relay_url,
            dns_lookup_origin: lookups.dns_origin,
            publishes_direct_addresses: false,
            portmapper_enabled: true,
            alpns: vec![ZTERM_ALPN.to_vec(), ZTERM_PAIR_ALPN.to_vec()],
        }
    }

    /// Builds, but does not bind, an endpoint using this profile.
    ///
    /// The returned builder deliberately remains configurable so a caller can
    /// choose socket bindings. Foundation tests also use that standard Iroh
    /// boundary to add controlled external-address candidates without adding a
    /// test hook to this profile.
    #[must_use]
    pub fn endpoint_builder(&self, secret_key: SecretKey) -> Builder {
        ProductionAddressLookups::from_iroh_constants()
            .apply(Endpoint::builder(presets::Minimal))
            .secret_key(secret_key)
            .relay_mode(self.relay_mode())
            .addr_filter(AddrFilter::relay_only())
            .alpns(vec![ZTERM_ALPN.to_vec(), ZTERM_PAIR_ALPN.to_vec()])
    }

    fn relay_mode(&self) -> RelayMode {
        match self {
            Self::OfficialN0 => RelayMode::Default,
            Self::SelfHosted { relay_url } => {
                RelayMode::Custom(RelayMap::from_iter([RelayConfig::new(
                    relay_url.clone(),
                    None,
                )]))
            }
        }
    }
}
