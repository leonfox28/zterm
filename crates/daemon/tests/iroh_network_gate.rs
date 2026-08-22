//! Explicit privileged/public-network Foundation Gate.

#![cfg(target_os = "linux")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    process::Command,
    sync::mpsc,
    time::Duration,
    time::Instant,
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures_util::StreamExt;
use iroh::{
    Endpoint, EndpointAddr, SecretKey, TransportAddr,
    endpoint::{Connection, PathEvent},
};
use patchbay::{Device, Lab, Nat};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};
use zterm_daemon::transport::{InfrastructureProfile, ZTERM_ALPN};

const ONLINE_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const STREAM_TIMEOUT: Duration = Duration::from_secs(30);
const CASE_TIMEOUT: Duration = Duration::from_secs(75);
const STREAM_COUNT: usize = 3;
const IROH_PORT_A: u16 = 41_101;
const IROH_PORT_B: u16 = 41_102;
const RAW_CONTROL_PORT_A: u16 = 42_101;
const RAW_CONTROL_PORT_B: u16 = 42_102;
const REFLECTOR_PORT: u16 = 43_101;
const RAW_CONTROL_TIMEOUT: Duration = Duration::from_secs(8);
const RELAY_IPV4_OVERRIDES_ENV: &str = "ZTERM_GATE_RELAY_IPV4_OVERRIDES";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GateCase {
    Product,
    ConfigCandidate,
    RelayFallback,
}

impl GateCase {
    fn label(self) -> &'static str {
        match self {
            Self::Product => "A",
            Self::ConfigCandidate => "B",
            Self::RelayFallback => "C",
        }
    }

    fn index(self) -> u8 {
        match self {
            Self::Product => 0,
            Self::ConfigCandidate => 1,
            Self::RelayFallback => 2,
        }
    }

    fn observation_timeout(self) -> Duration {
        match self {
            Self::Product | Self::ConfigCandidate => Duration::from_secs(15),
            Self::RelayFallback => Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PathKind {
    Direct,
    Relay,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NetworkGateVerdict {
    Go,
    GoWithDeferredAddressDiscovery,
}

impl fmt::Display for PathKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::Relay => write!(f, "relay"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[derive(Clone, Debug)]
struct EndpointEvidence {
    relay_urls: Vec<String>,
    candidate_sources: Vec<&'static str>,
}

#[derive(Clone, Debug)]
struct ResolvedRelay {
    host: String,
    ipv4: Ipv4Addr,
}

#[derive(Debug)]
struct ServerAdvert {
    dial_addr: EndpointAddr,
}

#[derive(Debug)]
struct PathEvidence {
    initial: PathKind,
    final_selected: PathKind,
    direct_selected: bool,
    timeline: Vec<String>,
}

#[derive(Debug)]
struct ClientEvidence {
    endpoint: EndpointEvidence,
    path: PathEvidence,
    stream_count: usize,
}

#[derive(Debug)]
struct CaseOutcome {
    server: EndpointEvidence,
    client: ClientEvidence,
    raw_udp_control: bool,
    non_dns_udp_blocked: bool,
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the disposable privileged Colima network Gate container"]
async fn double_nat_public_relay_gate() -> Result<()> {
    command("sysctl", &["-q", "-w", "net.ipv4.ip_forward=1"])
        .context("enable forwarding only inside the disposable container")?;
    let relays = resolve_production_relays()?;

    let case_a = run_case(GateCase::Product, &relays).await;
    let case_b = run_case(GateCase::ConfigCandidate, &relays).await;
    let case_c = run_case(GateCase::RelayFallback, &relays).await;

    print_case_result(GateCase::Product, &case_a);
    print_case_result(GateCase::ConfigCandidate, &case_b);
    print_case_result(GateCase::RelayFallback, &case_c);

    let case_a = case_a.context("Case A product profile failed to complete")?;
    let case_b = case_b.context("Case B configured-candidate control failed to complete")?;
    let case_c = case_c.context("Case C Relay fallback failed to complete")?;

    ensure_relay_contract(&case_a, &relays)?;
    ensure_relay_contract(&case_b, &relays)?;
    ensure_relay_contract(&case_c, &relays)?;
    match classify_network_gate(&case_a, &case_b, &case_c)? {
        NetworkGateVerdict::Go => println!("NETWORK_GATE=GO"),
        NetworkGateVerdict::GoWithDeferredAddressDiscovery => println!(
            "NETWORK_GATE=GO_WITH_DEFERRED_ADDRESS_DISCOVERY: Case A stayed relayed in the nested Colima/Patchbay/TUN lab; Case B became direct; Case C official WSS/TCP Relay fallback passed; real two-network automatic discovery is deferred to parent M10"
        ),
    }
    Ok(())
}

fn classify_network_gate(
    case_a: &CaseOutcome,
    case_b: &CaseOutcome,
    case_c: &CaseOutcome,
) -> Result<NetworkGateVerdict> {
    ensure!(
        case_b.raw_udp_control
            && case_b.client.path.direct_selected
            && case_b.client.path.final_selected == PathKind::Direct,
        "NETWORK_GATE=NO_GO_TRANSPORT: Case B did not retain a direct path after its raw UDP control passed"
    );
    ensure!(
        case_c.non_dns_udp_blocked
            && !case_c.client.path.direct_selected
            && case_c.client.path.final_selected == PathKind::Relay,
        "NETWORK_GATE=NO_GO_RELAY: Case C did not prove WSS/TCP Relay fallback while non-DNS UDP was blocked"
    );

    match (
        case_a.client.path.direct_selected,
        case_a.client.path.final_selected,
    ) {
        (true, PathKind::Direct) => Ok(NetworkGateVerdict::Go),
        (false, PathKind::Relay) => Ok(NetworkGateVerdict::GoWithDeferredAddressDiscovery),
        _ => bail!(
            "NETWORK_GATE=NO_GO_ADDRESS_DISCOVERY: Case A neither retained a direct path nor stayed exclusively on Relay"
        ),
    }
}

async fn run_case(case: GateCase, relays: &[ResolvedRelay]) -> Result<CaseOutcome> {
    let lab = Lab::builder()
        .allow_real_root()
        .label(format!("zterm-foundation-{}", case.label()))
        .build()
        .await
        .with_context(|| format!("create Patchbay lab for Case {}", case.label()))?;
    let _public_egress = configure_public_egress(&lab, case.index())?;

    let dns = lab.dns_server().context("start Patchbay DNS")?;
    for relay in relays {
        dns.set_host(&relay.host, IpAddr::V4(relay.ipv4))
            .with_context(|| format!("install pre-resolved A record for {}", relay.host))?;
    }

    let nat_a = lab
        .add_router("nat-a")
        .nat(Nat::Home)
        .build()
        .await
        .context("build Home NAT A")?;
    let nat_b = lab
        .add_router("nat-b")
        .nat(Nat::Home)
        .build()
        .await
        .context("build Home NAT B")?;
    let endpoint_a = lab
        .add_device("endpoint-a")
        .uplink(nat_a.id())
        .build()
        .await
        .context("build endpoint A namespace")?;
    let endpoint_b = lab
        .add_device("endpoint-b")
        .uplink(nat_b.id())
        .build()
        .await
        .context("build endpoint B namespace")?;

    let non_dns_udp_blocked = case == GateCase::RelayFallback;
    if non_dns_udp_blocked {
        block_simulated_direct_udp(&endpoint_a)?;
        block_simulated_direct_udp(&endpoint_b)?;
    }

    let (bind_a, external_a, bind_b, external_b, raw_udp_control) =
        if case == GateCase::ConfigCandidate {
            let reflector_router = lab
                .add_router("reflector-router")
                .build()
                .await
                .context("build Case B reflector router")?;
            let reflector = lab
                .add_device("reflector")
                .uplink(reflector_router.id())
                .build()
                .await
                .context("build Case B reflector device")?;
            let reflector_addr = SocketAddr::new(
                reflector
                    .ip()
                    .context("Case B reflector has no IPv4 address")?
                    .into(),
                REFLECTOR_PORT,
            );
            let _reflector_guard = reflector
                .spawn_reflector(reflector_addr)
                .await
                .context("start Case B address reflector")?;
            verify_raw_udp_holepunch(&endpoint_a, &endpoint_b, reflector_addr)
                .context("Case B raw UDP fixture control failed")?;
            let external_a = discover_external_candidate(&endpoint_a, IROH_PORT_A, reflector_addr)?;
            let external_b = discover_external_candidate(&endpoint_b, IROH_PORT_B, reflector_addr)?;
            // The raw control uses separate ports. Reflector probes on Iroh's
            // fixed ports discover the exact EIM mapping without opening the
            // peer-specific NAT filters used by the later transport control.
            (
                Some(IROH_PORT_A),
                Some(external_a),
                Some(IROH_PORT_B),
                Some(external_b),
                true,
            )
        } else {
            (None, None, None, None, false)
        };

    let (advert_tx, advert_rx) = oneshot::channel();
    let (done_tx, done_rx) = oneshot::channel();
    let server =
        endpoint_a.spawn(move |_device| server_task(bind_a, external_a, advert_tx, done_rx))?;
    let observation_timeout = case.observation_timeout();
    let client = endpoint_b.spawn(move |_device| {
        client_task(
            bind_b,
            external_b,
            advert_rx,
            done_tx,
            observation_timeout,
            case.label(),
        )
    })?;

    let (server, client) = timeout(CASE_TIMEOUT, async {
        tokio::try_join!(join_device_task(server), join_device_task(client))
    })
    .await
    .with_context(|| format!("Case {} exceeded {CASE_TIMEOUT:?}", case.label()))??;

    drop(lab);
    Ok(CaseOutcome {
        server,
        client,
        raw_udp_control,
        non_dns_udp_blocked,
    })
}

fn verify_raw_udp_holepunch(
    endpoint_a: &Device,
    endpoint_b: &Device,
    reflector: SocketAddr,
) -> Result<()> {
    let (addr_a_tx, addr_a_rx) = mpsc::channel();
    let (addr_b_tx, addr_b_rx) = mpsc::channel();
    let task_a = endpoint_a
        .spawn_thread(move || raw_udp_peer(RAW_CONTROL_PORT_A, reflector, addr_a_tx, addr_b_rx))?;
    let task_b = endpoint_b
        .spawn_thread(move || raw_udp_peer(RAW_CONTROL_PORT_B, reflector, addr_b_tx, addr_a_rx))?;

    task_a
        .join()
        .map_err(|_| anyhow!("raw UDP endpoint A thread panicked"))??;
    task_b
        .join()
        .map_err(|_| anyhow!("raw UDP endpoint B thread panicked"))??;
    Ok(())
}

fn raw_udp_peer(
    bind_port: u16,
    reflector: SocketAddr,
    own_addr_tx: mpsc::Sender<SocketAddr>,
    peer_addr_rx: mpsc::Receiver<SocketAddr>,
) -> Result<()> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, bind_port))
        .with_context(|| format!("bind raw UDP control port {bind_port}"))?;
    socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .context("set raw UDP control read timeout")?;
    let observed = reflected_addr(&socket, reflector)?;
    own_addr_tx
        .send(observed)
        .map_err(|_| anyhow!("raw UDP peer stopped before address exchange"))?;
    let peer = peer_addr_rx
        .recv_timeout(RAW_CONTROL_TIMEOUT)
        .context("raw UDP peer address exchange timed out")?;

    let payload = b"zterm-foundation-raw-udp";
    let deadline = Instant::now() + RAW_CONTROL_TIMEOUT;
    let mut buffer = [0_u8; 64];
    loop {
        socket
            .send_to(payload, peer)
            .with_context(|| format!("send raw UDP control probe to {peer}"))?;
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) if source == peer && &buffer[..length] == payload => {
                for _ in 0..3 {
                    let _ = socket.send_to(payload, peer);
                }
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(error).context("receive raw UDP control probe"),
        }
        if Instant::now() >= deadline {
            bail!("raw UDP holepunch timed out for local port {bind_port}");
        }
    }
}

fn discover_external_candidate(
    endpoint: &Device,
    bind_port: u16,
    reflector: SocketAddr,
) -> Result<SocketAddr> {
    endpoint
        .spawn_thread(move || {
            let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, bind_port))
                .with_context(|| format!("bind Iroh candidate discovery port {bind_port}"))?;
            socket
                .set_read_timeout(Some(Duration::from_millis(200)))
                .context("set candidate discovery timeout")?;
            reflected_addr(&socket, reflector)
        })?
        .join()
        .map_err(|_| anyhow!("candidate discovery thread panicked"))?
}

fn reflected_addr(socket: &std::net::UdpSocket, reflector: SocketAddr) -> Result<SocketAddr> {
    let deadline = Instant::now() + RAW_CONTROL_TIMEOUT;
    let mut buffer = [0_u8; 128];
    loop {
        socket
            .send_to(b"PROBE", reflector)
            .with_context(|| format!("send address probe to {reflector}"))?;
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) if source == reflector => {
                let reply = std::str::from_utf8(&buffer[..length])
                    .context("reflector response is not UTF-8")?;
                let observed = reply
                    .strip_prefix("OBSERVED ")
                    .context("reflector response has an unexpected format")?
                    .parse()
                    .context("reflector returned an invalid socket address")?;
                return Ok(observed);
            }
            Ok(_) => {}
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(error).context("receive address probe response"),
        }
        if Instant::now() >= deadline {
            bail!("address reflection timed out for {}", socket.local_addr()?);
        }
    }
}

async fn server_task(
    bind_port: Option<u16>,
    external_addr: Option<SocketAddr>,
    advert_tx: oneshot::Sender<ServerAdvert>,
    done_rx: oneshot::Receiver<()>,
) -> Result<EndpointEvidence> {
    let endpoint = bind_endpoint(bind_port, external_addr).await?;
    let addr = endpoint.addr();
    let evidence = endpoint_evidence(&addr, external_addr);
    let dial_addr = EndpointAddr::from_parts(
        addr.id,
        addr.addrs.into_iter().filter(|addr| {
            addr.is_relay()
                || matches!(
                    (addr, external_addr),
                    (TransportAddr::Ip(candidate), Some(configured)) if *candidate == configured
                )
        }),
    );
    ensure!(
        dial_addr.relay_urls().count() == 1,
        "server endpoint did not select exactly one home Relay"
    );
    ensure!(
        external_addr.is_none() || dial_addr.ip_addrs().count() == 1,
        "configured-candidate control did not retain exactly one Config address"
    );
    advert_tx
        .send(ServerAdvert { dial_addr })
        .map_err(|_| anyhow!("client stopped before receiving the server address"))?;

    let incoming = timeout(CONNECT_TIMEOUT, endpoint.accept())
        .await
        .context("server accept timed out")?
        .context("server endpoint closed before accepting")?;
    let connection = timeout(CONNECT_TIMEOUT, incoming)
        .await
        .context("server handshake timed out")?
        .context("server handshake failed")?;

    for stream_index in 0..STREAM_COUNT {
        echo_one_stream(&connection, stream_index).await?;
    }
    timeout(STREAM_TIMEOUT, done_rx)
        .await
        .context("server completion barrier timed out")?
        .context("client stopped before completing stream verification")?;

    connection.close(0u32.into(), b"foundation gate complete");
    timeout(STREAM_TIMEOUT, endpoint.close())
        .await
        .context("server endpoint close timed out")?;
    Ok(evidence)
}

async fn client_task(
    bind_port: Option<u16>,
    external_addr: Option<SocketAddr>,
    advert_rx: oneshot::Receiver<ServerAdvert>,
    done_tx: oneshot::Sender<()>,
    observation_timeout: Duration,
    case_label: &'static str,
) -> Result<ClientEvidence> {
    let endpoint = bind_endpoint(bind_port, external_addr).await?;
    let evidence = endpoint_evidence(&endpoint.addr(), external_addr);
    let advert = timeout(STREAM_TIMEOUT, advert_rx)
        .await
        .context("client address exchange timed out")?
        .context("server stopped before advertising its address")?;
    let connection = timeout(
        CONNECT_TIMEOUT,
        endpoint.connect(advert.dial_addr, ZTERM_ALPN),
    )
    .await
    .context("client connect timed out")?
    .context("client connect failed")?;

    let path = observe_paths(&connection, observation_timeout).await;
    for stream_index in 0..STREAM_COUNT {
        roundtrip_one_stream(&connection, case_label, stream_index).await?;
    }
    done_tx
        .send(())
        .map_err(|_| anyhow!("server stopped before client completion"))?;

    connection.close(0u32.into(), b"foundation gate complete");
    timeout(STREAM_TIMEOUT, endpoint.close())
        .await
        .context("client endpoint close timed out")?;
    Ok(ClientEvidence {
        endpoint: evidence,
        path,
        stream_count: STREAM_COUNT,
    })
}

async fn bind_endpoint(
    bind_port: Option<u16>,
    external_addr: Option<SocketAddr>,
) -> Result<Endpoint> {
    let profile = InfrastructureProfile::zterm();
    let mut builder = profile.endpoint_builder(SecretKey::generate());
    if let Some(port) = bind_port {
        builder = builder
            .bind_addr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port))
            .context("set deterministic test bind port")?;
    }
    if let Some(addr) = external_addr {
        builder = builder.external_addr(addr);
    }
    let endpoint = timeout(ONLINE_TIMEOUT, builder.bind())
        .await
        .context("endpoint bind timed out")?
        .context("endpoint bind failed")?;
    if let Some(port) = bind_port {
        ensure!(
            endpoint
                .bound_sockets()
                .iter()
                .any(|addr| addr.is_ipv4() && addr.port() == port),
            "configured IPv4 Iroh port {port} was not bound"
        );
    }
    timeout(ONLINE_TIMEOUT, endpoint.online())
        .await
        .context("endpoint did not connect to an official production Relay")?;
    Ok(endpoint)
}

fn endpoint_evidence(
    addr: &EndpointAddr,
    configured_external: Option<SocketAddr>,
) -> EndpointEvidence {
    let relay_urls = addr.relay_urls().map(ToString::to_string).collect();
    let candidate_sources = addr
        .ip_addrs()
        .map(|candidate| {
            if configured_external == Some(*candidate) {
                "Config"
            } else {
                // EndpointAddr intentionally omits Iroh's internal
                // DirectAddrType, so QAD/port-mapped/local cannot be
                // distinguished at this public boundary.
                "Unclassified"
            }
        })
        .collect();
    EndpointEvidence {
        relay_urls,
        candidate_sources,
    }
}

async fn observe_paths(connection: &Connection, wait: Duration) -> PathEvidence {
    let mut events = connection.path_events();
    let initial = selected_path(connection);
    let mut timeline = vec![format!("initial:{initial}")];
    let mut direct_selected = initial == PathKind::Direct;

    if !direct_selected {
        let observed = timeout(wait, async {
            while let Some(event) = events.next().await {
                let (description, selected) = summarize_path_event(&event);
                timeline.push(description);
                if selected == Some(PathKind::Direct) {
                    return true;
                }
                if matches!(event, PathEvent::Lagged { .. })
                    && selected_path(connection) == PathKind::Direct
                {
                    return true;
                }
            }
            false
        })
        .await;
        direct_selected = observed.unwrap_or(false);
    }

    let final_selected = selected_path(connection);
    timeline.push(format!("final:{final_selected}"));
    PathEvidence {
        initial,
        final_selected,
        direct_selected: direct_selected || final_selected == PathKind::Direct,
        timeline,
    }
}

fn summarize_path_event(event: &PathEvent) -> (String, Option<PathKind>) {
    match event {
        PathEvent::Opened { remote_addr, .. } => {
            (format!("opened:{}", path_kind(remote_addr)), None)
        }
        PathEvent::Closed { remote_addr, .. } => {
            (format!("closed:{}", path_kind(remote_addr)), None)
        }
        PathEvent::Selected { remote_addr, .. } => {
            let kind = path_kind(remote_addr);
            (format!("selected:{kind}"), Some(kind))
        }
        PathEvent::Lagged { missed, .. } => (format!("lagged:{missed}"), None),
        _ => ("unknown-event".to_string(), None),
    }
}

fn selected_path(connection: &Connection) -> PathKind {
    connection
        .paths()
        .iter()
        .find(|path| path.is_selected())
        .map(|path| {
            if path.is_ip() {
                PathKind::Direct
            } else if path.is_relay() {
                PathKind::Relay
            } else {
                PathKind::Unknown
            }
        })
        .unwrap_or(PathKind::Unknown)
}

fn path_kind(addr: &TransportAddr) -> PathKind {
    if addr.is_ip() {
        PathKind::Direct
    } else if addr.is_relay() {
        PathKind::Relay
    } else {
        PathKind::Unknown
    }
}

async fn echo_one_stream(connection: &Connection, stream_index: usize) -> Result<()> {
    timeout(STREAM_TIMEOUT, async {
        let (mut send, mut receive) = connection
            .accept_bi()
            .await
            .with_context(|| format!("accept stream {stream_index}"))?;
        let payload = receive
            .read_to_end(256)
            .await
            .with_context(|| format!("read stream {stream_index}"))?;
        send.write_all(&payload)
            .await
            .with_context(|| format!("echo stream {stream_index}"))?;
        send.finish()
            .with_context(|| format!("finish stream {stream_index}"))?;
        Result::<()>::Ok(())
    })
    .await
    .with_context(|| format!("stream {stream_index} server deadline"))?
}

async fn roundtrip_one_stream(
    connection: &Connection,
    case_label: &str,
    stream_index: usize,
) -> Result<()> {
    timeout(STREAM_TIMEOUT, async {
        let expected = format!("zterm-gate-{case_label}-{stream_index}").into_bytes();
        let (mut send, mut receive) = connection
            .open_bi()
            .await
            .with_context(|| format!("open stream {stream_index}"))?;
        send.write_all(&expected)
            .await
            .with_context(|| format!("write stream {stream_index}"))?;
        send.finish()
            .with_context(|| format!("finish stream {stream_index}"))?;
        let actual = receive
            .read_to_end(256)
            .await
            .with_context(|| format!("read echo {stream_index}"))?;
        ensure!(actual == expected, "stream {stream_index} echo mismatch");
        Result::<()>::Ok(())
    })
    .await
    .with_context(|| format!("stream {stream_index} client deadline"))?
}

async fn join_device_task<T>(task: JoinHandle<Result<T>>) -> Result<T> {
    task.await.context("Patchbay device task panicked")?
}

struct PublicEgress {
    outer_if: String,
    nft_table: String,
}

impl Drop for PublicEgress {
    fn drop(&mut self) {
        let _ = Command::new("ip")
            .args(["link", "delete", &self.outer_if])
            .output();
        let _ = Command::new("nft")
            .args(["delete", "table", "ip", &self.nft_table])
            .output();
    }
}

fn configure_public_egress(lab: &Lab, index: u8) -> Result<PublicEgress> {
    let ix_if = format!("ztix{index}");
    let outer_if = format!("ztox{index}");
    let table = format!("ztg{index}");
    let public_egress = PublicEgress {
        outer_if: outer_if.clone(),
        nft_table: table.clone(),
    };
    let octet = 240 + index;
    let ix_addr = format!("172.31.{octet}.2/30");
    let outer_addr = format!("172.31.{octet}.1/30");
    let outer_gateway = format!("172.31.{octet}.1");
    let ix_gateway = format!("172.31.{octet}.2");
    let process_id = std::process::id().to_string();

    let ix_if_in_ns = ix_if.clone();
    let outer_if_in_ns = outer_if.clone();
    lab.ix().run_sync(move || {
        command(
            "ip",
            &[
                "link",
                "add",
                &ix_if_in_ns,
                "type",
                "veth",
                "peer",
                "name",
                &outer_if_in_ns,
            ],
        )?;
        command("ip", &["addr", "add", &ix_addr, "dev", &ix_if_in_ns])?;
        command("ip", &["link", "set", &ix_if_in_ns, "up"])?;
        command(
            "ip",
            &["link", "set", &outer_if_in_ns, "netns", &process_id],
        )?;
        command("ip", &["route", "add", "default", "via", &outer_gateway])?;
        Ok(())
    })?;

    command("ip", &["addr", "add", &outer_addr, "dev", &outer_if])?;
    command("ip", &["link", "set", &outer_if, "up"])?;
    command(
        "ip",
        &[
            "route",
            "add",
            "198.18.0.0/15",
            "via",
            &ix_gateway,
            "dev",
            &outer_if,
        ],
    )?;

    command("nft", &["add", "table", "ip", &table])?;
    command(
        "nft",
        &[
            "add",
            "chain",
            "ip",
            &table,
            "postrouting",
            "{ type nat hook postrouting priority srcnat; policy accept; }",
        ],
    )?;
    command(
        "nft",
        &[
            "add",
            "rule",
            "ip",
            &table,
            "postrouting",
            "ip",
            "saddr",
            "198.18.0.0/15",
            "oifname",
            "eth0",
            "masquerade",
        ],
    )?;
    Ok(public_egress)
}

fn block_simulated_direct_udp(device: &Device) -> Result<()> {
    device.run_sync(|| {
        command("nft", &["add", "table", "inet", "zterm_gate"])?;
        command(
            "nft",
            &[
                "add",
                "chain",
                "inet",
                "zterm_gate",
                "output",
                "{ type filter hook output priority filter; policy accept; }",
            ],
        )?;
        command(
            "nft",
            &[
                "add",
                "rule",
                "inet",
                "zterm_gate",
                "output",
                "udp",
                "dport",
                "53",
                "accept",
            ],
        )?;
        command(
            "nft",
            &[
                "add",
                "rule",
                "inet",
                "zterm_gate",
                "output",
                "meta",
                "l4proto",
                "udp",
                "drop",
            ],
        )?;
        Ok(())
    })
}

fn resolve_production_relays() -> Result<Vec<ResolvedRelay>> {
    let summary = InfrastructureProfile::zterm().summary();
    let overrides = relay_ipv4_overrides()?;
    ensure!(
        summary.relays.len() == 4,
        "the product profile must expose the four Iroh 1.0.3 production Relays"
    );
    summary
        .relays
        .into_iter()
        .map(|relay| {
            ensure!(
                relay.quic_address_discovery,
                "production Relay {} has QAD disabled",
                relay.url
            );
            let host = relay
                .url
                .host_str()
                .context("production Relay URL has no hostname")?
                .to_string();
            let ipv4 = match overrides.get(&host) {
                Some(ipv4) => *ipv4,
                None => (host.as_str(), 443)
                    .to_socket_addrs()
                    .with_context(|| format!("resolve production Relay {host}"))?
                    .find_map(|addr| match addr.ip() {
                        IpAddr::V4(ip) => Some(ip),
                        IpAddr::V6(_) => None,
                    })
                    .with_context(|| format!("production Relay {host} has no IPv4 address"))?,
            };
            Ok(ResolvedRelay { host, ipv4 })
        })
        .collect()
}

fn relay_ipv4_overrides() -> Result<BTreeMap<String, Ipv4Addr>> {
    let Ok(value) = std::env::var(RELAY_IPV4_OVERRIDES_ENV) else {
        return Ok(BTreeMap::new());
    };
    value.split(';').filter(|entry| !entry.is_empty()).try_fold(
        BTreeMap::new(),
        |mut overrides, entry| {
            let (host, ipv4) = entry
                .split_once('=')
                .with_context(|| format!("invalid {RELAY_IPV4_OVERRIDES_ENV} entry {entry:?}"))?;
            let ipv4 = ipv4
                .parse()
                .with_context(|| format!("invalid IPv4 override for {host}"))?;
            ensure!(
                overrides.insert(host.to_string(), ipv4).is_none(),
                "duplicate IPv4 override for {host}"
            );
            Ok(overrides)
        },
    )
}

fn ensure_relay_contract(outcome: &CaseOutcome, relays: &[ResolvedRelay]) -> Result<()> {
    let official_urls = InfrastructureProfile::zterm()
        .summary()
        .relays
        .into_iter()
        .map(|relay| relay.url.to_string())
        .collect::<BTreeSet<_>>();
    ensure!(
        outcome.server.relay_urls.len() == 1 && outcome.client.endpoint.relay_urls.len() == 1,
        "each endpoint must expose exactly one home Relay"
    );
    ensure!(
        relays.len() == official_urls.len()
            && outcome
                .server
                .relay_urls
                .iter()
                .chain(&outcome.client.endpoint.relay_urls)
                .all(|url| official_urls.contains(url)),
        "each endpoint home Relay must belong to the official n0 production map"
    );
    ensure!(
        outcome.client.stream_count == STREAM_COUNT,
        "each Case must verify {STREAM_COUNT} independent bidi streams"
    );
    ensure!(
        outcome.client.path.initial == PathKind::Relay,
        "the Iroh connection must begin on an official production Relay"
    );
    Ok(())
}

fn print_case_result(case: GateCase, result: &Result<CaseOutcome>) {
    match result {
        Ok(outcome) => println!(
            "CASE={} STATUS=complete RAW_UDP_CONTROL={} NON_DNS_UDP_BLOCKED={} RELAY_TRANSPORT={} INITIAL={} FINAL={} DIRECT={} STREAMS={} SERVER_RELAYS={:?} CLIENT_RELAYS={:?} SERVER_CANDIDATES={:?} CLIENT_CANDIDATES={:?} TIMELINE={:?}",
            case.label(),
            if outcome.raw_udp_control {
                "passed"
            } else {
                "not-run"
            },
            outcome.non_dns_udp_blocked,
            if outcome.non_dns_udp_blocked && outcome.client.path.final_selected == PathKind::Relay
            {
                "wss-tcp"
            } else {
                "not-constrained"
            },
            outcome.client.path.initial,
            outcome.client.path.final_selected,
            outcome.client.path.direct_selected,
            outcome.client.stream_count,
            outcome.server.relay_urls,
            outcome.client.endpoint.relay_urls,
            outcome.server.candidate_sources,
            outcome.client.endpoint.candidate_sources,
            outcome.client.path.timeline,
        ),
        Err(error) => println!("CASE={} STATUS=error ERROR={error:#}", case.label()),
    }
}

fn command(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {:?} failed with {}: {}{}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(
        final_selected: PathKind,
        direct_selected: bool,
        raw_udp_control: bool,
        non_dns_udp_blocked: bool,
    ) -> CaseOutcome {
        CaseOutcome {
            server: EndpointEvidence {
                relay_urls: Vec::new(),
                candidate_sources: Vec::new(),
            },
            client: ClientEvidence {
                endpoint: EndpointEvidence {
                    relay_urls: Vec::new(),
                    candidate_sources: Vec::new(),
                },
                path: PathEvidence {
                    initial: PathKind::Relay,
                    final_selected,
                    direct_selected,
                    timeline: Vec::new(),
                },
                stream_count: STREAM_COUNT,
            },
            raw_udp_control,
            non_dns_udp_blocked,
        }
    }

    fn retained_b_direct() -> CaseOutcome {
        outcome(PathKind::Direct, true, true, false)
    }

    fn retained_c_relay() -> CaseOutcome {
        outcome(PathKind::Relay, false, false, true)
    }

    #[test]
    fn verdict_requires_the_exact_retained_a_b_c_paths() {
        assert_eq!(
            classify_network_gate(
                &outcome(PathKind::Relay, false, false, false),
                &retained_b_direct(),
                &retained_c_relay(),
            )
            .expect("retained relay/direct/relay evidence is valid"),
            NetworkGateVerdict::GoWithDeferredAddressDiscovery,
        );
        assert_eq!(
            classify_network_gate(
                &outcome(PathKind::Direct, true, false, false),
                &retained_b_direct(),
                &retained_c_relay(),
            )
            .expect("retained direct/direct/relay evidence is valid"),
            NetworkGateVerdict::Go,
        );

        assert!(
            classify_network_gate(
                &outcome(PathKind::Unknown, false, false, false),
                &retained_b_direct(),
                &retained_c_relay(),
            )
            .expect_err("unknown Case A path must not become a deferred verdict")
            .to_string()
            .contains("NO_GO_ADDRESS_DISCOVERY")
        );
    }

    #[test]
    fn verdict_keeps_case_b_and_c_as_hard_failures() {
        let retained_a_relay = outcome(PathKind::Relay, false, false, false);
        assert!(
            classify_network_gate(
                &retained_a_relay,
                &outcome(PathKind::Relay, true, true, false),
                &retained_c_relay(),
            )
            .expect_err("Case B must finish on direct")
            .to_string()
            .contains("NO_GO_TRANSPORT")
        );
        assert!(
            classify_network_gate(
                &retained_a_relay,
                &retained_b_direct(),
                &outcome(PathKind::Direct, true, false, true),
            )
            .expect_err("Case C must remain on Relay")
            .to_string()
            .contains("NO_GO_RELAY")
        );
    }
}
