//! Relay-only address resolution without mutating the configured Iroh profile.

use std::collections::BTreeSet;
use std::fmt;
use std::time::Instant;

use futures_util::StreamExt;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl};
use zterm_core::{DeviceId, DomainErrorKind, RelayHint, TransportLimits};

use crate::error::DaemonError;
use crate::store::{KnownDevice, RouteCacheDiagnostic, StoreHandle};

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

/// Resolves ordered fresh/cache/ticket relay candidates for the broker.
#[derive(Clone)]
pub struct RouteResolver {
    store: StoreHandle,
    limits: TransportLimits,
}

#[derive(Eq, PartialEq)]
struct CachedRouteFallback {
    relay_hints: Vec<RelayHint>,
    diagnostic: Option<RouteCacheDiagnostic>,
}

impl fmt::Debug for CachedRouteFallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CachedRouteFallback")
            .field("relay_hint_count", &self.relay_hints.len())
            .field("diagnostic", &self.diagnostic)
            .finish()
    }
}

impl RouteResolver {
    /// Creates a resolver using the sole daemon store actor.
    pub fn new(store: StoreHandle, limits: TransportLimits) -> Result<Self, DaemonError> {
        limits.validate().map_err(|error| {
            DaemonError::new(DomainErrorKind::ResourceExhausted, error.to_string())
        })?;
        Ok(Self { store, limits })
    }

    /// Resolves relay-only candidates without inserting routes into the endpoint.
    ///
    /// Fresh signed lookup runs for at most the configured two-second budget.
    /// Its errors are intentionally non-terminal: an existing verified cache or
    /// a caller-owned transient ticket route may still make the peer reachable.
    pub async fn candidates(
        &self,
        endpoint: &Endpoint,
        remote: DeviceId,
        transient_ticket_routes: &[RelayHint],
        deadline: Instant,
    ) -> Result<Vec<RouteCandidate>, DaemonError> {
        if Instant::now() >= deadline {
            return Err(deadline_exceeded("route resolution deadline elapsed"));
        }
        let endpoint_id = endpoint_id_from_device(remote)?;
        let fresh = self
            .fresh_relay_hints(endpoint, endpoint_id, deadline)
            .await;

        let known = if Instant::now() < deadline {
            self.store
                .run_blocking_until(deadline, move |store, deadline| {
                    store.known_device(remote, deadline)
                })
                .await?
        } else {
            None
        };
        let CachedRouteFallback {
            relay_hints: cache,
            diagnostic: _cache_diagnostic,
        } = cached_route_fallback(known);

        let candidates = build_relay_candidates(
            endpoint_id,
            fresh.unwrap_or_default(),
            cache,
            transient_ticket_routes.to_vec(),
            self.limits.max_relay_hints,
        )?;
        if candidates.is_empty() {
            Err(DaemonError::new(
                DomainErrorKind::AddressUnavailable,
                "no relay route is available for the target device",
            ))
        } else {
            Ok(candidates)
        }
    }

    async fn fresh_relay_hints(
        &self,
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
        let budget = self.limits.address_lookup_budget.min(remaining);
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
                        if hints.len() == self.limits.max_relay_hints {
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
}

fn cached_route_fallback(known: Option<KnownDevice>) -> CachedRouteFallback {
    known.map_or(
        CachedRouteFallback {
            relay_hints: Vec::new(),
            diagnostic: None,
        },
        |known| CachedRouteFallback {
            relay_hints: known
                .route_cache
                .map_or_else(Vec::new, |cache| cache.relay_hints),
            diagnostic: known.route_cache_diagnostic,
        },
    )
}

fn build_relay_candidates(
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

/// Plans the exact fresh/cache/transient fallback sequence for socket-free
/// named integration gates. Production resolution reaches the same helper only
/// after its bounded signed lookup and StoreActor cache read.
#[doc(hidden)]
pub fn plan_relay_candidates_for_test(
    remote: DeviceId,
    fresh: Vec<RelayHint>,
    cache: Vec<RelayHint>,
    transient: Vec<RelayHint>,
    maximum: usize,
) -> Result<Vec<RouteCandidate>, DaemonError> {
    build_relay_candidates(
        endpoint_id_from_device(remote)?,
        fresh,
        cache,
        transient,
        maximum,
    )
}

/// Converts the product's fixed bytes into Iroh's authenticated endpoint ID.
pub(crate) fn endpoint_id_from_device(remote: DeviceId) -> Result<EndpointId, DaemonError> {
    EndpointId::from_bytes(remote.as_bytes()).map_err(|_| {
        DaemonError::new(
            DomainErrorKind::IdentityInvalid,
            "device ID is not a valid Iroh endpoint key",
        )
    })
}

/// Converts an Iroh-authenticated endpoint ID into the transport-neutral ID.
#[must_use]
pub(crate) fn device_from_endpoint_id(remote: EndpointId) -> DeviceId {
    DeviceId::from_array(*remote.as_bytes())
}

fn merge_relay_sources(
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
    use crate::store::RelayRouteCache;
    use crate::transport::InfrastructureProfile;
    use zterm_core::{DeviceAlias, DeviceDisplayName};

    fn relay(url: &str) -> RelayHint {
        RelayHint::new(url).expect("test relay is valid")
    }

    #[test]
    fn fallback_order_is_stable_and_deduplicated() {
        let result = merge_relay_sources(
            vec![relay("https://fresh.example")],
            vec![
                relay("https://fresh.example"),
                relay("https://cache.example"),
            ],
            vec![relay("https://ticket.example")],
            4,
        );
        assert_eq!(
            result
                .iter()
                .map(|(source, hint)| (*source, hint.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (RouteSource::FreshLookup, "https://fresh.example"),
                (RouteSource::VerifiedCache, "https://cache.example"),
                (RouteSource::TransientTicket, "https://ticket.example"),
            ]
        );
    }

    #[test]
    fn full_fresh_source_does_not_starve_cache_or_ticket_fallback() {
        let result = merge_relay_sources(
            vec![
                relay("https://fresh-a.example"),
                relay("https://fresh-b.example"),
                relay("https://fresh-over-limit.example"),
            ],
            vec![
                relay("https://cache-a.example"),
                relay("https://cache-b.example"),
                relay("https://cache-over-limit.example"),
            ],
            vec![
                relay("https://ticket-a.example"),
                relay("https://ticket-b.example"),
                relay("https://ticket-over-limit.example"),
            ],
            2,
        );

        assert_eq!(
            result
                .iter()
                .map(|(source, hint)| (*source, hint.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (RouteSource::FreshLookup, "https://fresh-a.example"),
                (RouteSource::FreshLookup, "https://fresh-b.example"),
                (RouteSource::VerifiedCache, "https://cache-a.example"),
                (RouteSource::VerifiedCache, "https://cache-b.example"),
                (RouteSource::TransientTicket, "https://ticket-a.example"),
                (RouteSource::TransientTicket, "https://ticket-b.example"),
            ]
        );
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
        let fallback = CachedRouteFallback {
            relay_hints: vec![relay_hint],
            diagnostic: None,
        };

        let rendered = format!("{candidate:?} {fallback:?}");
        assert!(!rendered.contains(relay_sentinel));
        assert!(!rendered.contains(direct_sentinel));
        assert!(rendered.contains("TransientTicket"));
        assert!(rendered.contains("relay_address_count: 1"));
        assert!(rendered.contains("direct_address_count: 1"));
        assert!(rendered.contains("relay_hint_count: 1"));
        assert_eq!(candidate.relay_hint().as_str(), relay_sentinel);
        assert_eq!(candidate.endpoint_addr().id, remote);
        assert_eq!(candidate.endpoint_addr().ip_addrs().count(), 1);
    }

    #[test]
    fn cache_projection_retains_unknown_version_diagnostic_but_ignores_route() {
        let remote = DeviceId::from_array([0x31; 32]);
        let unsupported = KnownDevice {
            device_id: remote,
            local_alias: DeviceAlias::new("peer").expect("alias"),
            remote_name: DeviceDisplayName::new("Peer").expect("display name"),
            route_cache: None,
            route_cache_diagnostic: Some(RouteCacheDiagnostic::UnsupportedVersion { actual: 99 }),
        };
        assert_eq!(
            cached_route_fallback(Some(unsupported)),
            CachedRouteFallback {
                relay_hints: Vec::new(),
                diagnostic: Some(RouteCacheDiagnostic::UnsupportedVersion { actual: 99 }),
            }
        );

        let supported = KnownDevice {
            device_id: remote,
            local_alias: DeviceAlias::new("peer").expect("alias"),
            remote_name: DeviceDisplayName::new("Peer").expect("display name"),
            route_cache: Some(RelayRouteCache {
                relay_hints: vec![relay("https://cache.example")],
                verified_at_unix: 7,
            }),
            route_cache_diagnostic: None,
        };
        assert_eq!(
            cached_route_fallback(Some(supported)),
            CachedRouteFallback {
                relay_hints: vec![relay("https://cache.example")],
                diagnostic: None,
            }
        );
    }

    #[test]
    fn candidate_planning_preserves_fallback_order_and_profile_bytes() {
        let profile = InfrastructureProfile::SelfHosted {
            relay_url: "https://home.example".parse().expect("home Relay URL"),
        };
        let before = profile.summary();
        let remote = iroh::SecretKey::from_bytes(&[8; 32]).public();
        let candidates = build_relay_candidates(
            remote,
            vec![relay("https://fresh.example")],
            vec![
                relay("https://fresh.example"),
                relay("https://cache.example"),
            ],
            vec![relay("https://ticket.example")],
            4,
        )
        .expect("candidate plan");

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| (candidate.source(), candidate.relay_hint().as_str()))
                .collect::<Vec<_>>(),
            vec![
                (RouteSource::FreshLookup, "https://fresh.example"),
                (RouteSource::VerifiedCache, "https://cache.example"),
                (RouteSource::TransientTicket, "https://ticket.example"),
            ]
        );
        assert!(candidates.iter().all(|candidate| {
            candidate.endpoint_addr().id == remote
                && candidate.endpoint_addr().ip_addrs().next().is_none()
                && candidate.endpoint_addr().relay_urls().count() == 1
        }));
        assert_eq!(profile.summary(), before);
    }

    #[test]
    fn zero_candidate_bound_fails_closed_without_allocating_a_route() {
        assert!(
            merge_relay_sources(vec![relay("https://fresh.example")], vec![], vec![], 0).is_empty()
        );
    }
}
