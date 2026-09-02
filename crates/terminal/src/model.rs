use std::fmt;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use zterm_core::Revision;
use zterm_core::terminal::{
    ActiveScreen, MAX_HISTORY_PAGE_ROWS, TerminalDelta, TerminalDeltaResult, TerminalHistoryCursor,
    TerminalHistoryDirection, TerminalHistoryPage, TerminalHistoryResult, TerminalSize,
    TerminalSnapshot, TerminalState, TerminalUpdate,
};

use crate::ansi::{encode_delta, encode_full, encode_history_row};
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
    /// A history page requested zero rows or exceeded the fixed page bound.
    InvalidHistoryPageSize {
        /// Requested number of rows.
        requested: usize,
        /// Product maximum for one page.
        maximum: usize,
    },
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
            Self::InvalidHistoryPageSize { requested, maximum } => write!(
                formatter,
                "terminal history page size {requested} is outside 1..={maximum}",
            ),
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
            });
        }

        let next_revision = self.next_revision()?;
        let mut output = UpdateCollector::new();
        self.ingress.process(bytes, &mut self.engine, &mut output)?;
        self.revision = next_revision;
        self.refresh_history_epoch_after_ingest();
        let (replies, events) = output.finish();
        Ok(TerminalUpdate {
            revision: self.revision,
            replies,
            events,
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

    /// Captures a full reconnect snapshot of the latest active state.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSnapshot {
        let projection = project(&self.engine);
        TerminalSnapshot {
            revision: self.revision,
            size: projection.size,
            active_screen: projection.active_screen,
            screen_ansi: encode_full(&projection),
            recent_history_ansi: self.recent_history_ansi(),
            modes: projection.modes,
        }
    }

    /// Produces one merged latest-state delta or a full resynchronization.
    #[must_use]
    pub fn delta_or_resync(&self, checkpoint: &TerminalCheckpoint) -> TerminalDeltaResult {
        let latest = project(&self.engine);
        let snapshot = || TerminalSnapshot {
            revision: self.revision,
            size: latest.size,
            active_screen: latest.active_screen,
            screen_ansi: encode_full(&latest),
            recent_history_ansi: self.recent_history_ansi(),
            modes: latest.modes,
        };
        if checkpoint.revision > self.revision
            || checkpoint.projection.version != CHECKPOINT_FORMAT_VERSION
            || checkpoint.projection.size != latest.size
            || checkpoint.projection.active_screen != latest.active_screen
        {
            return TerminalDeltaResult::Resync(snapshot());
        }

        let changed_rows = checkpoint
            .projection
            .rows
            .iter()
            .zip(&latest.rows)
            .filter(|(before, after)| before != after)
            .count();
        if !latest.rows.is_empty() && changed_rows == latest.rows.len() {
            return TerminalDeltaResult::Resync(snapshot());
        }
        // A revision can advance without changing any client-visible terminal
        // semantics (for example, a successful same-size resize). Preserve the
        // revision edge, but do not manufacture cursor/mode ANSI when the
        // complete projected state is already identical.
        let ansi = if checkpoint.projection == latest {
            Vec::new()
        } else {
            encode_delta(&checkpoint.projection, &latest)
        };
        let delta = TerminalDelta {
            from_revision: checkpoint.revision,
            to_revision: self.revision,
            size: latest.size,
            active_screen: latest.active_screen,
            ansi,
            modes: latest.modes,
        };
        let full = snapshot();
        if delta.ansi_payload_len() >= full.ansi_payload_len() {
            TerminalDeltaResult::Resync(full)
        } else {
            TerminalDeltaResult::Delta(delta)
        }
    }

    /// Returns a zterm-owned semantic projection of the visible state.
    #[must_use]
    pub fn state(&self) -> TerminalState {
        project(&self.engine).to_state()
    }

    /// Returns one bounded, revision-aware page from retained main history.
    pub fn history_page(
        &self,
        direction: TerminalHistoryDirection,
        cursor: Option<TerminalHistoryCursor>,
        maximum_rows: usize,
    ) -> Result<TerminalHistoryResult, TerminalError> {
        if maximum_rows == 0 || maximum_rows > MAX_HISTORY_PAGE_ROWS {
            return Err(TerminalError::InvalidHistoryPageSize {
                requested: maximum_rows,
                maximum: MAX_HISTORY_PAGE_ROWS,
            });
        }
        if self.engine.active_screen() != ActiveScreen::Main {
            return Ok(TerminalHistoryResult::HistoryChanged {
                epoch: self.history_epoch,
                revision: self.revision,
            });
        }

        let total = self.retained_history_rows;
        let total_u64 = u64::try_from(total).unwrap_or(u64::MAX);
        let start = match direction {
            TerminalHistoryDirection::Newest => total.saturating_sub(maximum_rows),
            TerminalHistoryDirection::Older | TerminalHistoryDirection::Newer => {
                let Some(cursor) = cursor else {
                    return Ok(TerminalHistoryResult::HistoryGap {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                };
                if cursor.epoch != self.history_epoch {
                    return Ok(TerminalHistoryResult::HistoryChanged {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                }
                let end = cursor.start_row.checked_add(u64::from(cursor.row_count));
                if cursor.oldest_row != 0
                    || cursor.start_row < cursor.oldest_row
                    || end.is_none_or(|end| end > cursor.newest_row)
                    || cursor.newest_row > total_u64
                {
                    return Ok(TerminalHistoryResult::HistoryGap {
                        epoch: self.history_epoch,
                        revision: self.revision,
                    });
                }
                match direction {
                    TerminalHistoryDirection::Older => usize::try_from(cursor.start_row)
                        .unwrap_or(usize::MAX)
                        .saturating_sub(maximum_rows),
                    TerminalHistoryDirection::Newer => {
                        usize::try_from(end.unwrap_or(total_u64)).unwrap_or(usize::MAX)
                    }
                    TerminalHistoryDirection::Newest => unreachable!(),
                }
            }
        };
        if start > total {
            return Ok(TerminalHistoryResult::HistoryGap {
                epoch: self.history_epoch,
                revision: self.revision,
            });
        }
        let count = maximum_rows.min(total - start);
        let rows = self.formatted_history_rows(total, start, count);
        Ok(TerminalHistoryResult::Page(TerminalHistoryPage {
            cursor: TerminalHistoryCursor {
                epoch: self.history_epoch,
                revision: self.revision,
                start_row: u64::try_from(start).unwrap_or(u64::MAX),
                row_count: u32::try_from(rows.len()).unwrap_or(u32::MAX),
                oldest_row: 0,
                newest_row: total_u64,
            },
            rows,
        }))
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

    fn formatted_history_rows(&self, total: usize, start: usize, count: usize) -> Vec<Vec<u8>> {
        (start..start.saturating_add(count))
            .map(|index| {
                let distance = total.saturating_sub(index);
                let line = Line(-i32::try_from(distance).unwrap_or(i32::MAX));
                encode_history_row(&project_row(&self.engine, line))
            })
            .collect()
    }

    fn recent_history_ansi(&self) -> Vec<u8> {
        if self.engine.active_screen() != ActiveScreen::Main || self.retained_history_rows == 0 {
            return Vec::new();
        }
        let mut output = b"\x1b[m".to_vec();
        for row in
            self.formatted_history_rows(self.retained_history_rows, 0, self.retained_history_rows)
        {
            output.extend_from_slice(&row);
            output.extend_from_slice(b"\r\n");
        }
        for _ in 1..self.size().rows {
            output.extend_from_slice(b"\r\n");
        }
        output
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
    use crate::ansi::uses_only_allowlisted_ansi;

    #[test]
    fn revision_overflow_never_mutates_terminal_state() {
        let mut model = TerminalModel::new(TerminalSize::new(2, 8), 0).expect("valid model");
        model.revision = Revision::new(u64::MAX);
        let before = model.state();
        assert_eq!(
            model.ingest(b"not rendered"),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.state(), before);
        assert_eq!(
            model.resize(TerminalSize::new(3, 9)),
            Err(TerminalError::RevisionOverflow)
        );
        assert_eq!(model.state(), before);
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
    fn history_pages_are_ordered_revision_bound_and_non_mutating() {
        let mut model =
            TerminalModel::new(TerminalSize::new(2, 12), 8).expect("bounded history terminal");
        model
            .ingest(b"one\r\ntwo\r\nthree\r\nfour\r\nfive")
            .expect("seed history");
        let before = model.state();
        let TerminalHistoryResult::Page(page) = model
            .history_page(TerminalHistoryDirection::Newest, None, 2)
            .expect("newest page")
        else {
            panic!("newest history must be available");
        };
        assert_eq!(page.rows.len(), 2);
        assert!(String::from_utf8_lossy(&page.rows[0]).contains("two"));
        assert!(String::from_utf8_lossy(&page.rows[1]).contains("three"));
        assert_eq!(model.state(), before, "paging must not mutate live state");

        model.ingest(b"\r\nsix").expect("append below capacity");
        let TerminalHistoryResult::Page(newer) = model
            .history_page(
                TerminalHistoryDirection::Newer,
                Some(page.cursor),
                MAX_HISTORY_PAGE_ROWS,
            )
            .expect("newer page after monotonic append")
        else {
            panic!("monotonic append keeps the history epoch");
        };
        assert_eq!(newer.cursor.epoch, page.cursor.epoch);
        assert!(
            newer
                .rows
                .iter()
                .any(|row| String::from_utf8_lossy(row).contains("four"))
        );

        model
            .resize(TerminalSize::new(3, 12))
            .expect("resize invalidates row identity");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Older, Some(page.cursor), 2)
                .expect("typed stale result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));
        assert!(matches!(
            model.history_page(TerminalHistoryDirection::Newest, None, 0),
            Err(TerminalError::InvalidHistoryPageSize { .. })
        ));
    }

    #[test]
    fn history_eviction_and_alternate_screen_fail_conservatively() {
        let mut model = TerminalModel::new(TerminalSize::new(2, 10), 2)
            .expect("small bounded history terminal");
        model.ingest(b"one\r\ntwo\r\nthree").expect("fill history");
        let TerminalHistoryResult::Page(page) = model
            .history_page(TerminalHistoryDirection::Newest, None, 2)
            .expect("initial page")
        else {
            panic!("initial page must exist");
        };
        model
            .ingest(b"\r\nfour\r\nfive")
            .expect("evict old history");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Older, Some(page.cursor), 2)
                .expect("typed eviction result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));

        model
            .ingest(b"\x1b[?1049h")
            .expect("enter alternate screen");
        assert!(matches!(
            model
                .history_page(TerminalHistoryDirection::Newest, None, 2)
                .expect("typed alternate result"),
            TerminalHistoryResult::HistoryChanged { .. }
        ));
    }

    #[test]
    fn every_model_authored_ansi_surface_uses_the_allowlist() {
        let mut model = TerminalModel::new(TerminalSize::new(3, 12), 4).expect("valid model");
        model
            .ingest(b"history-one\r\nhistory-two\r\nvisible")
            .expect("initial state");
        let snapshot = model.snapshot();
        assert!(
            uses_only_allowlisted_ansi(&snapshot.screen_ansi, true),
            "snapshot ANSI: {:?}",
            String::from_utf8_lossy(&snapshot.screen_ansi),
        );
        assert!(uses_only_allowlisted_ansi(
            &snapshot.recent_history_ansi,
            false,
        ));

        let checkpoint = model.checkpoint();
        model.ingest(b"\x1b[2;2H!").expect("small change");
        match model.delta_or_resync(&checkpoint) {
            TerminalDeltaResult::Delta(delta) => {
                assert!(uses_only_allowlisted_ansi(&delta.ansi, false));
            }
            TerminalDeltaResult::Resync(snapshot) => {
                assert!(uses_only_allowlisted_ansi(&snapshot.screen_ansi, true));
                assert!(uses_only_allowlisted_ansi(
                    &snapshot.recent_history_ansi,
                    false,
                ));
            }
        }

        let TerminalHistoryResult::Page(page) = model
            .history_page(TerminalHistoryDirection::Newest, None, 2)
            .expect("history page")
        else {
            panic!("history remains available");
        };
        assert!(
            page.rows
                .iter()
                .all(|row| uses_only_allowlisted_ansi(row, false))
        );
    }
}
