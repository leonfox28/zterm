//! Complete semantic surface ownership shared by desktop and native renderers.

use zterm_core::Revision;
use zterm_core::terminal::{
    ActiveScreen, TerminalModes, TerminalSurface, TerminalSurfaceDelta, TerminalSurfaceError,
    TerminalSurfaceSnapshot,
};

/// Latest complete, validated semantic state for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachmentSurface {
    revision: Revision,
    /// The complete semantic viewport at `revision()`.
    pub surface: TerminalSurface,
}

impl AttachmentSurface {
    /// Validates a full replacement before retaining it.
    pub fn from_snapshot(snapshot: &TerminalSurfaceSnapshot) -> Result<Self, TerminalSurfaceError> {
        snapshot.validate()?;
        Ok(Self {
            revision: snapshot.revision,
            surface: snapshot.surface.clone(),
        })
    }

    /// Builds a complete successor; a revision gap requires synchronization.
    /// The current surface is unchanged until the caller commits the candidate.
    pub fn candidate_after_delta(
        &self,
        delta: &TerminalSurfaceDelta,
    ) -> Result<Option<Self>, TerminalSurfaceError> {
        if self.revision != delta.from_revision {
            return Ok(None);
        }
        let surface = delta.candidate(self.revision, &self.surface)?;
        Ok(Some(Self {
            revision: delta.to_revision,
            surface,
        }))
    }

    /// Revision of this complete surface.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Screen mode declared by the host.
    #[must_use]
    pub const fn active_screen(&self) -> ActiveScreen {
        self.surface.active_screen
    }

    /// Synchronized input modes declared by the host.
    #[must_use]
    pub const fn modes(&self) -> TerminalModes {
        self.surface.modes
    }
}
