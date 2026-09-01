//! Typed local-daemon lifecycle, pairing, and device-service dispatch.

#[cfg(unix)]
use std::future::Future;
#[cfg(unix)]
use std::pin::Pin;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use zeroize::{Zeroize, Zeroizing};
use zterm_core::DeviceId;
#[cfg(unix)]
use zterm_core::{
    AuthorizationSnapshot, AuthorizationStatus, Capabilities, DeviceAlias, DeviceSummary,
    DomainErrorKind,
};
#[cfg(unix)]
use zterm_proto::{DecodedFrame, WireKind, encode_message, v1};

#[cfg(unix)]
use crate::authorization::AuthorizationRegistry;
use crate::bootstrap::BootstrapResult;
#[cfg(unix)]
use crate::config::{ValidatedConfig, validate_setup_profile};
#[cfg(unix)]
use crate::connection_broker::{
    ConnectionBroker, ConnectionCloseReason, PeerConnectionObservation,
};
#[cfg(unix)]
use crate::device_directory::{DeviceDirectory, DeviceProjection};
#[cfg(unix)]
use crate::error::DaemonError;
use crate::network::NetworkObservation;
#[cfg(unix)]
use crate::network::NetworkObserver;
#[cfg(unix)]
use crate::pairing_service::{LocalPairAcceptInput, LocalPairCreateInput, PairingService};
#[cfg(unix)]
use crate::remote_session::RemoteSessionService;
#[cfg(unix)]
use crate::session::{SessionService, SessionSummary};
#[cfg(unix)]
use crate::store::StoreHandle;

/// Protocol version projected by readiness and status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolStatus {
    /// Product wire major.
    pub wire_major: u32,
    /// Persistent-state schema supported by this binary.
    pub state_schema: u32,
    /// Negotiable capability bits, retaining future unknown values.
    pub capabilities: u64,
}

/// Successful local readiness projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonReadiness {
    /// Supported protocol values.
    pub protocol: ProtocolStatus,
    /// Running package version.
    pub version: String,
    /// Daemon process start timestamp.
    pub started_at_unix: u64,
}

/// Current daemon status shared by CLI human and JSON renderers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonStatus {
    /// Supported protocol values.
    pub protocol: ProtocolStatus,
    /// Running package version.
    pub version: String,
    /// Current implementation phase.
    pub phase: String,
    /// Stable public device identity bytes.
    pub device_id: DeviceId,
    /// Iroh's canonical public endpoint encoding.
    pub endpoint_id: String,
    /// User-facing device name.
    pub device_name: String,
    /// Selected infrastructure profile name.
    pub infrastructure_profile: String,
    /// Daemon process start timestamp.
    pub started_at_unix: u64,
    /// Live terminal sessions in the current daemon.
    pub active_session_count: u32,
    /// Live terminal session names in stable display order.
    pub active_session_names: Vec<String>,
    /// Redacted observation from the daemon-owned Endpoint and broker.
    pub network: NetworkObservation,
}

/// Result of validating setup against the running daemon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSetupStatus {
    /// Stable public device identity bytes.
    pub device_id: DeviceId,
    /// Iroh's canonical public endpoint encoding.
    pub endpoint_id: String,
}

/// Active-session impact returned by stop and update preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionImpact {
    /// Live terminal sessions affected by the operation.
    pub active_session_count: u32,
    /// Live terminal session names affected by the operation.
    pub active_session_names: Vec<String>,
    /// Whether the accepted stop request is shutting the daemon down.
    pub stopping: bool,
    /// Whether a future manual update would interrupt work.
    pub interruption_required: bool,
}

/// Redacted live transport/session counters merged into a device projection.
#[cfg(unix)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeviceLiveObservation {
    /// Whether one promoted normal-ALPN connection is active.
    pub online: bool,
    /// Current application streams owned by the remote endpoint.
    pub active_stream_count: u32,
    /// Current Session attachments owned by the remote endpoint.
    pub remote_attachment_count: u32,
}

/// Daemon-owned hook for live peer observation and immediate connection close.
///
/// The local device service depends on this narrow boundary instead of an
/// Iroh Endpoint. Isolated same-UID tests can therefore prove durable revoke
/// ordering without binding UDP, while production composition injects its
/// existing connection broker.
#[cfg(unix)]
pub trait RemoteDeviceAccess: Send + Sync {
    /// Returns redacted live counters without exposing routes or direct IPs.
    fn observe<'a>(
        &'a self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<DeviceLiveObservation, DaemonError>> + Send + 'a>>;

    /// Closes every current connection/stream for one revoked endpoint.
    fn close_remote<'a>(
        &'a self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>>;
}

/// Production projection of the existing broker and SessionService owners.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) struct BrokerRemoteDeviceAccess {
    broker: ConnectionBroker,
    sessions: SessionService,
}

#[cfg(unix)]
impl BrokerRemoteDeviceAccess {
    #[must_use]
    pub(crate) const fn new(broker: ConnectionBroker, sessions: SessionService) -> Self {
        Self { broker, sessions }
    }
}

#[cfg(unix)]
impl RemoteDeviceAccess for BrokerRemoteDeviceAccess {
    fn observe<'a>(
        &'a self,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<DeviceLiveObservation, DaemonError>> + Send + 'a>> {
        Box::pin(async move {
            let peer = self.broker.peer_observation(device_id).await;
            let sessions = self.sessions.clone();
            let remote_attachment_count = run_service_blocking_until(deadline, move || {
                sessions.remote_attachment_count_until(device_id, deadline)
            })
            .await?;
            project_device_live_observation(peer, remote_attachment_count)
        })
    }

    fn close_remote<'a>(
        &'a self,
        device_id: DeviceId,
        _deadline: Instant,
    ) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + 'a>> {
        Box::pin(async move {
            self.broker
                .close_remote(device_id, ConnectionCloseReason::Unauthorized)
                .await;
            Ok(())
        })
    }
}

/// Existing store, directory, authorization gate, and remote-close owners used
/// by same-UID device IPC.
#[cfg(unix)]
#[derive(Clone)]
pub struct DeviceManagement {
    store: StoreHandle,
    directory: DeviceDirectory,
    authorization: AuthorizationRegistry,
    remote_access: Arc<dyn RemoteDeviceAccess>,
    #[cfg(test)]
    revoke_guard_after_first_poll_for_test: Option<tokio::sync::mpsc::UnboundedSender<DeviceId>>,
}

#[cfg(unix)]
impl DeviceManagement {
    /// Composes the already-running owners without opening SQLite or binding an
    /// Endpoint. The directory should be the same instance shared with pairing
    /// so alias reservations have one owner.
    #[must_use]
    pub fn new(
        store: StoreHandle,
        directory: DeviceDirectory,
        authorization: AuthorizationRegistry,
        remote_access: Arc<dyn RemoteDeviceAccess>,
    ) -> Self {
        Self {
            store,
            directory,
            authorization,
            remote_access,
            #[cfg(test)]
            revoke_guard_after_first_poll_for_test: None,
        }
    }

    /// Installs a deterministic notification after the revoke writer's first
    /// actual lock poll proves that it has queued or acquired the fair gate.
    #[cfg(test)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_revoke_guard_after_first_poll_for_test(
        mut self,
        observer: tokio::sync::mpsc::UnboundedSender<DeviceId>,
    ) -> Self {
        self.revoke_guard_after_first_poll_for_test = Some(observer);
        self
    }
}

/// Shared lifecycle and live-session service state for one daemon process.
#[derive(Clone)]
pub struct DaemonService {
    #[cfg(unix)]
    setup: BootstrapResult,
    #[cfg(unix)]
    started_at_unix: u64,
    #[cfg(unix)]
    sessions: SessionService,
    #[cfg(unix)]
    network: NetworkObserver,
    #[cfg(unix)]
    devices: Option<DeviceManagement>,
    #[cfg(unix)]
    pairing: Option<PairingService>,
    #[cfg(unix)]
    remote_sessions: Option<RemoteSessionService>,
}

impl DaemonService {
    /// Creates service state from already validated persistent setup.
    #[cfg(unix)]
    #[must_use]
    pub fn new(setup: BootstrapResult) -> Self {
        Self::with_started_at(setup, now_unix())
    }

    /// Creates the non-Unix placeholder for the current unsupported boundary.
    #[cfg(not(unix))]
    #[must_use]
    pub fn new(_setup: BootstrapResult) -> Self {
        Self {}
    }

    /// Creates service state with an explicit timestamp for isolated tests.
    #[cfg(unix)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_started_at(setup: BootstrapResult, started_at_unix: u64) -> Self {
        let sessions = SessionService::new(setup.device_id);
        Self {
            network: NetworkObserver::disabled(setup.device_id),
            setup,
            started_at_unix,
            sessions,
            devices: None,
            pairing: None,
            remote_sessions: None,
        }
    }

    /// Creates isolated service state around a task-private session service.
    #[cfg(unix)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_sessions(
        setup: BootstrapResult,
        started_at_unix: u64,
        sessions: SessionService,
    ) -> Self {
        Self {
            network: NetworkObserver::disabled(setup.device_id),
            setup,
            started_at_unix,
            sessions,
            devices: None,
            pairing: None,
            remote_sessions: None,
        }
    }

    /// Creates production service state around the prepared network observer.
    #[cfg(unix)]
    #[must_use]
    pub fn with_network(setup: BootstrapResult, network: NetworkObserver) -> Self {
        let sessions = SessionService::new(setup.device_id);
        Self {
            setup,
            started_at_unix: now_unix(),
            sessions,
            network,
            devices: None,
            pairing: None,
            remote_sessions: None,
        }
    }

    /// Creates isolated session state with an explicit network observer.
    #[cfg(unix)]
    #[doc(hidden)]
    #[must_use]
    pub fn with_sessions_and_network(
        setup: BootstrapResult,
        started_at_unix: u64,
        sessions: SessionService,
        network: NetworkObserver,
    ) -> Self {
        Self {
            setup,
            started_at_unix,
            sessions,
            network,
            devices: None,
            pairing: None,
            remote_sessions: None,
        }
    }

    /// Adds same-UID device management around already-running daemon owners.
    #[cfg(unix)]
    #[must_use]
    pub fn with_device_management(mut self, devices: DeviceManagement) -> Self {
        self.devices = Some(devices);
        self
    }

    /// Adds the one runtime pairing coordinator shared with the pair-ALPN
    /// network callback.
    #[cfg(unix)]
    #[must_use]
    pub fn with_pairing(mut self, pairing: PairingService) -> Self {
        self.pairing = Some(pairing);
        self
    }

    /// Adds exact target resolution and outbound Session unary forwarding.
    #[cfg(unix)]
    #[must_use]
    pub(crate) fn with_remote_sessions(mut self, remote_sessions: RemoteSessionService) -> Self {
        self.remote_sessions = Some(remote_sessions);
        self
    }

    #[cfg(unix)]
    pub(crate) fn pairing(&self) -> Option<&PairingService> {
        self.pairing.as_ref()
    }

    #[cfg(unix)]
    pub(crate) const fn sessions(&self) -> &SessionService {
        &self.sessions
    }

    #[cfg(unix)]
    pub(crate) fn remote_sessions(&self) -> Option<&RemoteSessionService> {
        self.remote_sessions.as_ref()
    }

    /// Creates the non-Unix placeholder for isolated cross-platform callers.
    #[cfg(not(unix))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_started_at(_setup: BootstrapResult, _started_at_unix: u64) -> Self {
        Self {}
    }

    /// Dispatches without blocking the daemon runtime thread.
    #[cfg(unix)]
    pub(crate) async fn dispatch_until(
        &self,
        frame: DecodedFrame,
        deadline: Instant,
    ) -> ServiceReply {
        if matches!(
            frame.kind,
            WireKind::LocalPairCreateRequest | WireKind::LocalPairAcceptRequest
        ) {
            return self.dispatch_pair_until(frame, deadline).await;
        }
        if matches!(
            frame.kind,
            WireKind::LocalDeviceListRequest
                | WireKind::LocalDeviceRenameRequest
                | WireKind::LocalDeviceRevokeRequest
        ) {
            return self.dispatch_device_until(frame, deadline).await;
        }
        if frame.kind == WireKind::LocalSessionUnaryRequest {
            return self.dispatch_remote_session_until(frame, deadline).await;
        }
        let request_id = frame.request_id;
        let service = self.clone();
        let result = tokio::task::spawn_blocking(move || service.dispatch_inner(frame, deadline))
            .await
            .map_err(|error| {
                DaemonError::new(
                    DomainErrorKind::Cancelled,
                    format!("local service worker ended unexpectedly: {error}"),
                )
            })
            .and_then(|result| result);
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    async fn dispatch_remote_session_until(
        &self,
        mut frame: DecodedFrame,
        deadline: Instant,
    ) -> ServiceReply {
        let request_id = frame.request_id;
        let request = decode_request::<v1::LocalSessionUnaryRequest>(&frame);
        frame.payload.zeroize();
        let result = async {
            let mut request = request?;
            let target: DeviceId = request
                .target_device_id
                .take()
                .ok_or_else(|| {
                    DaemonError::new(
                        DomainErrorKind::MalformedFrame,
                        "local remote-Session envelope omitted target_device_id",
                    )
                })?
                .try_into()
                .map_err(protocol_error)?;
            let bytes = Zeroizing::new(std::mem::take(&mut request.frame));
            let remote_sessions = self.remote_sessions.as_ref().ok_or_else(|| {
                DaemonError::new(
                    DomainErrorKind::TransportUnavailable,
                    "remote Session transport is not composed into this daemon",
                )
            })?;
            let response = remote_sessions
                .forward_preencoded(target, request_id, &bytes, deadline)
                .await?;
            if response.request_id != request_id {
                return Err(DaemonError::new(
                    DomainErrorKind::MalformedFrame,
                    "local forwarding envelope request_id differs from the inner Session request",
                ));
            }
            ServiceReply::decoded(response)
        }
        .await;
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    async fn dispatch_pair_until(
        &self,
        mut frame: DecodedFrame,
        deadline: Instant,
    ) -> ServiceReply {
        let request_id = frame.request_id;
        let Some(pairing) = self.pairing.clone() else {
            frame.payload.zeroize();
            return ServiceReply::error(
                request_id,
                &DaemonError::new(
                    DomainErrorKind::ServiceNotImplemented,
                    "local pairing is not composed into this daemon",
                ),
            );
        };
        let result = match frame.kind {
            WireKind::LocalPairCreateRequest => {
                let request = decode_request::<v1::LocalPairCreateRequest>(&frame);
                frame.payload.zeroize();
                match request {
                    Ok(request) => {
                        self.pair_create_reply(request_id, pairing, request, deadline)
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            WireKind::LocalPairAcceptRequest => {
                let request = decode_request::<v1::LocalPairAcceptRequest>(&frame);
                frame.payload.zeroize();
                match request {
                    Ok(request) => {
                        self.pair_accept_reply(request_id, pairing, request, deadline)
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            _ => unreachable!("pair dispatcher is selected from the wire kind above"),
        };
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    async fn pair_create_reply(
        &self,
        request_id: u64,
        pairing: PairingService,
        request: v1::LocalPairCreateRequest,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let (operation_id, fingerprint) = zterm_proto::validate_pair_operation(
            &request.ephemeral_operation_id,
            &request.fingerprint,
        )
        .map_err(pair_operation_wire_error)?;
        let input =
            LocalPairCreateInput::new(operation_id, fingerprint, u64::from(request.ttl_seconds));
        let created =
            run_service_blocking_until(deadline, move || pairing.create_until(input, deadline))
                .await?;
        let mut message = v1::LocalPairCreateResponse {
            ticket: created.ticket().expose().to_owned(),
        };
        let reply = ServiceReply::message(
            WireKind::LocalPairCreateResponse,
            request_id,
            &message,
            false,
        );
        message.ticket.zeroize();
        reply
    }

    #[cfg(unix)]
    async fn pair_accept_reply(
        &self,
        request_id: u64,
        pairing: PairingService,
        mut request: v1::LocalPairAcceptRequest,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        // Generated protobuf strings do not zeroize on drop. Transfer the
        // bearer text into an RAII scrubber before any validation can return.
        let mut ticket = Zeroizing::new(std::mem::take(&mut request.ticket));
        let Some(devices) = self.devices.as_ref() else {
            return Err(DaemonError::new(
                DomainErrorKind::ServiceNotImplemented,
                "local device management is required for pair acceptance",
            ));
        };
        let (operation_id, fingerprint) = zterm_proto::validate_pair_operation(
            &request.ephemeral_operation_id,
            &request.fingerprint,
        )
        .map_err(pair_operation_wire_error)?;
        let explicit_alias = if request.alias.is_empty() {
            None
        } else {
            Some(DeviceAlias::new(request.alias).map_err(|error| {
                DaemonError::new(DomainErrorKind::InvalidDeviceAlias, error.to_string())
            })?)
        };
        let input = LocalPairAcceptInput::new(
            operation_id,
            fingerprint,
            std::mem::take(&mut *ticket),
            explicit_alias,
        );
        let accepted = await_service_until(
            deadline,
            pairing.accept_until(input, deadline),
            "accepting pairing ticket",
        )
        .await??;
        let device = self
            .device_summary(devices, accepted.device_id(), deadline)
            .await?;
        ServiceReply::message(
            WireKind::LocalPairAcceptResponse,
            request_id,
            &v1::LocalPairAcceptResponse {
                device: Some((&device).into()),
            },
            false,
        )
    }

    #[cfg(unix)]
    async fn dispatch_device_until(&self, frame: DecodedFrame, deadline: Instant) -> ServiceReply {
        let request_id = frame.request_id;
        let Some(devices) = self.devices.clone() else {
            return ServiceReply::error(
                request_id,
                &DaemonError::new(
                    DomainErrorKind::ServiceNotImplemented,
                    "local device management is not composed into this daemon",
                ),
            );
        };
        let result = match frame.kind {
            WireKind::LocalDeviceListRequest => {
                let request: Result<v1::LocalDeviceListRequest, _> = decode_request(&frame);
                match request {
                    Ok(_) => self.device_list_reply(request_id, &devices, deadline).await,
                    Err(error) => Err(error),
                }
            }
            WireKind::LocalDeviceRenameRequest => {
                self.device_rename_reply(request_id, frame, &devices, deadline)
                    .await
            }
            WireKind::LocalDeviceRevokeRequest => {
                self.device_revoke_reply(request_id, frame, &devices, deadline)
                    .await
            }
            _ => unreachable!("device dispatcher is selected from the wire kind above"),
        };
        match result {
            Ok(reply) => reply,
            Err(error) => ServiceReply::error(request_id, &error),
        }
    }

    #[cfg(unix)]
    async fn device_list_reply(
        &self,
        request_id: u64,
        devices: &DeviceManagement,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let devices = self.device_summaries(devices, deadline).await?;
        ServiceReply::message(
            WireKind::LocalDeviceListResponse,
            request_id,
            &v1::LocalDeviceListResponse {
                devices: devices.iter().map(Into::into).collect(),
            },
            false,
        )
    }

    #[cfg(unix)]
    async fn device_rename_reply(
        &self,
        request_id: u64,
        frame: DecodedFrame,
        devices: &DeviceManagement,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let request: v1::LocalDeviceRenameRequest = decode_request(&frame)?;
        let (device_id, alias) = request.try_into().map_err(device_wire_error)?;
        let directory = devices.directory.clone();
        run_service_blocking_until(deadline, move || {
            directory.rename(device_id, alias, deadline)
        })
        .await?;
        let device = self.device_summary(devices, device_id, deadline).await?;
        ServiceReply::message(
            WireKind::LocalDeviceRenameResponse,
            request_id,
            &v1::LocalDeviceRenameResponse {
                device: Some((&device).into()),
            },
            false,
        )
    }

    #[cfg(unix)]
    async fn device_revoke_reply(
        &self,
        request_id: u64,
        frame: DecodedFrame,
        devices: &DeviceManagement,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let request: v1::LocalDeviceRevokeRequest = decode_request(&frame)?;
        let device_id = request.try_into().map_err(device_wire_error)?;

        // The write permit is held across the complete ordered revoke. A
        // queued sensitive commit cannot overtake it, while an already-held
        // read permit completes before the durable transaction begins.
        #[cfg(test)]
        let mut guard = match &devices.revoke_guard_after_first_poll_for_test {
            Some(observer) => {
                devices
                    .authorization
                    .revoke_guard_after_first_poll_for_test(device_id, observer)
                    .await?
            }
            None => devices.authorization.revoke_guard(device_id).await?,
        };
        #[cfg(not(test))]
        let mut guard = devices.authorization.revoke_guard(device_id).await?;
        let store = devices.store.clone();
        let revoked_at_unix = now_unix_i64();
        let generation = store
            .run_blocking_until(deadline, move |store, deadline| {
                store.revoke(device_id, revoked_at_unix, deadline)
            })
            .await?;
        guard.publish(AuthorizationSnapshot {
            status: AuthorizationStatus::Revoked,
            generation,
        })?;

        await_service_until(
            deadline,
            devices.remote_access.close_remote(device_id, deadline),
            "closing revoked device connections",
        )
        .await??;
        let sessions = self.sessions.clone();
        run_service_blocking_until(deadline, move || {
            sessions
                .detach_remote_principal_until(device_id, deadline)
                .map(|_| ())
        })
        .await?;
        drop(guard);

        let device = self.device_summary(devices, device_id, deadline).await?;
        ServiceReply::message(
            WireKind::LocalDeviceRevokeResponse,
            request_id,
            &v1::LocalDeviceRevokeResponse {
                device: Some((&device).into()),
            },
            false,
        )
    }

    #[cfg(unix)]
    async fn device_summary(
        &self,
        devices: &DeviceManagement,
        device_id: DeviceId,
        deadline: Instant,
    ) -> Result<DeviceSummary, DaemonError> {
        self.device_summaries(devices, deadline)
            .await?
            .into_iter()
            .find(|device| device.device_id() == device_id)
            .ok_or_else(device_not_found)
    }

    #[cfg(unix)]
    async fn device_summaries(
        &self,
        devices: &DeviceManagement,
        deadline: Instant,
    ) -> Result<Vec<DeviceSummary>, DaemonError> {
        let directory = devices.directory.clone();
        let projections =
            run_service_blocking_until(deadline, move || directory.list(deadline)).await?;
        let mut summaries = Vec::with_capacity(projections.len());
        for projection in projections {
            let live = await_service_until(
                deadline,
                devices
                    .remote_access
                    .observe(projection.device_id, deadline),
                "reading live device observation",
            )
            .await??;
            summaries.push(device_summary(projection, live)?);
        }
        Ok(summaries)
    }

    #[cfg(unix)]
    fn dispatch_inner(
        &self,
        frame: DecodedFrame,
        deadline: Instant,
    ) -> Result<ServiceReply, DaemonError> {
        let request_id = frame.request_id;
        match frame.kind {
            WireKind::LocalReadinessRequest => {
                let _: v1::LocalReadinessRequest = decode_request(&frame)?;
                ServiceReply::message(
                    WireKind::LocalReadinessResponse,
                    request_id,
                    &v1::LocalReadinessResponse {
                        protocol: Some(protocol_proto()),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        started_at_unix: self.started_at_unix,
                    },
                    false,
                )
            }
            WireKind::LocalStatusRequest => {
                let _: v1::LocalStatusRequest = decode_request(&frame)?;
                let sessions = self.sessions.list()?;
                let active_session_names = session_names(&sessions);
                let network = self.network.snapshot();
                ServiceReply::message(
                    WireKind::LocalStatusResponse,
                    request_id,
                    &v1::LocalStatusResponse {
                        protocol: Some(protocol_proto()),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        phase: zterm_core::PHASE_NAME.to_owned(),
                        device_id: Some(self.setup.device_id.into()),
                        endpoint_id: self.setup.endpoint_id.clone(),
                        device_name: self.setup.config.device_name.clone(),
                        infrastructure_profile: self
                            .setup
                            .config
                            .infrastructure
                            .profile_name()
                            .to_owned(),
                        started_at_unix: self.started_at_unix,
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names,
                        network_state: network.state.as_str().to_owned(),
                        endpoint_bound: network.endpoint_bound,
                        network_bind_attempts: network.bind_attempts,
                        home_relay: network.home_relay.unwrap_or_default(),
                        address_publish_state: network.publish.as_str().to_owned(),
                        address_lookup_state: network.lookup.as_str().to_owned(),
                        authenticated_connection_count: network.authenticated_connection_count,
                        primary_connection_count: network.primary_connection_count,
                        active_stream_count: network.active_stream_count,
                        direct_path_count: network.direct_path_count,
                        relay_path_count: network.relay_path_count,
                        network_diagnostic: network
                            .diagnostic
                            .map_or_else(String::new, |diagnostic| diagnostic.code().to_owned()),
                    },
                    false,
                )
            }
            WireKind::LocalValidateSetupRequest => {
                let request: v1::LocalValidateSetupRequest = decode_request(&frame)?;
                let requested = config_from_wire(&request)?;
                if requested != self.setup.config {
                    return Err(DaemonError::new(
                        DomainErrorKind::AlreadyConfiguredConflict,
                        "requested setup differs from the running daemon configuration",
                    ));
                }
                ServiceReply::message(
                    WireKind::LocalValidateSetupResponse,
                    request_id,
                    &v1::LocalValidateSetupResponse {
                        device_id: Some(self.setup.device_id.into()),
                        endpoint_id: self.setup.endpoint_id.clone(),
                    },
                    false,
                )
            }
            WireKind::LocalStopRequest => {
                let _: v1::LocalStopRequest = decode_request(&frame)?;
                let sessions = self.sessions.shutdown_until(deadline)?;
                ServiceReply::message(
                    WireKind::LocalStopResponse,
                    request_id,
                    &v1::LocalStopResponse {
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names: session_names(&sessions),
                        stopping: true,
                    },
                    true,
                )
            }
            WireKind::LocalUpdatePreflightRequest => {
                let _: v1::LocalUpdatePreflightRequest = decode_request(&frame)?;
                let sessions = self.sessions.list()?;
                ServiceReply::message(
                    WireKind::LocalUpdatePreflightResponse,
                    request_id,
                    &v1::LocalUpdatePreflightResponse {
                        active_session_count: u32::try_from(sessions.len()).unwrap_or(u32::MAX),
                        active_session_names: session_names(&sessions),
                        interruption_required: !sessions.is_empty(),
                    },
                    false,
                )
            }
            WireKind::LocalTargetResolveRequest => {
                let request: v1::LocalTargetResolveRequest = decode_request(&frame)?;
                let target = if request.selector == zterm_core::RESERVED_DEVICE_ALIAS {
                    crate::device_directory::ResolvedSessionTarget::local()
                } else {
                    self.remote_sessions
                        .as_ref()
                        .ok_or_else(|| {
                            DaemonError::new(
                                DomainErrorKind::TransportUnavailable,
                                "remote target resolution is unavailable in this daemon",
                            )
                        })?
                        .resolve(&request.selector, deadline)?
                };
                ServiceReply::message(
                    WireKind::LocalTargetResolveResponse,
                    request_id,
                    &v1::LocalTargetResolveResponse {
                        target: Some(resolved_target_wire(target)),
                    },
                    false,
                )
            }
            _ => Err(DaemonError::new(
                DomainErrorKind::ServiceNotImplemented,
                format!(
                    "wire service {:?} is not implemented by this daemon",
                    frame.kind
                ),
            )),
        }
    }
}

#[cfg(unix)]
fn device_summary(
    projection: DeviceProjection,
    live: DeviceLiveObservation,
) -> Result<DeviceSummary, DaemonError> {
    let outbound_known = projection.remote_name.is_some();
    let remote_name = projection
        .remote_name
        .as_ref()
        .map_or_else(String::new, |name| name.as_str().to_owned());
    DeviceSummary::new(
        projection.device_id,
        outbound_known,
        projection.alias,
        remote_name,
        projection.route_verified,
        projection.auth.status,
        projection.auth.generation,
        optional_timestamp(projection.paired_at_unix, "device pairing timestamp")?,
        optional_timestamp(projection.last_seen_at_unix, "device last-seen timestamp")?,
        live.online,
        live.active_stream_count,
        live.remote_attachment_count,
    )
    .map_err(|error| {
        DaemonError::new(
            DomainErrorKind::StoreUnavailable,
            format!("stored device projection is inconsistent: {error}"),
        )
    })
}

#[cfg(unix)]
fn project_device_live_observation(
    peer: PeerConnectionObservation,
    remote_attachment_count: usize,
) -> Result<DeviceLiveObservation, DaemonError> {
    Ok(DeviceLiveObservation {
        online: peer.primary.is_some(),
        active_stream_count: peer.active_stream_count,
        remote_attachment_count: bounded_device_count(remote_attachment_count),
    })
}

#[cfg(unix)]
fn bounded_device_count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(unix)]
fn optional_timestamp(value: Option<i64>, field: &str) -> Result<u64, DaemonError> {
    value.map_or(Ok(0), |value| {
        u64::try_from(value).map_err(|_| {
            DaemonError::new(
                DomainErrorKind::StoreUnavailable,
                format!("{field} must not be negative"),
            )
        })
    })
}

#[cfg(unix)]
async fn run_service_blocking_until<R>(
    deadline: Instant,
    operation: impl FnOnce() -> Result<R, DaemonError> + Send + 'static,
) -> Result<R, DaemonError>
where
    R: Send + 'static,
{
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "local device operation deadline elapsed before dispatch",
        ));
    }
    match tokio::time::timeout(remaining, tokio::task::spawn_blocking(operation)).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(DaemonError::new(
            DomainErrorKind::Cancelled,
            format!("local device worker ended unexpectedly: {error}"),
        )),
        Err(_) => Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            "local device operation exceeded its absolute deadline",
        )),
    }
}

#[cfg(unix)]
async fn await_service_until<R>(
    deadline: Instant,
    future: impl Future<Output = R>,
    operation: &str,
) -> Result<R, DaemonError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            format!("deadline elapsed before {operation}"),
        ));
    }
    tokio::time::timeout(remaining, future).await.map_err(|_| {
        DaemonError::new(
            DomainErrorKind::DeadlineExceeded,
            format!("deadline elapsed while {operation}"),
        )
    })
}

#[cfg(unix)]
fn device_wire_error(error: zterm_proto::WireFieldError) -> DaemonError {
    let kind = if matches!(error, zterm_proto::WireFieldError::InvalidAlias(_)) {
        DomainErrorKind::InvalidDeviceAlias
    } else {
        DomainErrorKind::MalformedFrame
    };
    DaemonError::new(kind, error.to_string())
}

#[cfg(unix)]
fn pair_operation_wire_error(error: zterm_proto::PairOperationError) -> DaemonError {
    DaemonError::new(DomainErrorKind::MalformedFrame, error.to_string())
}

#[cfg(unix)]
fn device_not_found() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DeviceNotFound,
        "device projection disappeared after the requested operation",
    )
}

#[cfg(unix)]
fn now_unix_i64() -> i64 {
    i64::try_from(now_unix()).unwrap_or(i64::MAX)
}

#[cfg(unix)]
pub(crate) struct ServiceReply {
    pub(crate) bytes: Zeroizing<Vec<u8>>,
    pub(crate) stop_after_flush: bool,
}

#[cfg(unix)]
impl ServiceReply {
    pub(crate) fn message<Message>(
        kind: WireKind,
        request_id: u64,
        message: &Message,
        stop_after_flush: bool,
    ) -> Result<Self, DaemonError>
    where
        Message: prost::Message,
    {
        let bytes =
            Zeroizing::new(encode_message(kind, request_id, 0, message).map_err(protocol_error)?);
        Ok(Self {
            bytes,
            stop_after_flush,
        })
    }

    pub(crate) fn error(request_id: u64, error: &DaemonError) -> Self {
        let message = v1::ServiceError {
            code: error.kind().code().to_owned(),
            message: error.detail().to_owned(),
        };
        let bytes = Zeroizing::new(
            encode_message(WireKind::ServiceErrorResponse, request_id, 0, &message)
                .expect("bounded daemon errors always fit the service-error frame"),
        );
        Self {
            bytes,
            stop_after_flush: false,
        }
    }

    pub(crate) fn decoded(frame: DecodedFrame) -> Result<Self, DaemonError> {
        let bytes = Zeroizing::new(
            zterm_proto::encode_payload(
                frame.kind,
                frame.request_id,
                frame.deadline_ms,
                frame.payload,
            )
            .map_err(protocol_error)?,
        );
        Ok(Self {
            bytes,
            stop_after_flush: false,
        })
    }
}

#[cfg(unix)]
fn resolved_target_wire(
    target: crate::device_directory::ResolvedSessionTarget,
) -> v1::TargetSelector {
    let target = match target.device_id() {
        Some(device_id) => v1::target_selector::Target::Device(device_id.into()),
        None => v1::target_selector::Target::Local(true),
    };
    v1::TargetSelector {
        target: Some(target),
    }
}

#[cfg(unix)]
pub(crate) fn protocol_error(error: zterm_proto::ProtocolError) -> DaemonError {
    use zterm_proto::ProtocolError;
    let kind = match error {
        ProtocolError::WireMajorMismatch { .. } => DomainErrorKind::WireMajorMismatch,
        ProtocolError::UnknownKind(_) => DomainErrorKind::UnknownKind,
        ProtocolError::FrameTooLarge(_) => DomainErrorKind::FrameTooLarge,
        ProtocolError::ControlPayloadTooLarge(_) => DomainErrorKind::ControlPayloadTooLarge,
        ProtocolError::MalformedVarint
        | ProtocolError::TruncatedFrame
        | ProtocolError::MalformedProtobuf(_)
        | ProtocolError::UnexpectedKind { .. }
        | ProtocolError::InvalidIdentifier(_)
        | ProtocolError::InvalidTerminalSize { .. } => DomainErrorKind::MalformedFrame,
    };
    DaemonError::new(kind, error.to_string())
}

#[cfg(unix)]
fn decode_request<Message>(frame: &DecodedFrame) -> Result<Message, DaemonError>
where
    Message: prost::Message + Default,
{
    frame.decode_message(frame.kind).map_err(protocol_error)
}

#[cfg(unix)]
fn protocol_proto() -> v1::ProtocolVersion {
    v1::ProtocolVersion {
        wire_major: zterm_proto::WIRE_MAJOR,
        state_schema: zterm_proto::STATE_SCHEMA_VERSION,
        capabilities: Capabilities::LOCAL_LIFECYCLE
            | Capabilities::SESSION_SERVICE
            | Capabilities::TERMINAL_SERVICE
            | Capabilities::HISTORY_PAGING,
    }
}

#[cfg(unix)]
fn session_names(sessions: &[SessionSummary]) -> Vec<String> {
    sessions
        .iter()
        .map(|session| session.name.to_string())
        .collect()
}

#[cfg(unix)]
fn config_from_wire(
    request: &v1::LocalValidateSetupRequest,
) -> Result<ValidatedConfig, DaemonError> {
    let relay_url = (!request.relay_url.is_empty()).then_some(request.relay_url.as_str());
    validate_setup_profile(
        &request.device_name,
        &request.infrastructure_profile,
        relay_url,
    )
}

#[cfg(unix)]
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use zterm_core::{ConnectionAttemptId, ConnectionCandidateKey};

    #[test]
    fn live_device_projection_uses_peer_and_session_owners() {
        let peer = PeerConnectionObservation {
            primary: Some(ConnectionCandidateKey::new(
                DeviceId::from_array([0x81; 32]),
                ConnectionAttemptId::from_array([0x82; 16]),
            )),
            candidate_count: 2,
            demand_count: 3,
            remote_acceptance_generation: None,
            path: crate::network::PathKind::Relay,
            active_stream_count: 7,
        };

        assert_eq!(
            project_device_live_observation(peer, 11).expect("bounded projection"),
            DeviceLiveObservation {
                online: true,
                active_stream_count: 7,
                remote_attachment_count: 11,
            }
        );
    }
}
