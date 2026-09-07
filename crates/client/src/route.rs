//! Relay-only signed discovery and authenticated cache/ticket route ordering.
use crate::error::ClientError as DaemonError;
use futures_util::StreamExt;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use std::{collections::BTreeSet, fmt, time::Instant};
use zterm_core::{DeviceId, DomainErrorKind, RelayHint, TransportLimits};

/// Origin of one ordered dial candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteSource {
    /// Signed address-lookup result obtained for this dial.
    FreshLookup,
    /// Previously handshake-verified SQLite route cache.
    VerifiedCache,
    /// Short-lived route carried by a pairing ticket.
    TransientTicket,
}

/// One independently dialable route containing no direct IP address.
#[derive(Clone, Eq, PartialEq)]
pub struct RouteCandidate {
    source: RouteSource,
    relay_hint: RelayHint,
    endpoint_addr: EndpointAddr,
}

impl fmt::Debug for RouteCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouteCandidate")
            .field("source", &self.source)
            .field("relay_hint", &self.relay_hint)
            .field("endpoint_id", &self.endpoint_addr.id)
            .field(
                "relay_address_count",
                &self.endpoint_addr.relay_urls().count(),
            )
            .field(
                "direct_address_count",
                &self.endpoint_addr.ip_addrs().count(),
            )
            .finish()
    }
}

impl RouteCandidate {
    fn relay(
        remote: EndpointId,
        source: RouteSource,
        relay_hint: RelayHint,
    ) -> Result<Self, DaemonError> {
        let relay_url: RelayUrl = relay_hint.as_str().parse().map_err(|_| {
            DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "relay route could not be adapted to Iroh's URL type",
            )
        })?;
        Ok(Self {
            source,
            endpoint_addr: EndpointAddr::new(remote).with_relay_url(relay_url),
            relay_hint,
        })
    }

    /// Candidate origin, in fallback order.
    #[must_use]
    pub const fn source(&self) -> RouteSource {
        self.source
    }

    /// Exact validated relay URL used by this candidate.
    #[must_use]
    pub fn relay_hint(&self) -> &RelayHint {
        &self.relay_hint
    }

    /// Relay-only Iroh endpoint address.
    #[must_use]
    pub fn endpoint_addr(&self) -> &EndpointAddr {
        &self.endpoint_addr
    }
}

/// Reads bounded fresh signed relay hints, before cache or ticket fallback.
pub async fn fresh_relay_hints(
    limits: TransportLimits,
    endpoint: &Endpoint,
    remote: EndpointId,
    deadline: Instant,
) -> Result<Vec<RelayHint>, DaemonError> {
    let services = endpoint.address_lookup().map_err(|_| {
        DaemonError::new(
            DomainErrorKind::TransportUnavailable,
            "endpoint address lookup is unavailable",
        )
    })?;
    let services = services.clone();
    let remaining = deadline.saturating_duration_since(Instant::now());
    let budget = limits.address_lookup_budget.min(remaining);
    if budget.is_zero() {
        return Err(deadline_exceeded("route resolution deadline elapsed"));
    }

    let resolved = tokio::time::timeout(budget, async move {
        let mut stream = Box::pin(services.resolve(remote));
        let mut hints = Vec::new();
        let mut seen = BTreeSet::new();
        while let Some(result) = stream.next().await {
            let Ok(Ok(item)) = result else {
                continue;
            };
            if item.endpoint_id() != remote {
                continue;
            }
            for relay in item.to_endpoint_addr().relay_urls() {
                let text = relay.to_string();
                if seen.insert(text.clone())
                    && let Ok(hint) = RelayHint::new(text)
                {
                    hints.push(hint);
                    if hints.len() == limits.max_relay_hints {
                        return hints;
                    }
                }
            }
        }
        hints
    })
    .await;

    match resolved {
        Ok(hints) => Ok(hints),
        Err(_) => Err(DaemonError::new(
            DomainErrorKind::AddressUnavailable,
            "fresh address lookup timed out",
        )),
    }
}
/// Adapts merged validated hints into independently dialable Iroh candidates.
pub fn build_relay_candidates(
    remote: EndpointId,
    fresh: Vec<RelayHint>,
    cache: Vec<RelayHint>,
    transient: Vec<RelayHint>,
    maximum: usize,
) -> Result<Vec<RouteCandidate>, DaemonError> {
    merge_relay_sources(fresh, cache, transient, maximum)
        .into_iter()
        .map(|(source, hint)| RouteCandidate::relay(remote, source, hint))
        .collect()
}

/// Converts the product's fixed bytes into Iroh's authenticated endpoint ID.
pub fn endpoint_id_from_device(remote: DeviceId) -> Result<EndpointId, DaemonError> {
    EndpointId::from_bytes(remote.as_bytes()).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::IdentityInvalid,
            "device ID is not a valid Iroh endpoint key",
        )
    })
}

/// Converts an Iroh-authenticated endpoint ID into the transport-neutral ID.
#[must_use]
pub fn device_from_endpoint_id(remote: EndpointId) -> DeviceId {
    DeviceId::from_array(*remote.as_bytes())
}

/// Orders and deduplicates fresh, verified-cache and ticket sources independently.
pub fn merge_relay_sources(
    fresh: Vec<RelayHint>,
    cache: Vec<RelayHint>,
    transient: Vec<RelayHint>,
    maximum_per_source: usize,
) -> Vec<(RouteSource, RelayHint)> {
    if maximum_per_source == 0 {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for (source, hints) in [
        (RouteSource::FreshLookup, fresh),
        (RouteSource::VerifiedCache, cache),
        (RouteSource::TransientTicket, transient),
    ] {
        // Each persisted or transient route set is independently bounded by
        // `max_relay_hints`. Do not apply that bound to the merged sequence:
        // a full fresh result must not starve cache/ticket fallback after its
        // candidates fail to connect.
        for hint in hints.into_iter().take(maximum_per_source) {
            if seen.insert(hint.as_str().to_owned()) {
                ordered.push((source, hint));
            }
        }
    }
    ordered
}

fn deadline_exceeded(detail: &'static str) -> DaemonError {
    DaemonError::new(DomainErrorKind::DeadlineExceeded, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn relay(url: &str) -> RelayHint {
        RelayHint::new(url).expect("valid route fixture")
    }
    #[test]
    fn route_candidate_contains_only_the_relay_transport() {
        let remote = iroh::SecretKey::from_bytes(&[7; 32]).public();
        let candidate = RouteCandidate::relay(
            remote,
            RouteSource::VerifiedCache,
            relay("https://relay.example"),
        )
        .expect("candidate adapts");
        assert_eq!(candidate.endpoint_addr().id, remote);
        assert_eq!(candidate.endpoint_addr().relay_urls().count(), 1);
        assert_eq!(candidate.endpoint_addr().ip_addrs().count(), 0);
    }

    #[test]
    fn route_debug_redacts_relay_and_direct_addresses_but_keeps_shape() {
        let relay_sentinel = "https://ROUTE_CANDIDATE_SENTINEL_2a9c.example.test/private";
        let direct_sentinel = "203.0.113.197:49152";
        let remote = iroh::SecretKey::from_bytes(&[0x6e; 32]).public();
        let relay_hint = relay(relay_sentinel);
        let candidate = RouteCandidate {
            source: RouteSource::TransientTicket,
            relay_hint: relay_hint.clone(),
            endpoint_addr: EndpointAddr::new(remote)
                .with_relay_url(relay_sentinel.parse().expect("valid Relay URL"))
                .with_ip_addr(
                    direct_sentinel
                        .parse()
                        .expect("valid direct socket address"),
                ),
        };
        let rendered = format!("{candidate:?}");
        assert!(!rendered.contains(relay_sentinel));
        assert!(!rendered.contains(direct_sentinel));
        assert!(rendered.contains("TransientTicket"));
        assert!(rendered.contains("relay_address_count: 1"));
        assert!(rendered.contains("direct_address_count: 1"));
        assert_eq!(candidate.relay_hint().as_str(), relay_sentinel);
        assert_eq!(candidate.endpoint_addr().id, remote);
        assert_eq!(candidate.endpoint_addr().ip_addrs().count(), 1);
    }
}
