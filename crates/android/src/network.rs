//! Application-owned pairing and Session API above the shared controller.
use crate::{
    NativeError, NativeRuntime,
    dns::{AndroidDns, NativeDnsServer},
};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;
use zterm_client::{
    iroh_controller::{IrohController, PairedHost, endpoint_builder},
    model::{ResolvedSessionTarget, SessionSummary},
};
use zterm_core::{DeviceId, RelayHint};

/// Persisted local host projection; routes must originate from confirmed pairing.
#[derive(Clone, Debug, uniffi::Record)]
pub struct NativeHost {
    /// Exact public device key, independent of its display name.
    pub device_id: String,
    /// User-visible host name.
    pub name: String,
    /// Previously authenticated relay-only route cache.
    pub relay_urls: Vec<String>,
}
/// Content-free connection evidence; never includes peer addresses or frame contents.
#[derive(Clone, Debug, uniffi::Record)]
pub struct NativeConnectionInfo {
    /// Selected unknown/direct/relay path.
    pub path: String,
    /// Selected path RTT.
    pub rtt_ms: u64,
    /// Sent QUIC bytes.
    pub sent_bytes: u64,
    /// Received QUIC bytes.
    pub received_bytes: u64,
    /// Lost packets.
    pub lost_packets: u64,
    /// Transport-reported closure.
    pub closed: bool,
}
/// Compact Session row identified by the host's stable ID.
#[derive(Clone, Debug, uniffi::Record)]
pub struct NativeSession {
    /// Exact daemon-lifetime Session identity.
    pub session_id: String,
    /// Host-validated unique Session name.
    pub name: String,
    /// Whether an existing attachment currently owns controller input.
    pub occupied: bool,
    /// Most recent terminal row count.
    pub rows: u16,
    /// Most recent terminal column count.
    pub columns: u16,
}
impl From<SessionSummary> for NativeSession {
    fn from(value: SessionSummary) -> Self {
        Self {
            session_id: value.session_id.to_string(),
            name: value.name.to_string(),
            occupied: value.has_controller,
            rows: value.viewport.rows,
            columns: value.viewport.columns,
        }
    }
}
/// Provisional pairing result retained until Android atomically saves its host record.
#[derive(uniffi::Object)]
pub struct NativePairing {
    host: NativeHost,
    paired: Mutex<Option<PairedHost>>,
}
#[uniffi::export]
impl NativePairing {
    /// Returns only the non-secret state which Android must durably save.
    pub fn host(&self) -> NativeHost {
        self.host.clone()
    }
}
pub(crate) struct Network {
    pub(crate) controller: IrohController,
    dns: AndroidDns,
}

#[uniffi::export]
impl NativeRuntime {
    /// Shared protocol text limit for camera, gallery and manual admission.
    pub fn ticket_text_limit(&self) -> u32 {
        zterm_core::MAX_TICKET_TEXT_BYTES as u32
    }
    /// Parses QR candidates locally. These labels are unauthenticated until pairing.
    pub fn inspect_ticket(&self, ticket: String) -> Result<NativeHost, NativeError> {
        let ticket = Zeroizing::new(ticket);
        let (fields, _secret) = zterm_proto::decode_pair_ticket(ticket.trim())
            .map_err(|_| crate::terminal::failure("invalid_ticket"))?;
        Ok(NativeHost {
            device_id: fields.host_device_id().to_string(),
            name: fields.host_name().to_owned(),
            relay_urls: Vec::new(),
        })
    }
    /// Initializes one endpoint only after Android has durably loaded or saved its seed.
    pub async fn initialize(
        &self,
        seed: Vec<u8>,
        dns_servers: Vec<NativeDnsServer>,
        hosts: Vec<NativeHost>,
    ) -> Result<String, NativeError> {
        let seed = Zeroizing::new(seed);
        let network = Arc::clone(&self.network);
        self.on_executor(async move {
            let bytes: [u8; 32] =
                seed.as_slice()
                    .try_into()
                    .map_err(|_| NativeError::RequestFailed {
                        code: "identity_invalid".to_owned(),
                    })?;
            let bytes = Zeroizing::new(bytes);
            let secret = iroh::SecretKey::from_bytes(&bytes);
            let expected = DeviceId::from_array(*secret.public().as_bytes());
            let initialized = network
                .get_or_try_init(|| async move {
                    let dns = AndroidDns::new(dns_servers)?;
                    let endpoint = endpoint_builder(secret, dns.resolver())
                        // Android virtual/network drivers can accept GSO without
                        // delivering batched datagrams. Ordinary UDP avoids that
                        // black hole; auth, framing and QUIC recovery are unchanged.
                        .transport_config(
                            iroh::endpoint::QuicTransportConfig::builder()
                                .enable_segmentation_offload(false)
                                .build(),
                        )
                        .bind()
                        .await
                        .map_err(|_| NativeError::RequestFailed {
                            code: "transport_unavailable".to_owned(),
                        })?;
                    let controller = IrohController::new(endpoint, "Zterm Android")?;
                    for host in hosts {
                        controller.remember_host(
                            parse_device(&host.device_id)?,
                            parse_routes(host.relay_urls)?,
                        )?;
                    }
                    Ok::<_, NativeError>(Network { controller, dns })
                })
                .await?;
            if initialized.controller.device_id() != expected {
                return Err(NativeError::RequestFailed {
                    code: "identity_state_mismatch".to_owned(),
                });
            }
            Ok(expected.to_string())
        })
        .await
    }
    /// Updates platform DNS after an actual network change, keeping the same endpoint.
    pub async fn update_dns(&self, servers: Vec<NativeDnsServer>) -> Result<(), NativeError> {
        let network = Arc::clone(&self.network);
        self.on_executor(async move {
            let network = require_network(&network)?;
            network.dns.update(servers)?;
            network.controller.network_change().await;
            Ok(())
        })
        .await
    }
    /// Executes one pairing exchange on the Application-owned executor.
    pub async fn pair_ticket(&self, ticket: String) -> Result<Arc<NativePairing>, NativeError> {
        let ticket = Zeroizing::new(ticket);
        let network = Arc::clone(&self.network);
        self.on_executor(async move {
            let paired = require_network(&network)?
                .controller
                .pair_ticket(ticket.trim())
                .await?;
            let host = NativeHost {
                device_id: paired.device_id().to_string(),
                name: paired.name().to_owned(),
                relay_urls: paired
                    .verified_relay()
                    .map(|route| vec![route.as_str().to_owned()])
                    .unwrap_or_default(),
            };
            Ok(Arc::new(NativePairing {
                host,
                paired: Mutex::new(Some(paired)),
            }))
        })
        .await
    }
    /// Publishes an already durably stored pairing; repeated commit is idempotent.
    pub fn commit_pairing(&self, pairing: Arc<NativePairing>) -> Result<(), NativeError> {
        let network = require_network(&self.network)?;
        if let Some(paired) = pairing
            .paired
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            network.controller.commit_pair(paired)?;
        }
        Ok(())
    }
    /// Removes local transport state only after Android's address-book write succeeds.
    pub async fn forget_host(&self, host: String) -> Result<(), NativeError> {
        let remote = parse_device(&host)?;
        let network = Arc::clone(&self.network);
        self.on_executor(async move {
            require_network(&network)?
                .controller
                .forget_host(remote)
                .await;
            Ok(())
        })
        .await
    }
    /// Reads transport counters without dialing or changing attachment state.
    pub async fn connection_info(
        &self,
        host: String,
    ) -> Result<Option<NativeConnectionInfo>, NativeError> {
        let remote = parse_device(&host)?;
        let network = Arc::clone(&self.network);
        self.on_executor(async move {
            Ok(require_network(&network)?
                .controller
                .connection_info(remote)
                .await?
                .map(|value| NativeConnectionInfo {
                    path: value.path.to_owned(),
                    rtt_ms: value.rtt_ms,
                    sent_bytes: value.sent_bytes,
                    received_bytes: value.received_bytes,
                    lost_packets: value.lost_packets,
                    closed: value.closed,
                }))
        })
        .await
    }
    /// Lists the exact host's existing Sessions without attaching, creating, or taking over.
    pub async fn list_sessions(&self, host: String) -> Result<Vec<NativeSession>, NativeError> {
        let remote = parse_device(&host)?;
        let network = Arc::clone(&self.network);
        let sessions = Arc::clone(&self.sessions);
        self.on_executor(async move {
            Ok(sessions
                .list_sessions_at(
                    &require_network(&network)?.controller,
                    ResolvedSessionTarget::device(remote),
                )
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        })
        .await
    }
    /// Creates exactly once through the shared target lease/replay owner.
    pub async fn create_session(
        &self,
        host: String,
        name: String,
        directory: Option<String>,
        viewport: crate::terminal::NativeViewport,
        dark: bool,
    ) -> Result<NativeSession, NativeError> {
        let target = ResolvedSessionTarget::device(parse_device(&host)?);
        let name = zterm_core::SessionName::new(name)
            .map_err(|_| crate::terminal::failure("invalid_session_name"))?;
        let size = viewport.size()?;
        let network = Arc::clone(&self.network);
        let sessions = Arc::clone(&self.sessions);
        self.on_executor(async move {
            Ok(sessions
                .create_session_at_with_colors(
                    &require_network(&network)?.controller,
                    target,
                    &name,
                    directory
                        .as_deref()
                        .filter(|value| !value.is_empty())
                        .map(std::path::Path::new),
                    Some(size),
                    crate::terminal::base_colors(dark),
                )
                .await?
                .into())
        })
        .await
    }
    /// Renames the exact selected row without switching attachments.
    pub async fn rename_session(
        &self,
        host: String,
        session: String,
        name: String,
    ) -> Result<NativeSession, NativeError> {
        let target = ResolvedSessionTarget::device(parse_device(&host)?);
        let id = crate::terminal::parse_session(&session)?;
        let name = zterm_core::SessionName::new(name)
            .map_err(|_| crate::terminal::failure("invalid_session_name"))?;
        let network = Arc::clone(&self.network);
        let sessions = Arc::clone(&self.sessions);
        self.on_executor(async move {
            Ok(sessions
                .rename_session_at(&require_network(&network)?.controller, target, id, &name)
                .await?
                .into())
        })
        .await
    }
    /// Ends an exact Session only after explicit UI confirmation.
    pub async fn close_session(
        &self,
        host: String,
        session: String,
    ) -> Result<NativeSession, NativeError> {
        let target = ResolvedSessionTarget::device(parse_device(&host)?);
        let id = crate::terminal::parse_session(&session)?;
        let network = Arc::clone(&self.network);
        let sessions = Arc::clone(&self.sessions);
        self.on_executor(async move {
            Ok(sessions
                .close_session_at(&require_network(&network)?.controller, target, id)
                .await?
                .into())
        })
        .await
    }
}
pub(crate) fn parse_device(value: &str) -> Result<DeviceId, NativeError> {
    value.parse().map_err(|_| NativeError::RequestFailed {
        code: "identity_invalid".to_owned(),
    })
}
pub(crate) fn require_network(
    network: &tokio::sync::OnceCell<Network>,
) -> Result<&Network, NativeError> {
    network.get().ok_or(NativeError::RuntimeUnavailable)
}
fn parse_routes(values: Vec<String>) -> Result<Vec<RelayHint>, NativeError> {
    values
        .into_iter()
        .map(|value| {
            RelayHint::new(value).map_err(|_| NativeError::RequestFailed {
                code: "address_unavailable".to_owned(),
            })
        })
        .collect()
}
