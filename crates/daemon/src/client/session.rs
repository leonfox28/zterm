//! Desktop constructor facade over the shared attachment state owner.
use super::transport::UnixAttachmentConnector;
use crate::{device_directory::ResolvedSessionTarget, error::DaemonError};
use std::{
    ops::{Deref, DerefMut},
    path::Path,
    sync::Arc,
};
pub use zterm_client::session::{LocalAttachmentEvent, LocalTakeoverRetryToken};
use zterm_core::{DeviceId, SessionSelector, terminal::TerminalColorProfile};

/// Desktop Session constructors retaining the existing local API.
#[derive(Debug)]
pub struct SessionClient(zterm_client::session::SessionClient);
impl SessionClient {
    /// Attaches to the daemon-lifetime default `main` session, creating it if absent.
    pub async fn connect_main(
        socket: impl AsRef<Path>,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            UnixAttachmentConnector::new(socket.as_ref()),
            ResolvedSessionTarget::local(),
            None,
            true,
            false,
            viewport,
            TerminalColorProfile::default(),
        )
        .await
    }

    /// Attaches to an existing session selected by stable ID or exact name.
    pub async fn connect_session(
        socket: impl AsRef<Path>,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            UnixAttachmentConnector::new(socket.as_ref()),
            ResolvedSessionTarget::local(),
            Some(selector),
            false,
            takeover,
            viewport,
            TerminalColorProfile::default(),
        )
        .await
    }

    /// Opens one frontend-owned remote default view through an opaque daemon tunnel.
    #[doc(hidden)]
    pub async fn connect_remote_main(
        socket: impl AsRef<Path>,
        target: DeviceId,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            UnixAttachmentConnector::new(socket.as_ref()),
            ResolvedSessionTarget::device(target),
            None,
            true,
            false,
            viewport,
            TerminalColorProfile::default(),
        )
        .await
    }

    /// Opens one frontend-owned remote named/ID view through an opaque daemon tunnel.
    #[doc(hidden)]
    pub async fn connect_remote_session(
        socket: impl AsRef<Path>,
        target: DeviceId,
        selector: SessionSelector,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            UnixAttachmentConnector::new(socket.as_ref()),
            ResolvedSessionTarget::device(target),
            Some(selector),
            false,
            takeover,
            viewport,
            TerminalColorProfile::default(),
        )
        .await
    }

    pub(crate) async fn connect_resolved_with_colors(
        connector: UnixAttachmentConnector,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
        base_colors: TerminalColorProfile,
    ) -> Result<Self, DaemonError> {
        Self::connect_inner(
            connector,
            target,
            selector,
            create_main,
            takeover,
            viewport,
            base_colors,
        )
        .await
    }

    async fn connect_inner(
        connector: UnixAttachmentConnector,
        target: ResolvedSessionTarget,
        selector: Option<SessionSelector>,
        create_main: bool,
        takeover: bool,
        viewport: Option<zterm_core::terminal::TerminalSize>,
        base_colors: TerminalColorProfile,
    ) -> Result<Self, DaemonError> {
        zterm_client::session::SessionClient::connect(
            Arc::new(connector),
            target,
            selector,
            create_main,
            takeover,
            viewport,
            base_colors,
        )
        .await
        .map(Self)
    }
    /// Transfers the single protocol owner into the prepared view.
    pub fn into_inner(self) -> zterm_client::session::SessionClient {
        self.0
    }
}
impl Deref for SessionClient {
    type Target = zterm_client::session::SessionClient;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl DerefMut for SessionClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
