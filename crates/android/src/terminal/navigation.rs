//! One mobile viewport/selection owner over the shared core page/query reducer.
use super::{NativeError, NativeNavigationStats, failure};
use std::sync::Arc;
use zterm_client::{input::append_resume_input, surface::AttachmentSurface};
use zterm_core::{
    Revision,
    terminal::*,
    terminal_selection::{TerminalTextPoint, TerminalTextRange},
    viewport_cache::{CachedViewportWindow, ViewportCache, ViewportCacheBudget},
};

type Page = Arc<CachedViewportWindow<TerminalSurfaceRow>>;
const MAX_ROWS: usize = 4096;
// Bounded page-pin and borrowed-row scratch vectors fit within this reservation.
const SELECTION_METADATA: usize = 128 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024 - SELECTION_METADATA;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct Point {
    pub row: i64,
    pub column: u16,
}
#[derive(Clone)]
struct Display {
    page: Page,
    offset: u64,
}
struct Selection {
    anchor: Point,
    focus: Point,
    epoch: Revision,
    size: TerminalSize,
    pages: Vec<Page>,
    blocked: bool,
}
#[derive(Clone, Copy, Eq, PartialEq)]
enum Position {
    Live,
    History,
    Returning,
    Recovering,
}

pub(super) struct Navigation {
    cache: ViewportCache<TerminalSurfaceRow>,
    position: Position,
    display: Option<Display>,
    awaiting_viewport: bool,
    evidence: NativeNavigationStats,
    request_start: Option<std::time::Instant>,
    live_capture: Option<Page>,
    selection: Option<Selection>,
    screen: ActiveScreen,
    size: TerminalSize,
    resume: Vec<u8>,
    snapshot_received: bool,
    discard_reply: bool,
    pub visible: bool,
    pub older: bool,
    pub horizon: u8,
    pub notice: Option<&'static str>,
}
impl Navigation {
    pub fn new(surface: &AttachmentSurface) -> Self {
        let mut value = Self {
            cache: ViewportCache::with_budget(ViewportCacheBudget {
                rows: MAX_ROWS,
                bytes: CACHE_BYTES,
            }),
            position: Position::Live,
            display: None,
            awaiting_viewport: false,
            evidence: NativeNavigationStats::default(),
            request_start: None,
            live_capture: None,
            selection: None,
            screen: surface.active_screen(),
            size: surface.surface.size,
            resume: Vec::new(),
            snapshot_received: false,
            discard_reply: false,
            visible: true,
            older: true,
            horizon: 4,
            notice: None,
        };
        value.observe(surface);
        value
    }
    pub fn observe(&mut self, surface: &AttachmentSurface) {
        if self.position == Position::Recovering {
            self.reset();
        }
        if self.screen != surface.active_screen() || self.size != surface.surface.size {
            self.reset();
            self.screen = surface.active_screen();
            self.size = surface.surface.size;
        }
        let anchor = if let Some(metrics) = surface.surface.scroll_metrics {
            TerminalHistoryWindowAnchor {
                epoch: metrics.epoch,
                revision: surface.revision(),
                max_offset_from_bottom: metrics.max_offset_from_bottom,
                viewport: surface.surface.size,
            }
        } else {
            // A local alternate-screen capture has no scrollback and is never queried.
            TerminalHistoryWindowAnchor {
                epoch: self
                    .cache
                    .anchor()
                    .map_or(surface.revision(), |old| old.epoch),
                revision: surface.revision(),
                max_offset_from_bottom: 0,
                viewport: surface.surface.size,
            }
        };
        if !self.cache.observe_anchor(anchor)
            && let Some(selection) = &mut self.selection
        {
            selection.blocked = true;
        }
    }
    pub fn reset(&mut self) {
        self.selection = None;
        self.display = None;
        self.awaiting_viewport = false;
        self.live_capture = None;
        self.resume.clear();
        self.position = Position::Live;
        self.snapshot_received = false;
        self.discard_reply |= self.cache.request_pending();
        self.cache.invalidate();
        self.notice = None;
    }
    pub fn disconnected(&mut self) {
        let display = self.display.clone().or_else(|| {
            self.live_capture.as_ref().map(|page| Display {
                page: Arc::clone(page),
                offset: 0,
            })
        });
        self.reset();
        self.display = display;
        if self.display.is_some() {
            self.position = Position::Recovering;
        }
    }
    pub fn request_failed(&mut self) {
        self.request_start = None;
        self.cache.defer_pending_request();
        self.notice = Some("history_unavailable");
    }
    pub fn request_started(&mut self) {
        self.evidence.requests += 1;
        self.request_start = Some(std::time::Instant::now());
    }
    pub fn stats(&mut self) -> NativeNavigationStats {
        if let Some(usage) = self.cache.retained_usage() {
            self.evidence.retained_rows = usage.rows as u32;
            self.evidence.retained_bytes = (usage.bytes + SELECTION_METADATA) as u64;
            self.evidence.peak_bytes = self.evidence.peak_bytes.max(self.evidence.retained_bytes);
        }
        self.evidence.clone()
    }
    pub fn returning(&self) -> bool {
        self.position == Position::Returning
    }
    pub fn live(&self) -> bool {
        self.position == Position::Live
    }
    pub fn selected(&self) -> bool {
        self.selection.is_some()
    }
    pub fn target(&self) -> u64 {
        self.cache.desired_offset_from_bottom()
    }
    pub fn offset(&self) -> u64 {
        self.display
            .as_ref()
            .and_then(|display| {
                let latest = self.cache.anchor()?;
                (latest.epoch == display.page.anchor.epoch).then(|| {
                    display.offset.saturating_add(
                        latest
                            .max_offset_from_bottom
                            .saturating_sub(display.page.anchor.max_offset_from_bottom),
                    )
                })
            })
            .unwrap_or_else(|| self.target())
    }
    pub fn first_row(&self) -> Option<i64> {
        let display = self.display.as_ref()?;
        i64::try_from(display.page.anchor.max_offset_from_bottom)
            .ok()?
            .checked_sub(i64::try_from(display.offset).ok()?)
    }
    pub fn rows(&self) -> Option<&[TerminalSurfaceRow]> {
        let display = self.display.as_ref()?;
        display.page.visible_rows_with_overscan(display.offset, 1)
    }
    pub fn endpoints(&self) -> Option<(Point, Point)> {
        self.selection.as_ref().map(|s| (s.anchor, s.focus))
    }
    pub fn frame_source(&mut self, surface: &AttachmentSurface) -> Option<(Page, u64)> {
        if !self.live() {
            return self
                .display
                .as_ref()
                .map(|display| (Arc::clone(&display.page), display.offset));
        }
        // A completed warmup at this exact live revision also supplies nearby
        // history for the first touch. Never substitute stale mutable live rows.
        if let Some(page) = self.cache.pin_visible_window()
            && page.anchor.revision == surface.revision()
            && page.visible_rows(0) == Some(surface.surface.rows.as_slice())
        {
            return Some((page, 0));
        }
        if self
            .live_capture
            .as_ref()
            .is_none_or(|page| page.anchor.revision != surface.revision())
        {
            self.live_capture = None;
            let rows = surface.surface.rows.clone();
            let bytes = row_allocations(&rows);
            let metrics = surface.surface.scroll_metrics;
            let anchor = TerminalHistoryWindowAnchor {
                epoch: metrics.map_or_else(
                    || {
                        self.cache
                            .anchor()
                            .map_or(surface.revision(), |anchor| anchor.epoch)
                    },
                    |value| value.epoch,
                ),
                revision: surface.revision(),
                viewport: surface.surface.size,
                max_offset_from_bottom: metrics.map_or(0, |value| value.max_offset_from_bottom),
            };
            self.live_capture = self.cache.capture_live_at(anchor, rows, bytes);
        }
        self.live_capture.as_ref().map(|page| (Arc::clone(page), 0))
    }
    fn freeze_live(&mut self, surface: &AttachmentSurface) -> Result<(), NativeError> {
        if self.display.is_some() {
            return Ok(());
        }
        let (page, offset) = self
            .frame_source(surface)
            .ok_or_else(|| failure("resource_limit"))?;
        self.display = Some(Display { page, offset });
        Ok(())
    }
    fn show_cached(&mut self) {
        if self.position != Position::History || !self.awaiting_viewport {
            return;
        }
        if let Some(page) = self.cache.pin_visible_window() {
            let latest = self.cache.anchor().expect("cached window has anchor");
            let offset = self.target().saturating_sub(
                latest
                    .max_offset_from_bottom
                    .saturating_sub(page.anchor.max_offset_from_bottom),
            );
            // A page read may include newer mutable live rows. Never display
            // those over captured selection text while Copy still uses its pin.
            if let Some(selection) = &mut self.selection {
                let first = page.first_ordinal().expect("validated page ordinal");
                let low = selection.anchor.row.min(selection.focus.row);
                let high = selection.anchor.row.max(selection.focus.row);
                for (index, row) in page.rows.iter().enumerate() {
                    let ordinal = first + index as i64;
                    if (low..=high).contains(&ordinal)
                        && let Ok(captured) = selected_rows(selection, ordinal, ordinal)
                        && captured[0] != row
                    {
                        selection.blocked = true;
                        self.notice = Some("selection_changed");
                        self.awaiting_viewport = false;
                        return;
                    }
                }
            }
            self.display = Some(Display { page, offset });
            self.awaiting_viewport = false;
            self.cache.commit_visible_presentation();
        }
    }
    /// Returns a query only on a miss. Prefetch is separately scheduled while visible.
    pub fn scroll(
        &mut self,
        surface: &AttachmentSurface,
        offset: u64,
        older: bool,
        horizon: u8,
    ) -> Result<Option<TerminalHistoryWindowQuery>, NativeError> {
        self.notice = None;
        self.older = older;
        self.horizon = horizon.clamp(2, 8);
        if offset == 0 && !self.selected() && !self.returning() {
            self.show_live();
            return Ok(None);
        }
        if self.returning() || surface.surface.scroll_metrics.is_none() {
            return Ok(None);
        }
        if self.live() {
            self.freeze_live(surface)?;
            self.position = Position::History;
        }
        self.awaiting_viewport = true;
        let update = self.cache.set_target(offset);
        if update.render_local {
            self.evidence.cache_hits += 1;
        } else {
            self.evidence.cache_misses += 1;
        }
        self.show_cached();
        Ok(update.request)
    }
    fn show_live(&mut self) {
        // The actor keeps applying/acknowledging live revisions while this
        // viewport reads history or a selection. Visual return needs no barrier.
        self.position = Position::Live;
        self.display = None;
        self.awaiting_viewport = false;
        if self.cache.set_target(0).request.is_some() {
            // Preserve an already outstanding query's late-reply correlation;
            // discard only a new query proposed for the current live surface.
            self.cache.defer_pending_request();
        }
    }
    pub fn prefetch(&mut self) -> Option<TerminalHistoryWindowQuery> {
        if !self.visible
            || self.returning()
            || self.screen != ActiveScreen::Main
            || self.discard_reply
            || self.notice == Some("resource_limit")
        {
            return None;
        }
        let horizon = self
            .horizon
            .saturating_add((self.evidence.query_rtt_ms / 100).min(4) as u8)
            .min(8);
        self.cache
            .prefetch_toward(self.target(), self.older, horizon)
    }
    pub fn install(
        &mut self,
        result: TerminalSurfaceHistoryWindowResult,
    ) -> Result<Option<TerminalHistoryWindowQuery>, NativeError> {
        if let Some(start) = self.request_start.take() {
            let measured = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            self.evidence.query_rtt_ms = (self.evidence.query_rtt_ms * 3 + measured) / 4;
        }
        if self.discard_reply {
            self.discard_reply = false;
            return Ok(None);
        }
        match result {
            TerminalSurfaceHistoryWindowResult::Frame(frame) => {
                let bytes = row_allocations(&frame.rows);
                let installed = self
                    .cache
                    .install_accounted_window(
                        CachedViewportWindow {
                            disposition: frame.disposition,
                            anchor: frame.anchor,
                            target_offset_from_bottom: frame.target_offset_from_bottom,
                            first_row_from_live_top: frame.first_row_from_live_top,
                            rows: frame.rows,
                        },
                        bytes,
                    )
                    .map_err(|_| failure("malformed_frame"))?;
                if installed.capacity_rejected {
                    self.notice = Some("resource_limit");
                }
                self.show_cached();
                Ok(installed.request)
            }
            TerminalSurfaceHistoryWindowResult::HistoryChanged { .. }
            | TerminalSurfaceHistoryWindowResult::HistoryGap { .. } => {
                self.cache.defer_pending_request();
                self.cache.invalidate_rows();
                if let Some(selection) = &mut self.selection {
                    selection.blocked = true;
                }
                self.notice = Some("selection_changed");
                Ok(None)
            }
        }
    }
    pub fn begin_return(&mut self, bytes: &[u8]) -> Result<bool, NativeError> {
        append_resume_input(&mut self.resume, bytes).map_err(|_| failure("resource_limit"))?;
        if self.returning() {
            return Ok(false);
        }
        self.selection = None;
        self.position = Position::Returning;
        self.snapshot_received = false;
        self.discard_reply |= self.cache.request_pending();
        self.cache.defer_pending_request();
        Ok(true)
    }
    pub fn snapshot_installed(&mut self) {
        if self.returning() {
            self.snapshot_received = true;
        }
    }
    pub fn active(&mut self) -> Option<Vec<u8>> {
        if !self.returning() || !self.snapshot_received {
            return None;
        }
        self.position = Position::Live;
        self.display = None;
        let _ = self.cache.set_target(0);
        self.cache.defer_pending_request();
        self.notice = None;
        Some(std::mem::take(&mut self.resume))
    }
    pub fn select_source(
        &mut self,
        page: Page,
        offset: u64,
        row: u16,
        column: u16,
    ) -> Result<(), NativeError> {
        let rows = page
            .visible_rows(offset)
            .ok_or_else(|| failure("selection_changed"))?;
        let range = TerminalTextRange::new(
            TerminalTextPoint::new(row, column),
            TerminalTextPoint::new(row, column),
        )
        .expand_wide(rows)
        .map_err(|_| failure("selection_changed"))?;
        let first = i64::try_from(page.anchor.max_offset_from_bottom)
            .ok()
            .and_then(|maximum| maximum.checked_sub(i64::try_from(offset).ok()?))
            .ok_or_else(|| failure("selection_changed"))?;
        self.selection = Some(Selection {
            anchor: Point {
                row: first + i64::from(row),
                column: range.start.column,
            },
            focus: Point {
                row: first + i64::from(row),
                column: range.end.column,
            },
            epoch: page.anchor.epoch,
            size: page.anchor.viewport,
            pages: vec![Arc::clone(&page)],
            blocked: self
                .cache
                .anchor()
                .is_none_or(|anchor| anchor.epoch != page.anchor.epoch),
        });
        self.display = Some(Display { page, offset });
        self.awaiting_viewport = false;
        self.position = Position::History;
        Ok(())
    }
    pub fn extend_source(
        &mut self,
        page: &Page,
        offset: u64,
        row: u16,
        column: u16,
        anchor_handle: bool,
    ) -> Result<(), NativeError> {
        let first = i64::try_from(page.anchor.max_offset_from_bottom)
            .ok()
            .and_then(|maximum| maximum.checked_sub(i64::try_from(offset).ok()?))
            .ok_or_else(|| failure("selection_changed"))?;
        let selection = self
            .selection
            .as_mut()
            .ok_or_else(|| failure("selection_changed"))?;
        if selection.blocked
            || page.anchor.epoch != selection.epoch
            || page.anchor.viewport != selection.size
        {
            return Err(failure("selection_changed"));
        }
        if row >= selection.size.rows || column >= selection.size.columns {
            return Err(failure("selection_changed"));
        }
        // Pins only windows actually touched. Gaps remain errors during extension,
        // so a fast drag cannot manufacture continuity between distant pages.
        let source = page;
        if !selection.pages.iter().any(|page| Arc::ptr_eq(page, source)) {
            if selection.pages.len() >= MAX_ROWS {
                return Err(failure("resource_limit"));
            }
            // Joining a newer page must not silently replace captured mutable
            // live cells. Require the overlapping capture to still agree.
            for old in &selection.pages {
                if old.anchor.revision == page.anchor.revision {
                    continue;
                }
                let old_first = old
                    .first_ordinal()
                    .ok_or_else(|| failure("selection_changed"))?;
                let new_first = page
                    .first_ordinal()
                    .ok_or_else(|| failure("selection_changed"))?;
                for (index, old_row) in old.rows.iter().enumerate() {
                    let ordinal = old_first + index as i64;
                    if ordinal < old.anchor.max_offset_from_bottom as i64 {
                        continue;
                    }
                    if let Ok(index) = usize::try_from(ordinal - new_first)
                        && let Some(new_row) = page.rows.get(index)
                        && old_row != new_row
                    {
                        return Err(failure("selection_changed"));
                    }
                }
            }
            selection.pages.push(Arc::clone(page));
        }
        let point = Point {
            row: first + i64::from(row),
            column,
        };
        let anchor = if anchor_handle {
            point
        } else {
            selection.anchor
        };
        let focus = if anchor_handle {
            selection.focus
        } else {
            point
        };
        let (a, b) = (anchor.min(focus), anchor.max(focus));
        let rows = selected_rows(selection, a.row, b.row)?;
        let range = TerminalTextRange::new(
            TerminalTextPoint::new(0, a.column),
            TerminalTextPoint::new((rows.len() - 1) as u16, b.column),
        )
        .expand_wide(&rows)
        .map_err(|_| failure("selection_changed"))?;
        let low = Point {
            row: a.row,
            column: range.start.column,
        };
        let high = Point {
            row: b.row,
            column: range.end.column,
        };
        if anchor <= focus {
            selection.anchor = low;
            selection.focus = high;
        } else {
            selection.anchor = high;
            selection.focus = low;
        }
        Ok(())
    }
    pub fn copy(&self) -> Result<TerminalClipboardWrite, NativeError> {
        let selection = self
            .selection
            .as_ref()
            .ok_or_else(|| failure("selection_changed"))?;
        let (a, b) = (
            selection.anchor.min(selection.focus),
            selection.anchor.max(selection.focus),
        );
        let rows = selected_rows(selection, a.row, b.row)?;
        TerminalTextRange::new(
            TerminalTextPoint::new(0, a.column),
            TerminalTextPoint::new((rows.len() - 1) as u16, b.column),
        )
        .extract(&rows)
        .map_err(|error| match error {
            zterm_core::terminal_selection::TerminalTextSelectionError::Clipboard(
                TerminalClipboardError::TooLarge,
            ) => failure("resource_limit"),
            _ => failure("selection_changed"),
        })
    }
    pub fn clear_selection(&mut self) {
        if self.selection.take().is_some()
            && self.position == Position::History
            && self.offset() == 0
            && !self.awaiting_viewport
        {
            self.show_live();
        }
        self.notice = None;
    }
}
fn selected_rows(
    selection: &Selection,
    first: i64,
    last: i64,
) -> Result<Vec<&TerminalSurfaceRow>, NativeError> {
    let count = last
        .checked_sub(first)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n <= MAX_ROWS)
        .ok_or_else(|| failure("resource_limit"))?;
    let mut rows = Vec::with_capacity(count);
    for ordinal in first..=last {
        // Earlier pins win: later output must never replace already captured text.
        let row = selection
            .pages
            .iter()
            .find_map(|page| {
                let relative = ordinal.checked_sub(page.first_ordinal()?)?;
                page.rows.get(usize::try_from(relative).ok()?)
            })
            .ok_or_else(|| failure("selection_changed"))?;
        rows.push(row);
    }
    Ok(rows)
}
pub(super) fn row_allocations(rows: &[TerminalSurfaceRow]) -> usize {
    rows.iter().fold(0usize, |sum, row| {
        sum.saturating_add(row.cells.capacity() * std::mem::size_of::<TerminalCell>())
            .saturating_add(
                row.cells
                    .iter()
                    .map(|cell| cell.contents.capacity())
                    .sum::<usize>(),
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sleeping_subscriber_receives_final_frame_before_cancellation() {
        use super::super::{NativeTerminal, project};
        use std::{
            future::Future,
            task::{Context, Poll, Waker},
        };
        use tokio::sync::{mpsc, watch};
        use tokio_util::sync::CancellationToken;

        for state in ["lease_lost", "ended", "closed"] {
            let surface = surface(1, 1, 0);
            let (frame, _) = watch::channel(project(
                &surface,
                1,
                "active",
                None,
                true,
                &surface.surface.rows,
            ));
            let (commands, _receiver) = mpsc::channel(1);
            let terminal = NativeTerminal {
                session_id: zterm_core::SessionId::from_array([1; 16]),
                commands,
                frame,
                cancel: CancellationToken::new(),
            };
            let pending = terminal.wait_for_frame(1);
            tokio::pin!(pending);
            assert!(matches!(
                pending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            terminal.frame.send_replace(project(
                &surface,
                2,
                state,
                None,
                true,
                &surface.surface.rows,
            ));
            terminal.cancel.cancel();
            assert_eq!(pending.await.expect("final published state").state, state);
            assert!(matches!(
                terminal.wait_for_frame(2).await,
                Err(crate::NativeError::Closed)
            ));
        }
    }
    fn rows(first: i64, count: usize) -> Vec<TerminalSurfaceRow> {
        (0..count)
            .map(|index| TerminalSurfaceRow {
                cells: format!("{:07} ", first + index as i64)
                    .chars()
                    .map(|value| TerminalCell {
                        contents: value.to_string(),
                        ..Default::default()
                    })
                    .collect(),
                wrapped: false,
            })
            .collect()
    }
    fn surface(revision: u64, epoch: u64, maximum: u64) -> AttachmentSurface {
        AttachmentSurface::from_snapshot(&TerminalSurfaceSnapshot {
            revision: Revision::new(revision),
            surface: TerminalSurface {
                colors: TerminalColorSnapshot::default(),
                size: TerminalSize::new(4, 8),
                active_screen: ActiveScreen::Main,
                rows: rows(maximum as i64, 4),
                cursor: TerminalCursor {
                    row: 0,
                    column: 0,
                    visible: true,
                    style: TerminalStyle::default(),
                },
                modes: TerminalModes::default(),
                scroll_metrics: Some(TerminalScrollMetrics {
                    epoch: Revision::new(epoch),
                    revision: Revision::new(revision),
                    max_offset_from_bottom: maximum,
                    offset_from_bottom: 0,
                    viewport_rows: 4,
                }),
            },
        })
        .expect("validated fixture")
    }
    fn response(query: TerminalHistoryWindowQuery) -> TerminalSurfaceHistoryWindowResult {
        let shape = query.response_shape(query.anchor).expect("bounded request");
        TerminalSurfaceHistoryWindowResult::Frame(TerminalSurfaceHistoryWindowFrame {
            colors: TerminalColorSnapshot::default(),
            disposition: shape.disposition,
            anchor: query.anchor,
            target_offset_from_bottom: shape.target_offset_from_bottom,
            first_row_from_live_top: shape.first_row_from_live_top,
            rows: rows(
                query.anchor.max_offset_from_bottom as i64 + shape.first_row_from_live_top,
                shape.row_count,
            ),
        })
    }
    fn scroll(nav: &mut Navigation, live: &AttachmentSurface, offset: u64) {
        if let Some(query) = nav.scroll(live, offset, true, 4).expect("scroll") {
            assert!(nav.install(response(query)).expect("install").is_none());
        }
    }

    #[test]
    fn live_warmup_supplies_adjacent_rows_without_moving_the_viewport() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        let query = nav.prefetch().expect("warmup");
        nav.install(response(query)).expect("warm page");
        let frame = super::super::project_navigation(
            &live,
            1,
            "active",
            None,
            true,
            &mut nav,
            7,
            1,
            false,
            &Arc::new(()),
            None,
        );
        assert_eq!(frame.history_offset, 0);
        assert!(frame.window_offset >= 4);
        let source = frame.source.expect("warm live source");
        assert!(
            source.viewport_source(100).is_ok(),
            "current live pixels remain available"
        );
        assert!(
            source.viewport_source(99).is_ok(),
            "first row of motion needs no actor reply"
        );
    }

    #[test]
    fn presentation_window_reuses_rows_and_binds_locally_drawn_coordinates() {
        let live = surface(1, 1, 100);
        let origin = Arc::new(());
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 50);
        let first = super::super::project_navigation(
            &live, 1, "active", None, true, &mut nav, 7, 1, false, &origin, None,
        );
        let source = first.source.as_ref().expect("window source");
        let rows = source.presentation_rows();
        assert!(first.rows.is_empty(), "metadata does not marshal cells");
        assert!(
            rows.len() > 4 && rows.len() <= 13,
            "at most three screens plus overscan"
        );
        let repeated = super::super::project_navigation(
            &live,
            2,
            "active",
            None,
            true,
            &mut nav,
            7,
            1,
            false,
            &origin,
            Some(&first),
        );
        assert_eq!(first.content_generation, repeated.content_generation);
        scroll(&mut nav, &live, 51);
        let moved = super::super::project_navigation(
            &live,
            3,
            "active",
            None,
            true,
            &mut nav,
            7,
            1,
            false,
            &origin,
            Some(&repeated),
        );
        assert_eq!(
            first.content_generation, moved.content_generation,
            "a local row step reuses the DTO window"
        );
        assert_eq!(first.first_row, moved.first_row);
        assert_eq!(first.window_offset, moved.window_offset);
        let displayed_first = first.first_row + 1;
        let drawn = source
            .viewport_source(displayed_first)
            .expect("locally drawn viewport");
        assert!(drawn.valid_for(&origin, 7, 1, &live));
        assert!(source.viewport_source(first.first_row - 1).is_err());
        assert!(
            source
                .viewport_source(first.first_row + rows.len() as i64 - 3)
                .is_err()
        );
        nav.select_source(Arc::clone(&drawn.page), drawn.offset, 0, 0)
            .expect("select drawn rows");
        assert_eq!(
            nav.endpoints().expect("selected endpoints").0.row,
            displayed_first
        );
        assert!(!drawn.valid_for(&origin, 7, 2, &live));
        assert!(!drawn.valid_for(&origin, 8, 1, &live));
        assert!(!drawn.valid_for(&Arc::new(()), 7, 1, &live));
        let recolored = super::super::project_navigation(
            &live,
            4,
            "active",
            None,
            false,
            &mut nav,
            7,
            1,
            false,
            &origin,
            Some(&moved),
        );
        assert_ne!(
            moved.content_generation, recolored.content_generation,
            "palette changes replace projected rows"
        );
    }

    #[test]
    fn healthy_resize_retains_input_but_retires_coordinates_even_after_a_b_a() {
        let live = surface(1, 1, 100);
        let origin = Arc::new(());
        let mut nav = Navigation::new(&live);
        let initial = super::super::project_navigation(
            &live, 1, "active", None, true, &mut nav, 7, 1, false, &origin, None,
        );
        let source = initial.source.expect("drawn source");
        assert!(source.valid_for(&origin, 7, 1, &live));
        let resizing = super::super::project_navigation(
            &live,
            2,
            "synchronizing",
            None,
            true,
            &mut nav,
            7,
            2,
            true,
            &origin,
            None,
        );
        assert!(resizing.input_ready);
        assert_eq!(resizing.cursor_visible, live.surface.cursor.visible);
        assert!(
            !resizing
                .source
                .as_ref()
                .expect("presentation rows remain available")
                .valid_for(&origin, 7, 2, &live),
            "presentation-only resize source cannot authorize pointer input"
        );
        assert!(
            !source.valid_for(&origin, 7, 3, &live),
            "same dimensions do not revive old coordinates"
        );
        let reconnected = super::super::project_navigation(
            &live,
            3,
            "reconnecting",
            None,
            true,
            &mut nav,
            8,
            3,
            false,
            &origin,
            None,
        );
        assert!(!reconnected.input_ready);
        assert!(!source.valid_for(&origin, 8, 1, &live));
        assert!(!source.valid_for(&Arc::new(()), 7, 1, &live));
    }

    #[test]
    fn clearing_live_selection_restores_latest_pixels_without_sync() {
        for alternate in [false, true] {
            let mut live = surface(1, 1, 100);
            if alternate {
                live.surface.active_screen = ActiveScreen::Alternate;
                live.surface.scroll_metrics = None;
            }
            let mut nav = Navigation::new(&live);
            let (page, offset) = nav.frame_source(&live).expect("drawn source");
            nav.select_source(page, offset, 0, 0).expect("selection");
            let mut next = surface(2, 1, 100);
            next.surface.active_screen = live.active_screen();
            if alternate {
                next.surface.scroll_metrics = None;
            }
            next.surface.rows[0].cells[0].contents = "x".into();
            nav.observe(&next);
            assert_eq!(nav.copy().expect("captured text").as_str(), "0");
            let origin = Arc::new(());
            let frozen = super::super::project_navigation(
                &next, 1, "active", None, true, &mut nav, 7, 1, false, &origin, None,
            );
            assert_eq!(
                frozen
                    .source
                    .as_ref()
                    .expect("presentation source")
                    .presentation_rows()[0]
                    .cells[0]
                    .text,
                "0",
                "render the captured selection"
            );
            assert!(!frozen.cursor_visible);
            nav.clear_selection();
            assert!(
                nav.live(),
                "selection exit must restore child pointer admission"
            );
            assert!(!nav.selected());
            assert!(
                !nav.returning(),
                "no snapshot barrier for local presentation"
            );
            assert!(nav.rows().is_none(), "release frozen display");
            let restored = super::super::project_navigation(
                &next, 2, "active", None, true, &mut nav, 7, 1, false, &origin, None,
            );
            assert_eq!(
                restored
                    .source
                    .as_ref()
                    .expect("presentation source")
                    .presentation_rows()[0]
                    .cells[0]
                    .text,
                "x",
                "render current live pixels"
            );
            assert!(restored.cursor_visible);
            let (page, offset) = nav.frame_source(&next).expect("latest source");
            assert_eq!(offset, 0);
            assert_eq!(page.rows[0].cells[0].contents, "x");
            nav.clear_selection();
            assert!(nav.live(), "ActionMode destruction may clear twice");
        }
    }

    #[test]
    fn clearing_selection_preserves_scrolled_or_newly_historical_pixels() {
        for offset in [0, 12] {
            let live = surface(1, 1, 100);
            let mut nav = Navigation::new(&live);
            scroll(&mut nav, &live, offset);
            let (page, source_offset) = nav.frame_source(&live).expect("drawn source");
            nav.select_source(page, source_offset, 0, 0)
                .expect("selection");
            let first = nav.first_row();
            nav.observe(&surface(2, 1, 104));
            nav.clear_selection();
            assert!(!nav.live(), "do not jump past the user's reading position");
            assert!(!nav.selected());
            assert_eq!(nav.first_row(), first);
            assert_eq!(nav.offset(), offset + 4);
        }
    }

    #[test]
    fn clearing_selection_preserves_pending_scroll_and_late_reply_correlation() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        let (page, offset) = nav.frame_source(&live).expect("drawn source");
        nav.select_source(page, offset, 0, 0).expect("selection");
        let query = nav
            .scroll(&live, 70, true, 4)
            .expect("scroll")
            .expect("miss");
        nav.clear_selection();
        assert!(!nav.live(), "pending older scroll retains its intent");
        nav.install(response(query)).expect("older page");
        assert_eq!(nav.offset(), 70);

        scroll(&mut nav, &live, 0);
        let pending = nav.prefetch().expect("background query");
        let (page, offset) = nav.frame_source(&live).expect("live source");
        nav.select_source(page, offset, 0, 0).expect("selection");
        nav.clear_selection();
        assert!(nav.live());
        nav.install(response(pending)).expect("late reply");
        assert!(nav.live());
        assert!(nav.rows().is_none());
        assert_eq!(nav.offset(), 0);
    }

    #[test]
    fn rendered_source_remains_exact_after_new_live_output() {
        let old = surface(1, 1, 100);
        let mut nav = Navigation::new(&old);
        let (page, offset) = nav.frame_source(&old).expect("render source");
        let mut next = surface(2, 1, 104);
        next.surface.rows[0].cells[0].contents = "x".into();
        nav.observe(&next);
        nav.select_source(page, offset, 0, 0)
            .expect("select actual old pixels");
        assert_eq!(nav.copy().expect("copy captured glyph").as_str(), "0");
        assert_eq!(nav.offset(), 4);
        assert_eq!(nav.first_row(), Some(100));
        let request = nav.prefetch().expect("background warmup");
        nav.install(response(request)).expect("warm page");
        assert_eq!(
            nav.first_row(),
            Some(100),
            "prefetch cannot move the selected viewport"
        );
        assert_eq!(nav.copy().expect("same selection").as_str(), "0");
    }

    #[test]
    fn selection_spans_three_screens_and_survives_trim_as_frozen_text() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        let (page, offset) = nav.frame_source(&live).expect("history frame");
        nav.select_source(page, offset, 3, 7).expect("anchor");
        scroll(&mut nav, &live, 20);
        let (page, offset) = nav.frame_source(&live).expect("older frame");
        nav.extend_source(&page, offset, 0, 0, false)
            .expect("cross page extension");
        let copied = nav.copy().expect("copy multiple screens").into_string();
        assert_eq!(
            copied,
            (80..=91)
                .map(|row| format!("{row:07} "))
                .collect::<Vec<_>>()
                .join("\n")
        );
        nav.observe(&surface(2, 2, 90));
        assert!(nav.extend_source(&page, offset, 1, 0, false).is_err());
        assert_eq!(
            nav.copy().expect("frozen text survives trim").as_str(),
            copied
        );
    }

    #[test]
    fn missing_range_refuses_extension_without_replacing_existing_copy() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        let (page, offset) = nav.frame_source(&live).expect("source");
        nav.select_source(page, offset, 0, 0).expect("anchor");
        let old = nav.copy().expect("old copy").into_string();
        scroll(&mut nav, &live, 60);
        let (page, offset) = nav.frame_source(&live).expect("distant source");
        assert!(nav.extend_source(&page, offset, 0, 0, false).is_err());
        assert_eq!(nav.copy().expect("copy preserved").as_str(), old);
    }

    #[test]
    fn bottom_scroll_uses_latest_live_surface_without_sync_or_late_page_replacement() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        let pending = nav
            .scroll(&live, 70, true, 4)
            .expect("older miss")
            .expect("query");
        let next = surface(2, 1, 104);
        nav.observe(&next);
        assert!(
            nav.scroll(&next, 0, false, 4)
                .expect("local bottom")
                .is_none()
        );
        assert!(
            nav.live(),
            "visual return cannot require a new snapshot/Active"
        );
        assert_eq!(nav.offset(), 0);
        assert!(nav.rows().is_none(), "project the current surface directly");
        let (page, offset) = nav.frame_source(&next).expect("current surface");
        assert_eq!(page.anchor.revision, next.revision());
        assert_eq!(offset, 0);
        nav.install(response(pending)).expect("late history reply");
        assert!(nav.live());
        assert!(nav.rows().is_none());
        assert_eq!(nav.offset(), 0);
    }

    #[test]
    fn selected_bottom_stays_pinned_and_does_not_enable_child_pointer_input() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        let (page, offset) = nav.frame_source(&live).expect("display source");
        nav.select_source(page, offset, 0, 0).expect("selection");
        let copied = nav.copy().expect("captured text").into_string();
        scroll(&mut nav, &live, 0);
        assert!(!nav.live());
        assert!(nav.selected());
        assert_eq!(nav.copy().expect("same captured text").as_str(), copied);
    }

    #[test]
    fn healthy_return_waits_for_snapshot_and_active_then_drains_input_once() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        assert!(
            nav.begin_return("first中".as_bytes())
                .expect("first admission")
        );
        assert!(nav.active().is_none());
        assert!(!nav.begin_return(b"second").expect("ordered admission"));
        nav.clear_selection();
        assert!(
            nav.returning(),
            "late selection exit preserves the input barrier"
        );
        nav.snapshot_installed();
        assert_eq!(
            nav.active().expect("release after ACK and Active"),
            "first中second".as_bytes()
        );
        assert!(nav.active().is_none());
        assert!(nav.live());
    }

    #[test]
    fn disconnection_discards_retained_input_and_preserves_last_complete_rows() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        scroll(&mut nav, &live, 12);
        let first = nav.first_row();
        nav.begin_return(b"never replay")
            .expect("healthy admission");
        nav.disconnected();
        nav.clear_selection();
        assert!(
            !nav.live(),
            "late ActionMode exit must not override recovery"
        );
        assert_eq!(nav.first_row(), first);
        assert!(nav.rows().is_some());
        nav.observe(&surface(2, 1, 100));
        nav.snapshot_installed();
        assert!(nav.active().is_none());
        assert!(nav.live());
    }

    #[test]
    fn render_pin_budget_includes_nested_cell_allocations() {
        let live = surface(1, 1, 100);
        let mut nav = Navigation::new(&live);
        let (page, _) = nav.frame_source(&live).expect("pin");
        let before = nav.cache.retained_usage().expect("accounted cache");
        assert_eq!(before.rows, 4);
        assert!(before.bytes >= row_allocations(&page.rows));
        let (second, _) = nav.frame_source(&live).expect("same immutable source");
        assert!(Arc::ptr_eq(&page, &second));
        assert_eq!(
            nav.cache.retained_usage().expect("unchanged allocations"),
            before
        );
    }
}
