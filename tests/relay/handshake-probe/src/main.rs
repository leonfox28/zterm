use std::{
    env,
    error::Error,
    io::{self, Write},
    time::Duration,
};

use iroh::{Endpoint, RelayConfig, RelayMap, RelayMode, RelayUrl, Watcher, endpoint::presets};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(45);
const RECONNECT_TIMEOUT: Duration = Duration::from_secs(90);

type ProbeResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ProbeResult<()> {
    let mut args = env::args().skip(1);
    let raw_url = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: zterm-relay-handshake-probe <relay-url> [--expect-reconnect]",
        )
    })?;
    let expect_reconnect = match args.next().as_deref() {
        None => false,
        Some("--expect-reconnect") => true,
        Some(other) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown argument: {other}"),
            )
            .into());
        }
    };
    if args.next().is_some() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "too many arguments").into());
    }
    let relay_url: RelayUrl = raw_url.parse()?;

    // The selected deployment provides Relay over HTTPS/WebSocket only. Passing
    // None is intentional: a bare RelayUrl would also enable UDP QAD on 7842.
    let relay_map: RelayMap = RelayConfig::new(relay_url.clone(), None).into();
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(relay_map))
        .bind()
        .await?;

    wait_for_initial_connection(&endpoint).await?;

    let statuses = endpoint.home_relay_status().get();
    let expected_connected = statuses
        .iter()
        .any(|status| status.url() == &relay_url && status.is_connected());
    if !expected_connected {
        let detail = format_statuses(&statuses);
        endpoint.close().await;
        return Err(io::Error::other(format!(
            "a relay connected, but not the requested URL {relay_url}: {detail}"
        ))
        .into());
    }

    println!("authenticated Iroh relay connection established: {relay_url} (QAD disabled)");
    io::stdout().flush()?;

    let reconnect_result = if expect_reconnect {
        wait_for_reconnect(&endpoint, &relay_url).await
    } else {
        Ok(())
    };
    endpoint.close().await;
    reconnect_result
}

async fn wait_for_initial_connection(endpoint: &Endpoint) -> ProbeResult<()> {
    if tokio::time::timeout(HANDSHAKE_TIMEOUT, endpoint.online())
        .await
        .is_err()
    {
        let detail = relay_status_detail(endpoint);
        endpoint.close().await;
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("relay authentication did not complete within 45 seconds: {detail}"),
        )
        .into());
    }
    Ok(())
}

async fn wait_for_reconnect(endpoint: &Endpoint, relay_url: &RelayUrl) -> ProbeResult<()> {
    let mut watcher = endpoint.home_relay_status();
    let result: ProbeResult<()> = tokio::time::timeout(RECONNECT_TIMEOUT, async {
        let mut saw_disconnect = false;
        loop {
            let statuses = watcher.updated().await.map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "home relay status watcher disconnected",
                )
            })?;
            let connected = statuses
                .iter()
                .any(|status| status.url() == relay_url && status.is_connected());

            if !connected && !saw_disconnect {
                saw_disconnect = true;
                println!("relay disconnect observed: {relay_url}");
                io::stdout().flush()?;
            } else if connected && saw_disconnect {
                println!("authenticated Iroh relay reconnection established: {relay_url}");
                io::stdout().flush()?;
                return Ok(());
            }
        }
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "relay disconnect and authenticated reconnection were not both observed within 90 seconds",
        )
    })?;
    result
}

fn relay_status_detail(endpoint: &Endpoint) -> String {
    format_statuses(&endpoint.home_relay_status().get())
}

fn format_statuses(statuses: &[iroh::endpoint::RelayStatus]) -> String {
    if statuses.is_empty() {
        return "no home relay status was reported".to_owned();
    }

    statuses
        .iter()
        .map(|status| {
            let state = if status.is_connected() {
                "connected".to_owned()
            } else if let Some(error) = status.last_error() {
                format!("disconnected: {error:#}")
            } else {
                "connecting".to_owned()
            };
            format!("{}={state}", status.url())
        })
        .collect::<Vec<_>>()
        .join(", ")
}
