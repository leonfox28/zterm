use zterm_core::Revision;
use zterm_core::terminal::{
    ActiveScreen, TerminalModes, TerminalSurface, TerminalSurfaceDelta, TerminalSurfaceSnapshot,
};

use super::{CliError, semantic_surface_error};

/// Latest complete, validated semantic state for one attachment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AttachmentSurface {
    revision: Revision,
    pub(super) surface: TerminalSurface,
}

impl AttachmentSurface {
    pub(super) fn from_snapshot(snapshot: &TerminalSurfaceSnapshot) -> Result<Self, CliError> {
        snapshot
            .validate()
            .map_err(|_| semantic_surface_error("semantic terminal snapshot is invalid"))?;
        Ok(Self {
            revision: snapshot.revision,
            surface: snapshot.surface.clone(),
        })
    }

    pub(super) fn candidate_after_delta(
        &self,
        delta: &TerminalSurfaceDelta,
    ) -> Result<Option<Self>, CliError> {
        if self.revision != delta.from_revision {
            return Ok(None);
        }
        let mut surface = self.surface.clone();
        delta
            .apply_to(self.revision, &mut surface)
            .map_err(|_| semantic_surface_error("semantic terminal delta is incompatible"))?;
        Ok(Some(Self {
            revision: delta.to_revision,
            surface,
        }))
    }

    pub(super) const fn revision(&self) -> Revision {
        self.revision
    }

    pub(super) const fn active_screen(&self) -> ActiveScreen {
        self.surface.active_screen
    }

    pub(super) const fn modes(&self) -> TerminalModes {
        self.surface.modes
    }
}
