//! Relay-only address resolution without mutating the configured Iroh profile.

use std::fmt;
use std::time::Instant;

use iroh::Endpoint;
use zterm_core::{DeviceId, DomainErrorKind, RelayHint, TransportLimits};

use crate::error::DaemonError;
use crate::store::{KnownDevice, RouteCacheDiagnostic, StoreHandle};

#[cfg(test)]
use zterm_client::route::merge_relay_sources;
pub use zterm_client::route::{RouteCandidate, RouteSource};
use zterm_client::route::{build_relay_candidates, fresh_relay_hints};
pub(crate) use zterm_client::route::{device_from_endpoint_id, endpoint_id_from_device};

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
        let fresh = fresh_relay_hints(self.limits, endpoint, endpoint_id, deadline).await;

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
    fn cached_route_debug_redacts_relay_values() {
        let sentinel = "https://route-cache-sentinel.example.test/private";
        let fallback = CachedRouteFallback {
            relay_hints: vec![relay(sentinel)],
            diagnostic: None,
        };
        let rendered = format!("{fallback:?}");
        assert!(!rendered.contains(sentinel));
        assert!(rendered.contains("relay_hint_count: 1"));
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
