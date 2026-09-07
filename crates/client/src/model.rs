//! Route-neutral Session identifiers and semantic summaries.
use crate::{
    error::ClientError,
    protocol::{malformed, protocol_error},
};
use std::{fmt, path::PathBuf};
use zterm_core::{DeviceId, Revision, SessionId, SessionName, terminal::TerminalSize};
use zterm_proto::v2;

/// Opaque exact Session target returned by the daemon-side resolver.
///
/// The value contains no alias. Holding it across lease allocation and retry
/// therefore cannot be retargeted by a concurrent alias rename.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResolvedSessionTarget(ResolvedSessionTargetKind);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ResolvedSessionTargetKind {
    Local,
    Device(DeviceId),
}

impl ResolvedSessionTarget {
    /// Returns whether this exact target is the current local daemon.
    #[must_use]
    pub const fn is_local(self) -> bool {
        matches!(self.0, ResolvedSessionTargetKind::Local)
    }

    /// Returns the frozen full device identity for a remote target.
    #[must_use]
    pub const fn device_id(self) -> Option<DeviceId> {
        match self.0 {
            ResolvedSessionTargetKind::Local => None,
            ResolvedSessionTargetKind::Device(device_id) => Some(device_id),
        }
    }

    /// Constructs the explicit local route.
    pub const fn local() -> Self {
        Self(ResolvedSessionTargetKind::Local)
    }

    /// Freezes a validated known-host identity; aliases must already be resolved.
    pub const fn device(device_id: DeviceId) -> Self {
        Self(ResolvedSessionTargetKind::Device(device_id))
    }
}

/// Current user-visible state of one live session.
#[derive(Clone, Eq, PartialEq)]
pub struct SessionSummary {
    /// Stable daemon-lifetime identity.
    pub session_id: SessionId,
    /// Current unique name.
    pub name: SessionName,
    /// Latest host terminal revision.
    pub revision: Revision,
    /// Whether an attachment owns controller input.
    pub has_controller: bool,
    /// Validated working directory used to start the login shell.
    pub working_directory: PathBuf,
    /// Last accepted terminal viewport.
    pub viewport: TerminalSize,
}

impl fmt::Debug for SessionSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionSummary")
            .field("session_id", &self.session_id)
            .field("name", &self.name)
            .field("revision", &self.revision)
            .field("has_controller", &self.has_controller)
            .field("working_directory", &"[REDACTED]")
            .field(
                "working_directory_len",
                &self.working_directory.as_os_str().len(),
            )
            .field("viewport", &self.viewport)
            .finish()
    }
}

/// Decodes a validated semantic Session summary without host filesystem access.
pub fn session_summary_from_wire(
    summary: v2::SessionSummary,
) -> Result<SessionSummary, ClientError> {
    let session_id = summary
        .session_id
        .ok_or_else(|| malformed("session summary omitted session_id"))?
        .try_into()
        .map_err(protocol_error)?;
    let name = SessionName::new(summary.name).map_err(|error| malformed(error.to_string()))?;
    let viewport = summary
        .viewport
        .ok_or_else(|| malformed("session summary omitted viewport"))?
        .try_into()
        .map_err(protocol_error)?;
    Ok(SessionSummary {
        session_id,
        name,
        revision: Revision::new(summary.revision),
        has_controller: summary.has_controller,
        working_directory: PathBuf::from(summary.working_directory),
        viewport,
    })
}
