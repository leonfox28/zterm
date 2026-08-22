//! Infrastructure profile contract tests.

use std::{collections::BTreeSet, process::Command};

use iroh::{
    RelayMode, RelayUrl, SecretKey,
    address_lookup::{N0_DNS_ENDPOINT_ORIGIN_PROD, N0_DNS_PKARR_RELAY_PROD},
    defaults::DEFAULT_RELAY_QUIC_PORT,
};
use zterm_daemon::transport::{InfrastructureProfile, ZTERM_ALPN};

const EXPECTED_N0_PRODUCTION_RELAYS: [&str; 4] = [
    "https://use1-1.relay.n0.iroh.link.",
    "https://usw1-1.relay.n0.iroh.link.",
    "https://euc1-1.relay.n0.iroh.link.",
    "https://aps1-1.relay.n0.iroh.link.",
];

const SELF_HOSTED_RELAY: &str = "https://relay.zenithconsulting.cn";
const EXPECTED_N0_PRODUCTION_PKARR_URL: &str = "https://dns.iroh.link/pkarr";
const EXPECTED_N0_PRODUCTION_DNS_ORIGIN: &str = "dns.iroh.link.";
const STAGING_INFRA_ENV: &str = "IROH_FORCE_STAGING_RELAYS";
const STAGING_ENV_CHILD: &str = "ZTERM_STAGING_ENV_PROFILE_CHILD";

#[test]
fn effective_profile_is_the_exact_n0_production_map_with_qad() {
    let profile = InfrastructureProfile::zterm();
    assert_production_relay_contract(&profile);
}

#[test]
fn effective_profile_excludes_staging_and_the_optional_self_hosted_relay() {
    let profile = InfrastructureProfile::zterm();
    let summary = profile.summary();
    let actual = relay_urls(summary.relays.into_iter().map(|relay| relay.url));
    let staging = relay_urls(
        RelayMode::Staging
            .relay_map()
            .relays::<Vec<_>>()
            .into_iter()
            .map(|relay| relay.url.clone()),
    );
    let self_hosted = parse_relay_url(SELF_HOSTED_RELAY);

    assert!(actual.is_disjoint(&staging));
    assert!(!actual.contains(&self_hosted));
    assert!(actual.iter().all(|url| {
        url.host_str()
            .is_some_and(|host| host.ends_with(".relay.n0.iroh.link."))
    }));
}

#[test]
fn effective_profile_uses_relay_only_n0_lookup_and_gate_alpn() {
    let profile = InfrastructureProfile::zterm();
    assert_production_lookup_contract(&profile);
}

#[test]
fn staging_environment_cannot_change_the_production_profile() {
    let output = Command::new(std::env::current_exe().expect("test executable is available"))
        .args([
            "--ignored",
            "--exact",
            "production_profile_under_staging_environment_child",
        ])
        .env(STAGING_INFRA_ENV, "1")
        .env(STAGING_ENV_CHILD, "1")
        .output()
        .expect("profile child process starts");

    assert!(
        output.status.success(),
        "profile child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "executed in an isolated child by staging_environment_cannot_change_the_production_profile"]
fn production_profile_under_staging_environment_child() {
    if std::env::var_os(STAGING_ENV_CHILD).is_none() {
        return;
    }

    assert!(std::env::var_os(STAGING_INFRA_ENV).is_some());
    let profile = InfrastructureProfile::zterm();
    assert_production_relay_contract(&profile);
    assert_production_lookup_contract(&profile);
}

fn parse_relay_url(url: &str) -> RelayUrl {
    url.parse().expect("test Relay URL is valid")
}

fn relay_urls(urls: impl IntoIterator<Item = RelayUrl>) -> BTreeSet<RelayUrl> {
    urls.into_iter().collect()
}

fn assert_production_relay_contract(profile: &InfrastructureProfile) {
    let summary = profile.summary();
    let actual = relay_urls(summary.relays.iter().map(|relay| relay.url.clone()));
    let expected = relay_urls(EXPECTED_N0_PRODUCTION_RELAYS.map(parse_relay_url));
    let official = relay_urls(
        RelayMode::Default
            .relay_map()
            .relays::<Vec<_>>()
            .into_iter()
            .map(|relay| relay.url.clone()),
    );

    assert_eq!(actual, expected);
    assert_eq!(actual, official);
    assert!(
        summary
            .relays
            .iter()
            .all(|relay| relay.quic_address_discovery),
        "every official production Relay must retain Iroh's QAD configuration"
    );
    assert!(RelayMode::Default.relay_map().relays::<Vec<_>>().iter().all(
        |relay| relay.quic.as_ref().map(|quic| quic.port) == Some(DEFAULT_RELAY_QUIC_PORT)
    ));
}

fn assert_production_lookup_contract(profile: &InfrastructureProfile) {
    let summary = profile.summary();

    assert_eq!(N0_DNS_PKARR_RELAY_PROD, EXPECTED_N0_PRODUCTION_PKARR_URL);
    assert_eq!(
        N0_DNS_ENDPOINT_ORIGIN_PROD,
        EXPECTED_N0_PRODUCTION_DNS_ORIGIN
    );
    assert_eq!(
        summary.pkarr_publisher_url.as_str(),
        EXPECTED_N0_PRODUCTION_PKARR_URL
    );
    assert_eq!(
        summary.pkarr_resolver_url.as_str(),
        EXPECTED_N0_PRODUCTION_PKARR_URL
    );
    assert_eq!(summary.dns_lookup_origin, EXPECTED_N0_PRODUCTION_DNS_ORIGIN);
    assert!(!summary.publishes_direct_addresses);
    assert!(summary.portmapper_enabled);
    assert_eq!(summary.alpns, [ZTERM_ALPN.to_vec()]);

    assert_effective_builder_contract(profile);
}

fn assert_effective_builder_contract(profile: &InfrastructureProfile) {
    let builder_debug = format!("{:?}", profile.endpoint_builder(SecretKey::generate()));

    // Iroh 1.0.3 does not expose read-only Builder accessors for lookup
    // services. Its pinned Debug projection lets this regression inspect the
    // builder that will actually be bound without publishing a test identity
    // to either production or staging infrastructure.
    assert_eq!(builder_debug.matches("PkarrPublisherBuilder").count(), 1);
    assert_eq!(builder_debug.matches("PkarrResolverBuilder").count(), 1);
    assert_eq!(builder_debug.matches("DnsAddressLookupBuilder").count(), 1);
    assert_eq!(
        builder_debug.matches("Domain(\"dns.iroh.link\")").count(),
        2
    );
    assert_eq!(builder_debug.matches("path: \"/pkarr\"").count(), 2);
    assert!(builder_debug.contains("origin_domain: \"dns.iroh.link.\""));
    assert!(!builder_debug.contains("staging-dns.iroh.link"));
    assert!(builder_debug.contains("addr_filter: Some(AddrFilter"));
    assert!(builder_debug.contains("portmapper_config: Enabled"));

    for relay in EXPECTED_N0_PRODUCTION_RELAYS.map(parse_relay_url) {
        let host = relay.host_str().expect("production Relay has a host");
        assert!(
            builder_debug.contains(host),
            "effective builder omitted production Relay {host}"
        );
    }
    for relay in RelayMode::Staging.relay_map().relays::<Vec<_>>() {
        let host = relay.url.host_str().expect("Iroh staging Relay has a host");
        assert!(
            !builder_debug.contains(host),
            "effective builder included staging Relay {host}"
        );
    }
}
