use std::fmt;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use zterm_core::Revision;
use zterm_core::terminal::{
    ActiveScreen, TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalScrollMetrics,
    TerminalSize, TerminalSurfaceDelta, TerminalSurfaceDeltaResult,
    TerminalSurfaceHistoryWindowFrame, TerminalSurfaceHistoryWindowResult, TerminalSurfaceRowPatch,
    TerminalSurfaceSnapshot, TerminalUpdate,
};

use crate::engine::AlacrittyEngine;
use crate::ingress::{IngressError, TerminalIngressPolicy, UpdateCollector};
use crate::projection::{CHECKPOINT_FORMAT_VERSION, ProjectedScreen, project, project_row};

/// Opaque, content-redacted baseline for one merged latest-state delta.
#[derive(Clone)]
pub struct TerminalCheckpoint {
    revision: Revision,
    projection: ProjectedScreen,
}

impl fmt::Debug for TerminalCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalCheckpoint")
            .field("revision", &self.revision)
            .field("size", &self.projection.size)
            .field("active_screen", &self.projection.active_screen)
            .field("format_version", &self.projection.version)
            .finish_non_exhaustive()
    }
}

impl TerminalCheckpoint {
    /// Returns the exact authoritative revision represented by this baseline.
    #[doc(hidden)]
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the visible-cell capacity retained by this checkpoint.
    #[doc(hidden)]
    #[must_use]
    pub fn retained_cell_capacity(&self) -> usize {
        self.projection.retained_cell_capacity()
    }

    /// Checkpoints never retain main-screen scrollback.
    #[doc(hidden)]
    #[must_use]
    pub const fn retained_scrollback_rows(&self) -> usize {
        0
    }
}

/// Errors produced at the host terminal-model boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalError {
    /// A viewport dimension was zero.
    InvalidSize(TerminalSize),
    /// Checked grid-size arithmetic could not represent the requested model.
    AllocationOverflow {
        /// Requested viewport.
        size: TerminalSize,
        /// Requested scrollback capacity.
        scrollback_rows: usize,
    },
    /// The monotonically increasing revision reached `u64::MAX`.
    RevisionOverflow,
    /// Canonical terminal replies exceeded the per-update security bound.
    ReplyOverflow,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize(size) => write!(
                formatter,
                "terminal size must be non-zero, got {}x{}",
                size.columns, size.rows
            ),
            Self::AllocationOverflow {
                size,
                scrollback_rows,
            } => write!(
                formatter,
                "terminal allocation dimensions overflow for {}x{} with {scrollback_rows} scrollback rows",
                size.columns, size.rows
            ),
            Self::RevisionOverflow => write!(formatter, "terminal revision overflow"),
            Self::ReplyOverflow => write!(formatter, "terminal reply output exceeded its bound"),
        }
    }
}

impl std::error::Error for TerminalError {}

impl From<IngressError> for TerminalError {
    fn from(error: IngressError) -> Self {
        match error {
            IngressError::ReplyOverflow => Self::ReplyOverflow,
        }
    }
}

/// Host-authoritative Alacritty-backed terminal model.
pub struct TerminalModel {
    engine: AlacrittyEngine,
    ingress: TerminalIngressPolicy,
    revision: Revision,
    scrollback_rows: usize,
    history_epoch: Revision,
    retained_history_rows: usize,
}

impl TerminalModel {
    /// Creates a terminal with bounded main-screen scrollback.
    pub fn new(size: TerminalSize, scrollback_rows: usize) -> Result<Self, TerminalError> {
        validate_allocation(size, scrollback_rows)?;
        Ok(Self {
            engine: AlacrittyEngine::new(size, scrollback_rows),
            ingress: TerminalIngressPolicy::default(),
            revision: Revision::ZERO,
            scrollback_rows,
            history_epoch: Revision::ZERO,
            retained_history_rows: 0,
        })
    }

    /// Returns the current monotonically increasing revision.
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the current viewport size.
    #[must_use]
    pub fn size(&self) -> TerminalSize {
        self.engine.size()
    }

    /// Processes one ordered PTY byte chunk.
    pub fn ingest(&mut self, bytes: &[u8]) -> Result<TerminalUpdate, TerminalError> {
        if bytes.is_empty() {
            return Ok(TerminalUpdate {
                revision: self.revision,
                replies: Vec::new(),
                events: Vec::new(),
                host_effect: None,
            });
        }

        let next_revision = self.next_revision()?;
        let mut output = UpdateCollector::new();
        self.ingress.process(bytes, &mut self.engine, &mut output)?;
        self.revision = next_revision;
        self.refresh_history_epoch_after_ingest();
        let (replies, events, host_effect) = output.finish();
        Ok(TerminalUpdate {
            revision: self.revision,
            replies,
            events,
            host_effect,
        })
    }

    /// Resizes the viewport and advances the revision exactly once.
    pub fn resize(&mut self, size: TerminalSize) -> Result<TerminalUpdate, TerminalError> {
        let next_revision = self.preflight_resize(size)?;
        self.engine.resize(size);
        self.revision = next_revision;
        self.history_epoch = next_revision;
        if self.engine.active_screen() == ActiveScreen::Main {
            self.retained_history_rows = self.engine.term().grid().history_size();
        }
        Ok(TerminalUpdate {
            revision: self.revision,
            replies: Vec::new(),
            events: Vec::new(),
            host_effect: None,
        })
    }

    /// Validates a resize without mutating or allocating terminal state.
    pub fn preflight_resize(&self, size: TerminalSize) -> Result<Revision, TerminalError> {
        validate_allocation(size, self.scrollback_rows)?;
        self.next_revision()
    }

    /// Captures an opaque visible-screen baseline for a later merged delta.
    #[must_use]
    pub fn checkpoint(&self) -> TerminalCheckpoint {
        TerminalCheckpoint {
            revision: self.revision,
            projection: project(&self.engine),
        }
    }

    /// Captures a complete exact semantic surface without constructing ANSI.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSurfaceSnapshot {
        let projection = project(&self.engine);
        TerminalSurfaceSnapshot {
            revision: self.revision,
            surface: projection.to_surface(self.live_scroll_metrics()),
        }
    }

    /// Produces one merged semantic row update or a complete semantic replacement.
    #[must_use]
    pub fn delta_or_resync(&self, checkpoint: &TerminalCheckpoint) -> TerminalSurfaceDeltaResult {
        let latest = project(&self.engine);
        let snapshot = || TerminalSurfaceSnapshot {
            revision: self.revision,
            surface: latest.to_surface(self.live_scroll_metrics()),
        };
        if checkpoint.revision >= self.revision
            || checkpoint.projection.version != CHECKPOINT_FORMAT_VERSION
            || checkpoint.projection.size != latest.size
            || checkpoint.projection.active_screen != latest.active_screen
        {
            return TerminalSurfaceDeltaResult::Resync(snapshot());
        }

        let row_patches = checkpoint
            .projection
            .rows
            .iter()
            .zip(&latest.rows)
            .enumerate()
            .filter(|(_, (before, after))| before != after)
            .map(|(row, (_, replacement))| TerminalSurfaceRowPatch {
                row: u16::try_from(row).unwrap_or(u16::MAX),
                replacement: replacement.to_surface_row(),
            })
            .collect();
        let delta = TerminalSurfaceDelta {
            from_revision: checkpoint.revision,
            to_revision: self.revision,
            size: latest.size,
            active_screen: latest.active_screen,
            row_patches,
            cursor: latest.cursor,
            modes: latest.modes,
            scroll_metrics: self.live_scroll_metrics(),
        };
        debug_assert!(delta.validate().is_ok());
        TerminalSurfaceDeltaResult::Delta(delta)
    }

    /// Returns the live main-screen scroll extent without changing terminal state.
    #[must_use]
    pub fn live_scroll_metrics(&self) -> Option<TerminalScrollMetrics> {
        (self.engine.active_screen() == ActiveScreen::Main).then_some(TerminalScrollMetrics {
            epoch: self.history_epoch,
            revision: self.revision,
            offset_from_bottom: 0,
            max_offset_from_bottom: u64::try_from(self.retained_history_rows).unwrap_or(u64::MAX),
            viewport_rows: self.size().rows,
        })
    }

    /// Projects one semantic history window without constructing ANSI.
    #[must_use]
    pub fn history_window(
        &self,
        query: TerminalHistoryWindowQuery,
    ) -> TerminalSurfaceHistoryWindowResult {
        let current = TerminalHistoryWindowAnchor {
            epoch: self.history_epoch,
            revision: self.revision,
            max_offset_from_bottom: u64::try_from(self.retained_history_rows).unwrap_or(u64::MAX),
            viewport: self.size(),
        };
        if !query.is_valid() || query.anchor.revision > self.revision {
            return TerminalSurfaceHistoryWindowResult::HistoryGap {
                epoch: current.epoch,
                revision: current.revision,
            };
        }
        if self.engine.active_screen() != ActiveScreen::Main {
            return TerminalSurfaceHistoryWindowResult::HistoryChanged {
                epoch: current.epoch,
                revision: current.revision,
            };
        }

        let Some(shape) = query.response_shape(current) else {
            return TerminalSurfaceHistoryWindowResult::HistoryGap {
                epoch: current.epoch,
                revision: current.revision,
            };
        };
        let Ok(row_count) = i64::try_from(shape.row_count) else {
            return TerminalSurfaceHistoryWindowResult::HistoryGap {
                epoch: current.epoch,
                revision: current.revision,
            };
        };
        let Some(end_row_exclusive) = shape.first_row_from_live_top.checked_add(row_count) else {
            return TerminalSurfaceHistoryWindowResult::HistoryGap {
                epoch: current.epoch,
                revision: current.revision,
            };
        };
        let Some(lines) = (shape.first_row_from_live_top..end_row_exclusive)
            .map(|line| i32::try_from(line).ok())
            .collect::<Option<Vec<_>>>()
        else {
            return TerminalSurfaceHistoryWindowResult::HistoryGap {
                epoch: current.epoch,
                revision: current.revision,
            };
        };
        let rows = lines
            .into_iter()
            .map(|line| project_row(&self.engine, Line(line)).to_surface_row())
            .collect();
        let frame = TerminalSurfaceHistoryWindowFrame {
            disposition: shape.disposition,
            anchor: current,
            target_offset_from_bottom: shape.target_offset_from_bottom,
            first_row_from_live_top: shape.first_row_from_live_top,
            rows,
        };
        debug_assert!(frame.validate_for(query).is_ok());
        TerminalSurfaceHistoryWindowResult::Frame(frame)
    }

    fn next_revision(&self) -> Result<Revision, TerminalError> {
        self.revision
            .checked_next()
            .ok_or(TerminalError::RevisionOverflow)
    }

    fn refresh_history_epoch_after_ingest(&mut self) {
        if self.engine.active_screen() != ActiveScreen::Main {
            return;
        }
        let retained = self.engine.term().grid().history_size();
        let reached_capacity = self.scrollback_rows > 0
            && retained == self.scrollback_rows
            && self.retained_history_rows <= retained;
        if retained < self.retained_history_rows || reached_capacity {
            self.history_epoch = self.revision;
        }
        self.retained_history_rows = retained;
    }
}

fn validate_allocation(size: TerminalSize, scrollback_rows: usize) -> Result<(), TerminalError> {
    if size.rows == 0 || size.columns == 0 {
        return Err(TerminalError::InvalidSize(size));
    }
    let columns = usize::from(size.columns);
    let visible = usize::from(size.rows).checked_mul(columns);
    let history = scrollback_rows.checked_mul(columns);
    let representable = visible
        .and_then(|cells| cells.checked_mul(2))
        .and_then(|cells| history.and_then(|history| cells.checked_add(history)))
        .is_some()
        && scrollback_rows <= i32::MAX as usize;
    if !representable {
        return Err(TerminalError::AllocationOverflow {
            size,
            scrollback_rows,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_overflow_never_mutates_terminal_state() {
        let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("valid model");
        model.revision = Revision::new(u64::MAX);
        let before = model.snapshot();
        assert_eq!(
            model.ingest(b"not rendered"),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.snapshot(), before);
        assert_eq!(
            model.resize(TerminalSize::new(3, 9)),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.snapshot(), before);
    }

    #[test]
    fn invalid_and_overflowing_dimensions_are_rejected_before_allocation() {
        assert_eq!(
            TerminalModel::new(TerminalSize::new(0, 8), 0).err(),
            Some(TerminalError::InvalidSize(TerminalSize::new(0, 8)))
        );
        assert!(matches!(
            TerminalModel::new(TerminalSize::new(1, 2), usize::MAX).err(),
            Some(TerminalError::AllocationOverflow { .. })
        ));
    }

    #[test]
    fn dropping_model_releases_engine_while_checkpoint_remains_engine_free() {
        let model = TerminalModel::new(TerminalSize::new(2, 8), 4).expect("valid model");
        let probe = model.engine.lifecycle_probe();
        let checkpoint = model.checkpoint();
        assert!(probe.upgrade().is_some());
        drop(model);
        assert!(probe.upgrade().is_none());
        assert_eq!(checkpoint.retained_scrollback_rows(), 0);
        assert_eq!(checkpoint.retained_cell_capacity(), 16);
    }

    #[test]
    fn semantic_snapshot_delta_replay_matches_one_fresh_projection() {
        let mut model = TerminalModel::new(TerminalSize::new(3, 8), 8).expect("semantic model");
        model
            .ingest("old\r\n\x1b[31m界\x1b[0m".as_bytes())
            .expect("seed semantic surface");
        let checkpoint = model.checkpoint();
        let mut applied = model.snapshot();
        applied.validate().expect("valid semantic snapshot");

        model
            .ingest(b"\x1b[2;4H!\x1b[3;8H#")
            .expect("change multiple exact cells");
        let TerminalSurfaceDeltaResult::Delta(delta) = model.delta_or_resync(&checkpoint) else {
            panic!("compatible semantic geometry must produce row patches");
        };
        assert!(
            delta
                .row_patches
                .windows(2)
                .all(|rows| rows[0].row < rows[1].row)
        );
        delta
            .apply_to(applied.revision, &mut applied.surface)
            .expect("semantic delta applies transactionally");
        applied.revision = delta.to_revision;
        assert_eq!(applied, model.snapshot());

        model
            .ingest(b"\x1b[?1049hfull-screen")
            .expect("switch active screen");
        assert!(matches!(
            model.delta_or_resync(&checkpoint),
            TerminalSurfaceDeltaResult::Resync(_)
        ));
    }

    #[test]
    fn equal_or_future_checkpoint_never_fabricates_a_delta() {
        let current = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("current model");
        let equal = current.checkpoint();
        assert!(matches!(
            current.delta_or_resync(&equal),
            TerminalSurfaceDeltaResult::Resync(_)
        ));

        let mut future_source =
            TerminalModel::new(TerminalSize::new(2, 8), 0).expect("future source");
        future_source
            .ingest(b"future")
            .expect("advance future source");
        let future = future_source.checkpoint();
        assert!(future.revision() > current.revision());
        assert!(matches!(
            current.delta_or_resync(&future),
            TerminalSurfaceDeltaResult::Resync(_)
        ));
    }

    #[test]
    fn an_older_checkpoint_format_forces_a_complete_resync() {
        let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("current model");
        let mut stale = model.checkpoint();
        stale.projection.version = CHECKPOINT_FORMAT_VERSION.saturating_sub(1);
        model.ingest(b"new state").expect("advance current model");

        let TerminalSurfaceDeltaResult::Resync(snapshot) = model.delta_or_resync(&stale) else {
            panic!("an incompatible checkpoint format must not produce a delta");
        };
        assert_eq!(snapshot, model.snapshot());
    }

    #[test]
    fn semantic_history_window_preserves_exact_cells_without_mutation() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 8), 8).expect("semantic history model");
        model
            .ingest("\x1b[32m界\x1b[0m\r\nplain\r\nlive".as_bytes())
            .expect("seed semantic history");
        let before_state = model.snapshot();
        let before_checkpoint = model.checkpoint();
        let anchor = TerminalHistoryWindowAnchor {
            epoch: model.history_epoch,
            revision: model.revision(),
            max_offset_from_bottom: u64::try_from(model.retained_history_rows)
                .expect("bounded history"),
            viewport: model.size(),
        };
        let query = TerminalHistoryWindowQuery {
            anchor,
            target_offset_from_bottom: 1,
            older_margin_rows: 1,
            newer_margin_rows: 1,
        };
        let TerminalSurfaceHistoryWindowResult::Frame(frame) = model.history_window(query) else {
            panic!("semantic history is available");
        };
        frame.validate_for(query).expect("request-shaped frame");
        assert!(frame.rows.iter().any(|row| {
            row.cells
                .iter()
                .any(|cell| cell.contents == "界" && cell.wide)
        }));
        assert!(frame.rows.iter().any(|row| {
            row.cells
                .iter()
                .any(|cell| cell.wide_continuation && cell.contents.is_empty())
        }));
        assert_eq!(model.snapshot(), before_state);
        assert_eq!(model.checkpoint().revision(), before_checkpoint.revision());
    }
}
