//! Renderer-neutral bounded viewport-window cache and request coalescer.

mod pages;
use pages::Pages;
pub use pages::ViewportCacheBudget;
use std::sync::Arc;

use crate::terminal::{
    MAX_HISTORY_WINDOW_ROWS, TerminalHistoryWindowAnchor, TerminalHistoryWindowQuery, TerminalSize,
    TerminalViewportDisposition,
};

/// Immutable identity of one complete cached viewport slice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportSliceIdentity {
    /// Retained-history epoch of the immutable cached window.
    pub epoch: crate::Revision,
    /// Model revision at which the cached rows were projected.
    pub revision: crate::Revision,
    /// First visible row in the cached window's live-top coordinates.
    pub first_row_from_live_top: i64,
    /// Exact viewport geometry represented by the slice.
    pub viewport: TerminalSize,
}

/// One immutable contiguous row window in live-top coordinates.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedViewportWindow<Row> {
    /// Whether this response retained or replaced the request identity.
    pub disposition: TerminalViewportDisposition,
    /// Coordinate-space anchor used by these rows.
    pub anchor: TerminalHistoryWindowAnchor,
    /// Resolved target used to center the response.
    pub target_offset_from_bottom: u64,
    /// Coordinate of the first row relative to live-screen top.
    pub first_row_from_live_top: i64,
    /// Contiguous rows in top-to-bottom order.
    pub rows: Vec<Row>,
}

impl<Row> CachedViewportWindow<Row> {
    /// Returns whether the window is structurally valid and contains its target viewport.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if !self.anchor.is_valid()
            || self.target_offset_from_bottom > self.anchor.max_offset_from_bottom
            || self.rows.len() < usize::from(self.anchor.viewport.rows)
            || self.rows.len() > MAX_HISTORY_WINDOW_ROWS
        {
            return false;
        }
        let Some(end) = self.end_row_exclusive() else {
            return false;
        };
        let Ok(history) = i64::try_from(self.anchor.max_offset_from_bottom) else {
            return false;
        };
        self.first_row_from_live_top >= -history
            && end <= i64::from(self.anchor.viewport.rows)
            && self.visible_range(self.target_offset_from_bottom).is_some()
    }

    /// Returns whether this is the exact bounded response to `query`.
    #[must_use]
    pub fn is_valid_for(&self, query: TerminalHistoryWindowQuery) -> bool {
        self.is_valid()
            && query.response_shape(self.anchor).is_some_and(|shape| {
                self.disposition == shape.disposition
                    && self.target_offset_from_bottom == shape.target_offset_from_bottom
                    && self.first_row_from_live_top == shape.first_row_from_live_top
                    && self.rows.len() == shape.row_count
            })
    }

    /// Returns the exact full-height row slice for an absolute target, when cached.
    #[must_use]
    pub fn visible_rows(&self, target_offset_from_bottom: u64) -> Option<&[Row]> {
        let range = self.visible_range(target_offset_from_bottom)?;
        self.rows.get(range)
    }

    /// Known overscan rows for fractional native scrolling; never fabricates a row.
    #[must_use]
    pub fn visible_rows_with_overscan(&self, offset: u64, extra: u16) -> Option<&[Row]> {
        let mut range = self.visible_range(offset)?;
        range.end = range
            .end
            .saturating_add(usize::from(extra))
            .min(self.rows.len());
        self.rows.get(range)
    }

    /// Stable first historical ordinal, valid only within this epoch/geometry.
    #[must_use]
    pub fn first_ordinal(&self) -> Option<i64> {
        i64::try_from(self.anchor.max_offset_from_bottom)
            .ok()?
            .checked_add(self.first_row_from_live_top)
    }

    fn visible_range(&self, target_offset_from_bottom: u64) -> Option<std::ops::Range<usize>> {
        if target_offset_from_bottom > self.anchor.max_offset_from_bottom {
            return None;
        }
        let target = i64::try_from(target_offset_from_bottom).ok()?;
        let visible_start = target.checked_neg()?;
        let relative = visible_start.checked_sub(self.first_row_from_live_top)?;
        let start = usize::try_from(relative).ok()?;
        let end = start.checked_add(usize::from(self.anchor.viewport.rows))?;
        (end <= self.rows.len()).then_some(start..end)
    }

    fn end_row_exclusive(&self) -> Option<i64> {
        self.first_row_from_live_top
            .checked_add(i64::try_from(self.rows.len()).ok()?)
    }
}

/// Result of changing a client-owned desired viewport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportCacheUpdate {
    /// A complete local slice is ready for immediate presentation.
    pub render_local: bool,
    /// One miss or low-water request should be sent now.
    pub request: Option<TerminalHistoryWindowQuery>,
}

/// Result of installing one response window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportCacheInstall {
    /// Valid response could not fit without evicting pinned content.
    pub capacity_rejected: bool,
    /// The response covers the latest desired target and may be presented.
    pub render_local: bool,
    /// A newer uncovered desired target requires one follow-up request.
    pub request: Option<TerminalHistoryWindowQuery>,
}

/// Compact two-phase plan for observing a live history-coordinate anchor.
///
/// The plan contains only cache metadata. In particular, it never clones the
/// cached row window, so a platform presenter may validate and flush related
/// host effects before committing the corresponding semantic observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportAnchorObservation {
    latest_anchor: Option<TerminalHistoryWindowAnchor>,
    desired_offset_from_bottom: u64,
    presented_offset_from_bottom: Option<u64>,
    cache_action: AnchorCacheAction,
    compatible: bool,
    presented_slice_identity: Option<ViewportSliceIdentity>,
}

impl ViewportAnchorObservation {
    /// Returns whether the observation preserves the cached coordinate space.
    #[must_use]
    pub const fn is_compatible(self) -> bool {
        self.compatible
    }

    /// Returns the presented slice identity after this plan is committed.
    #[must_use]
    pub const fn presented_slice_identity(self) -> Option<ViewportSliceIdentity> {
        self.presented_slice_identity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnchorCacheAction {
    Preserve,
    InvalidateRows,
    InvalidateAll,
}

/// A bounded, contiguous, renderer-independent viewport cache.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewportCache<Row> {
    latest_anchor: Option<TerminalHistoryWindowAnchor>,
    window: Option<CachedViewportWindow<Row>>,
    pages: Option<Pages<Row>>,
    desired_offset_from_bottom: u64,
    presented_offset_from_bottom: Option<u64>,
    pending_query: Option<TerminalHistoryWindowQuery>,
}

impl<Row> Default for ViewportCache<Row> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Row> ViewportCache<Row> {
    /// Creates an empty live-position cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            latest_anchor: None,
            window: None,
            pages: None,
            desired_offset_from_bottom: 0,
            presented_offset_from_bottom: None,
            pending_query: None,
        }
    }

    /// Opts into accounted immutable pages; the desktop default stays single-window.
    #[must_use]
    pub fn with_budget(budget: ViewportCacheBudget) -> Self {
        Self {
            pages: Some(Pages::new(budget)),
            ..Self::new()
        }
    }

    /// Accounted retained allocations, including pages held by selection pins.
    #[must_use]
    pub fn retained_usage(&self) -> Option<ViewportCacheBudget> {
        self.pages.as_ref().map(Pages::usage)
    }

    /// Pins the visible source without cloning rows. Keep this owner until the
    /// selection or presentation no longer refers to its captured content.
    #[must_use]
    pub fn pin_visible_window(&self) -> Option<Arc<CachedViewportWindow<Row>>> {
        self.pages
            .as_ref()?
            .pin(self.latest_anchor?, self.desired_offset_from_bottom)
    }

    /// Accounts and pins an authoritative live snapshot for a local selection.
    /// It does not claim that this snapshot supplies older retained history.
    pub fn capture_live(
        &mut self,
        rows: Vec<Row>,
        nested_bytes: usize,
    ) -> Option<Arc<CachedViewportWindow<Row>>> {
        self.capture_live_at(self.latest_anchor?, rows, nested_bytes)
    }

    /// Pins a validated local frame at its own revision, even when an earlier
    /// history response has already advanced cache metrics beyond that frame.
    pub fn capture_live_at(
        &mut self,
        anchor: TerminalHistoryWindowAnchor,
        rows: Vec<Row>,
        nested_bytes: usize,
    ) -> Option<Arc<CachedViewportWindow<Row>>> {
        if !anchor.is_valid()
            || rows.len() != usize::from(anchor.viewport.rows)
            || self
                .latest_anchor
                .is_none_or(|latest| anchor.revision > latest.revision)
        {
            return None;
        }
        let window = CachedViewportWindow {
            disposition: TerminalViewportDisposition::Exact,
            anchor,
            target_offset_from_bottom: 0,
            first_row_from_live_top: 0,
            rows,
        };
        let pages = self.pages.as_mut()?;
        if !pages.insert(window, nested_bytes) {
            return None;
        }
        pages.pin(anchor, 0)
    }

    fn window_for(
        &self,
        latest: TerminalHistoryWindowAnchor,
        target: u64,
    ) -> Option<(&CachedViewportWindow<Row>, u64)> {
        if let Some(pages) = &self.pages {
            return pages.find(latest, target);
        }
        let window = self.window.as_ref()?;
        let offset = target_in_window_coordinates(latest, window.anchor, target)?;
        window.visible_rows(offset)?;
        Some((window, offset))
    }

    /// Returns the latest authoritative coordinate-space anchor.
    #[must_use]
    pub const fn anchor(&self) -> Option<TerminalHistoryWindowAnchor> {
        self.latest_anchor
    }

    /// Returns the latest client-desired absolute offset.
    #[must_use]
    pub const fn desired_offset_from_bottom(&self) -> u64 {
        self.desired_offset_from_bottom
    }

    /// Returns the last locally presentable absolute offset.
    #[must_use]
    pub const fn presented_offset_from_bottom(&self) -> Option<u64> {
        self.presented_offset_from_bottom
    }

    /// Returns whether one request is currently outstanding.
    #[must_use]
    pub const fn request_pending(&self) -> bool {
        self.pending_query.is_some()
    }

    /// Returns the exact full-height slice for the latest desired target.
    #[must_use]
    pub fn visible_rows(&self) -> Option<&[Row]> {
        let (window, target) =
            self.window_for(self.latest_anchor?, self.desired_offset_from_bottom)?;
        window.visible_rows(target)
    }

    /// Immutable identity of the latest desired full-height slice.
    #[must_use]
    pub fn visible_slice_identity(&self) -> Option<ViewportSliceIdentity> {
        let (window, target) =
            self.window_for(self.latest_anchor?, self.desired_offset_from_bottom)?;
        slice_identity(window, target)
    }

    /// Last complete slice committed for presentation.
    #[must_use]
    pub fn presented_rows(&self) -> Option<&[Row]> {
        let (window, target) =
            self.window_for(self.latest_anchor?, self.presented_offset_from_bottom?)?;
        window.visible_rows(target)
    }

    /// Immutable identity of the last locally presentable slice.
    #[must_use]
    pub fn presented_slice_identity(&self) -> Option<ViewportSliceIdentity> {
        let (window, target) =
            self.window_for(self.latest_anchor?, self.presented_offset_from_bottom?)?;
        slice_identity(window, target)
    }

    /// Observes a live anchor, translating a pinned cache across monotonic append.
    ///
    /// Returns `false` when incompatible identity invalidated cached rows.
    pub fn observe_anchor(&mut self, anchor: TerminalHistoryWindowAnchor) -> bool {
        let observation = self.preview_anchor_observation(anchor);
        self.commit_anchor_observation(observation)
    }

    /// Previews a live-anchor observation without mutating or cloning cached rows.
    #[must_use]
    pub fn preview_anchor_observation(
        &self,
        anchor: TerminalHistoryWindowAnchor,
    ) -> ViewportAnchorObservation {
        if !anchor.is_valid() {
            return ViewportAnchorObservation {
                latest_anchor: None,
                desired_offset_from_bottom: 0,
                presented_offset_from_bottom: None,
                cache_action: AnchorCacheAction::InvalidateAll,
                compatible: false,
                presented_slice_identity: None,
            };
        }
        let Some(previous) = self.latest_anchor else {
            return self.anchor_observation(
                Some(anchor),
                self.desired_offset_from_bottom,
                self.presented_offset_from_bottom,
                AnchorCacheAction::Preserve,
                true,
            );
        };
        if anchor.revision < previous.revision {
            // Live observations can race an already accepted response. Never
            // let an older observation regress the authoritative coordinate
            // space or invalidate a newer complete cache entry.
            return self.anchor_observation(
                self.latest_anchor,
                self.desired_offset_from_bottom,
                self.presented_offset_from_bottom,
                AnchorCacheAction::Preserve,
                true,
            );
        }
        if previous.epoch != anchor.epoch
            || previous.viewport != anchor.viewport
            || anchor.max_offset_from_bottom < previous.max_offset_from_bottom
        {
            return self.anchor_observation(
                Some(anchor),
                self.desired_offset_from_bottom
                    .min(anchor.max_offset_from_bottom),
                None,
                AnchorCacheAction::InvalidateRows,
                false,
            );
        }
        let growth = anchor
            .max_offset_from_bottom
            .saturating_sub(previous.max_offset_from_bottom);
        let mut desired_offset_from_bottom = self.desired_offset_from_bottom;
        let mut presented_offset_from_bottom = self.presented_offset_from_bottom;
        let mut cache_action = AnchorCacheAction::Preserve;
        if growth > 0 && self.desired_offset_from_bottom > 0 {
            desired_offset_from_bottom = self
                .desired_offset_from_bottom
                .saturating_add(growth)
                .min(anchor.max_offset_from_bottom);
            presented_offset_from_bottom = self.presented_offset_from_bottom.map(|offset| {
                offset
                    .saturating_add(growth)
                    .min(anchor.max_offset_from_bottom)
            });
        } else if anchor.revision > previous.revision && self.desired_offset_from_bottom == 0 {
            // Live updates cannot patch a row-addressable prefetch. Keep
            // latest metrics separate from the immutable response snapshot and
            // force the next live gesture to refill instead of presenting stale
            // live rows as the new revision.
            if self.pages.is_none() {
                cache_action = AnchorCacheAction::InvalidateRows;
            }
            presented_offset_from_bottom = None;
        }
        self.anchor_observation(
            Some(anchor),
            desired_offset_from_bottom,
            presented_offset_from_bottom,
            cache_action,
            true,
        )
    }

    /// Commits a previously previewed live-anchor observation.
    ///
    /// Callers must not mutate this cache between preview and commit.
    pub fn commit_anchor_observation(&mut self, observation: ViewportAnchorObservation) -> bool {
        match observation.cache_action {
            AnchorCacheAction::Preserve => {}
            AnchorCacheAction::InvalidateRows => {
                self.window = None;
                if let Some(pages) = &mut self.pages {
                    pages.discard_unpinned();
                }
            }
            AnchorCacheAction::InvalidateAll => {
                self.window = None;
                if let Some(pages) = &mut self.pages {
                    pages.discard_unpinned();
                }
                self.pending_query = None;
            }
        }
        self.latest_anchor = observation.latest_anchor;
        self.desired_offset_from_bottom = observation.desired_offset_from_bottom;
        self.presented_offset_from_bottom = observation.presented_offset_from_bottom;
        observation.compatible
    }

    fn anchor_observation(
        &self,
        latest_anchor: Option<TerminalHistoryWindowAnchor>,
        desired_offset_from_bottom: u64,
        presented_offset_from_bottom: Option<u64>,
        cache_action: AnchorCacheAction,
        compatible: bool,
    ) -> ViewportAnchorObservation {
        let presented_slice_identity = if cache_action == AnchorCacheAction::Preserve {
            latest_anchor.and_then(|latest| {
                let (window, target) = self.window_for(latest, presented_offset_from_bottom?)?;
                slice_identity(window, target)
            })
        } else {
            None
        };
        ViewportAnchorObservation {
            latest_anchor,
            desired_offset_from_bottom,
            presented_offset_from_bottom,
            cache_action,
            compatible,
            presented_slice_identity,
        }
    }

    /// Changes the desired absolute offset and returns local-render/fetch work.
    #[must_use]
    pub fn set_target(&mut self, target_offset_from_bottom: u64) -> ViewportCacheUpdate {
        let Some(anchor) = self.latest_anchor else {
            return ViewportCacheUpdate {
                render_local: false,
                request: None,
            };
        };
        self.desired_offset_from_bottom =
            target_offset_from_bottom.min(anchor.max_offset_from_bottom);
        if let Some(pages) = &mut self.pages {
            pages.touch(anchor, self.desired_offset_from_bottom);
        }
        let render_local = self.visible_rows().is_some();
        let request = if self.pending_query.is_none()
            && (!render_local
                || (self.pages.is_none() && self.needs_prefetch(self.desired_offset_from_bottom)))
        {
            let query = self.make_query(anchor, self.desired_offset_from_bottom);
            self.pending_query = Some(query);
            Some(query)
        } else {
            None
        };
        ViewportCacheUpdate {
            render_local,
            request,
        }
    }

    /// Installs a complete response window and resolves latest-target coalescing.
    pub fn install_window(
        &mut self,
        window: CachedViewportWindow<Row>,
    ) -> Result<ViewportCacheInstall, CachedViewportWindow<Row>> {
        if self.pages.is_some() {
            return Err(window);
        }
        self.install_accounted_window(window, 0)
    }

    /// Installs one correlated reply. `nested_bytes` accounts cell buffers,
    /// strings and other allocations reachable from rows, excluding the row Vec
    /// itself. Multi-page callers must use this method rather than omit accounting.
    pub fn install_accounted_window(
        &mut self,
        window: CachedViewportWindow<Row>,
        nested_bytes: usize,
    ) -> Result<ViewportCacheInstall, CachedViewportWindow<Row>> {
        let Some(pending_query) = self.pending_query else {
            return Err(window);
        };
        if !window.is_valid_for(pending_query) {
            return Err(window);
        }
        let response_anchor = window.anchor;
        self.pending_query = None;
        let pending_target = pending_query.target_offset_from_bottom;
        let previous_anchor = self.latest_anchor.unwrap_or(response_anchor);
        let mut desired = self.desired_offset_from_bottom;
        let same_identity = previous_anchor.epoch == response_anchor.epoch
            && previous_anchor.viewport == response_anchor.viewport;
        let response_is_newer_identity =
            response_anchor.revision.get() > previous_anchor.revision.get();

        if !same_identity && !response_is_newer_identity {
            return Ok(self.follow_latest_after_stale_response(previous_anchor));
        }
        if same_identity
            && response_anchor.max_offset_from_bottom > previous_anchor.max_offset_from_bottom
            && response_anchor.revision.get() <= previous_anchor.revision.get()
        {
            return Ok(self.follow_latest_after_stale_response(previous_anchor));
        }

        let latest_anchor = if same_identity
            && response_anchor.revision.get() <= previous_anchor.revision.get()
            && response_anchor.max_offset_from_bottom <= previous_anchor.max_offset_from_bottom
        {
            previous_anchor
        } else {
            let growth = if same_identity {
                response_anchor
                    .max_offset_from_bottom
                    .saturating_sub(previous_anchor.max_offset_from_bottom)
            } else {
                0
            };
            desired = if desired == pending_target {
                window.target_offset_from_bottom
            } else if same_identity && desired > 0 {
                desired.saturating_add(growth)
            } else {
                desired
            };
            response_anchor
        };

        // A response to a live prefetch cannot update rows after a newer live
        // revision. Leave it dirty and wait for the first actual history miss
        // instead of continuously refreshing under active output.
        if self.pages.is_none()
            && desired == 0
            && response_anchor.revision.get() < latest_anchor.revision.get()
        {
            return Ok(ViewportCacheInstall {
                capacity_rejected: false,
                render_local: false,
                request: None,
            });
        }
        desired = desired.min(latest_anchor.max_offset_from_bottom);
        self.latest_anchor = Some(latest_anchor);
        self.desired_offset_from_bottom = desired;
        let window_target = target_in_window_coordinates(latest_anchor, response_anchor, desired);
        let covers_latest = window_target
            .and_then(|target| window.visible_rows(target))
            .is_some();
        let mut capacity_rejected = false;
        if let Some(pages) = &mut self.pages {
            if !same_identity {
                pages.discard_unpinned();
            }
            capacity_rejected = !pages.insert(window, nested_bytes);
        } else if covers_latest {
            self.window = Some(window);
        }
        let covers_latest = if self.pages.is_some() {
            self.visible_rows().is_some()
        } else {
            covers_latest
        };
        let request = (!covers_latest
            && !capacity_rejected
            && (self.pages.is_none() || desired > 0))
            .then(|| {
                let query = self.make_query(latest_anchor, desired);
                self.pending_query = Some(query);
                query
            });
        Ok(ViewportCacheInstall {
            capacity_rejected,
            render_local: covers_latest,
            request,
        })
    }

    /// Clears transport-pending state while preserving the latest desired target.
    ///
    /// Presentation adapters use this when coalescing drag motion before any
    /// request has actually been written.
    pub fn defer_pending_request(&mut self) {
        self.pending_query = None;
    }

    /// Commits the current complete desired slice only after its renderer has
    /// successfully presented it.
    pub fn commit_visible_presentation(&mut self) -> Option<ViewportSliceIdentity> {
        self.visible_rows()?;
        self.presented_offset_from_bottom = Some(self.desired_offset_from_bottom);
        self.presented_slice_identity()
    }

    /// Clears cached rows and interaction state.
    pub fn invalidate(&mut self) {
        self.latest_anchor = None;
        self.window = None;
        if let Some(pages) = &mut self.pages {
            pages.discard_unpinned();
        }
        self.desired_offset_from_bottom = 0;
        self.presented_offset_from_bottom = None;
        self.pending_query = None;
    }

    /// Discards cached row data while retaining anchor, desired target, and
    /// request correlation.
    pub fn invalidate_rows(&mut self) {
        // The physical host keeps the last complete presentation. Its cached
        // pixels are not part of this reducer; stale rows must not satisfy a
        // later cache hit or be repainted after invalidation.
        self.window = None;
        if let Some(pages) = &mut self.pages {
            pages.discard_unpinned();
        }
        self.presented_offset_from_bottom = None;
    }

    fn follow_latest_after_stale_response(
        &mut self,
        latest_anchor: TerminalHistoryWindowAnchor,
    ) -> ViewportCacheInstall {
        self.latest_anchor = Some(latest_anchor);
        self.desired_offset_from_bottom = self
            .desired_offset_from_bottom
            .min(latest_anchor.max_offset_from_bottom);
        let request = (self.desired_offset_from_bottom > 0).then(|| {
            let query = self.make_query(latest_anchor, self.desired_offset_from_bottom);
            self.pending_query = Some(query);
            query
        });
        ViewportCacheInstall {
            capacity_rejected: false,
            render_local: false,
            request,
        }
    }

    /// Fills missing pages toward travel without moving the desired viewport.
    /// The caller derives a 2–8 screen horizon from velocity/latency, or four
    /// screens for initial warmup. One correlation owner bounds all reads.
    pub fn prefetch_toward(
        &mut self,
        offset: u64,
        older: bool,
        screens: u8,
    ) -> Option<TerminalHistoryWindowQuery> {
        self.pages.as_ref()?;
        if self.pending_query.is_some() {
            return None;
        }
        let anchor = self.latest_anchor?;
        for screen in 1..=screens.clamp(2, 8) {
            let distance = u64::from(anchor.viewport.rows) * u64::from(screen);
            let target = if older {
                offset
                    .saturating_add(distance)
                    .min(anchor.max_offset_from_bottom)
            } else {
                offset.saturating_sub(distance)
            };
            if target == offset {
                break;
            }
            if self.window_for(anchor, target).is_none() {
                let query = self.make_query(anchor, target);
                self.pending_query = Some(query);
                return Some(query);
            }
        }
        None
    }

    fn make_query(
        &self,
        anchor: TerminalHistoryWindowAnchor,
        target: u64,
    ) -> TerminalHistoryWindowQuery {
        let mut query = make_query(anchor, target);
        if self.pages.is_some() {
            let rows = anchor.viewport.rows;
            // A local pixel viewport can remain at either physical edge while
            // its neighbor loads. Include that complete edge viewport so the
            // new page can replace it without replaying an unseen row step.
            if target <= u64::from(rows) {
                query.newer_margin_rows = target as u16;
                query.older_margin_rows = rows * 2 - query.newer_margin_rows;
            } else if anchor.max_offset_from_bottom - target <= u64::from(rows) {
                query.older_margin_rows = (anchor.max_offset_from_bottom - target) as u16;
                query.newer_margin_rows = rows * 2 - query.older_margin_rows;
            }
        }
        query
    }

    fn needs_prefetch(&self, target: u64) -> bool {
        let Some(window) = &self.window else {
            return false;
        };
        let Some(end) = window.end_row_exclusive() else {
            return true;
        };
        let Some(target) = self
            .latest_anchor
            .and_then(|latest| target_in_window_coordinates(latest, window.anchor, target))
            .and_then(|target| i64::try_from(target).ok())
        else {
            return true;
        };
        let visible_start = -target;
        let visible_end = visible_start.saturating_add(i64::from(window.anchor.viewport.rows));
        let threshold = i64::from(window.anchor.viewport.rows.div_ceil(2).max(1));
        let oldest = -i64::try_from(window.anchor.max_offset_from_bottom).unwrap_or(i64::MAX);
        let live_end = i64::from(window.anchor.viewport.rows);
        (window.first_row_from_live_top > oldest
            && visible_start.saturating_sub(window.first_row_from_live_top) < threshold)
            || (end < live_end && end.saturating_sub(visible_end) < threshold)
    }
}

fn slice_identity<Row>(
    window: &CachedViewportWindow<Row>,
    target_offset_from_bottom: u64,
) -> Option<ViewportSliceIdentity> {
    Some(ViewportSliceIdentity {
        epoch: window.anchor.epoch,
        revision: window.anchor.revision,
        first_row_from_live_top: i64::try_from(target_offset_from_bottom)
            .ok()?
            .checked_neg()?,
        viewport: window.anchor.viewport,
    })
}

fn target_in_window_coordinates(
    latest: TerminalHistoryWindowAnchor,
    window: TerminalHistoryWindowAnchor,
    target: u64,
) -> Option<u64> {
    if latest.epoch != window.epoch
        || latest.viewport != window.viewport
        || latest.max_offset_from_bottom < window.max_offset_from_bottom
    {
        return None;
    }
    target.checked_sub(
        latest
            .max_offset_from_bottom
            .saturating_sub(window.max_offset_from_bottom),
    )
}

fn make_query(
    anchor: TerminalHistoryWindowAnchor,
    target_offset_from_bottom: u64,
) -> TerminalHistoryWindowQuery {
    let rows = anchor.viewport.rows;
    let two_rows = rows.saturating_mul(2);
    let (older_margin_rows, newer_margin_rows) = if target_offset_from_bottom <= u64::from(rows) {
        (two_rows, 0)
    } else if anchor
        .max_offset_from_bottom
        .saturating_sub(target_offset_from_bottom)
        <= u64::from(rows)
    {
        (0, two_rows)
    } else {
        (rows, rows)
    };
    TerminalHistoryWindowQuery {
        anchor,
        target_offset_from_bottom,
        older_margin_rows,
        newer_margin_rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Revision;

    fn anchor(revision: u64, maximum: u64) -> TerminalHistoryWindowAnchor {
        TerminalHistoryWindowAnchor {
            epoch: Revision::new(1),
            revision: Revision::new(revision),
            max_offset_from_bottom: maximum,
            viewport: crate::terminal::TerminalSize::new(4, 10),
        }
    }

    fn response(query: TerminalHistoryWindowQuery) -> CachedViewportWindow<i64> {
        let shape = query
            .response_shape(query.anchor)
            .expect("valid test fixture");
        CachedViewportWindow {
            disposition: shape.disposition,
            anchor: query.anchor,
            target_offset_from_bottom: shape.target_offset_from_bottom,
            first_row_from_live_top: shape.first_row_from_live_top,
            rows: (0..shape.row_count)
                .map(|row| {
                    query.anchor.max_offset_from_bottom as i64
                        + shape.first_row_from_live_top
                        + row as i64
                })
                .collect(),
        }
    }
    fn paged(rows: usize, bytes: usize) -> ViewportCache<i64> {
        let mut cache = ViewportCache::with_budget(ViewportCacheBudget { rows, bytes });
        cache.observe_anchor(anchor(1, 100));
        cache
    }
    fn fetch(cache: &mut ViewportCache<i64>, offset: u64, nested: usize) -> ViewportCacheInstall {
        let query = cache
            .set_target(offset)
            .request
            .expect("valid test fixture");
        cache
            .install_accounted_window(response(query), nested)
            .expect("valid test fixture")
    }

    #[test]
    fn multiple_pages_backtrack_locally_and_preserve_proven_rows_on_live_updates() {
        let mut cache = paged(4096, 16 * 1024 * 1024);
        fetch(&mut cache, 12, 0);
        fetch(&mut cache, 20, 0);
        fetch(&mut cache, 28, 0);
        for offset in (8..=32).chain((8..=32).rev()) {
            let update = cache.set_target(offset);
            assert!(update.render_local);
            assert!(update.request.is_none());
            assert_eq!(
                cache.visible_rows().expect("valid test fixture")[0],
                100 - offset as i64
            );
        }
        let _ = cache.set_target(0);
        cache.defer_pending_request();
        cache.observe_anchor(anchor(2, 100));
        assert!(cache.set_target(12).render_local);
        cache.commit_visible_presentation();
        cache.observe_anchor(anchor(3, 107));
        assert_eq!(cache.desired_offset_from_bottom(), 19);
        assert_eq!(
            cache.visible_rows().expect("valid test fixture"),
            &[88, 89, 90, 91]
        );
    }

    #[test]
    fn mutable_live_rows_cannot_be_relabelled_as_history_after_append() {
        let mut cache = paged(4096, 16 * 1024 * 1024);
        fetch(&mut cache, 0, 0);
        let pin = cache.pin_visible_window().expect("valid test fixture");
        cache.observe_anchor(anchor(2, 104));
        // Offset four translates to the old live screen, which was mutable.
        assert!(!cache.set_target(4).render_local);
        cache.defer_pending_request();
        // Historical part of that same immutable page remains reusable.
        assert!(cache.set_target(8).render_local);
        assert_eq!(
            cache.visible_rows().expect("valid test fixture"),
            &[96, 97, 98, 99]
        );
        assert_eq!(
            pin.visible_rows(0).expect("valid test fixture"),
            &[100, 101, 102, 103]
        );
    }

    #[test]
    fn row_budget_evicts_lru_but_never_pinned_selection_source() {
        let mut cache = paged(24, usize::MAX);
        fetch(&mut cache, 12, 0);
        let selected = cache.pin_visible_window().expect("valid test fixture");
        fetch(&mut cache, 28, 0);
        fetch(&mut cache, 44, 0);
        assert_eq!(cache.retained_usage().expect("valid test fixture").rows, 24);
        assert!(cache.set_target(12).render_local);
        assert_eq!(
            selected.visible_rows(12).expect("valid test fixture"),
            &[88, 89, 90, 91]
        );
        assert!(!cache.set_target(28).render_local);
        cache.defer_pending_request();
        drop(selected);
        fetch(&mut cache, 60, 0);
        assert!(cache.retained_usage().expect("valid test fixture").rows <= 24);
    }

    #[test]
    fn pinned_capacity_rejection_clears_request_without_eviction_or_retry_loop() {
        let mut cache = paged(12, usize::MAX);
        fetch(&mut cache, 12, 0);
        let pin = cache.pin_visible_window().expect("valid test fixture");
        let rejected = fetch(&mut cache, 40, 0);
        assert!(rejected.capacity_rejected);
        assert!(!rejected.render_local);
        assert!(rejected.request.is_none());
        assert!(!cache.request_pending());
        assert_eq!(
            pin.visible_rows(12).expect("valid test fixture"),
            &[88, 89, 90, 91]
        );
        drop(pin);
        assert!(!fetch(&mut cache, 40, 0).capacity_rejected);
    }

    #[test]
    fn nested_allocation_budget_is_admitted_and_oversized_response_is_atomic() {
        let mut cache = paged(4096, 1800);
        assert!(!fetch(&mut cache, 12, 1000).capacity_rejected);
        let before = cache.retained_usage().expect("valid test fixture");
        assert!(before.bytes >= 1000);
        assert!(before.bytes <= 1800);
        assert!(fetch(&mut cache, 40, 1800).capacity_rejected);
        assert_eq!(cache.retained_usage().expect("valid test fixture"), before);
        assert!(!fetch(&mut cache, 40, 1000).capacity_rejected);
        assert!(cache.retained_usage().expect("valid test fixture").bytes <= 1800);
    }

    #[test]
    fn bounded_prefetch_has_one_owner_and_gives_latest_miss_priority() {
        let mut cache = paged(4096, 16 * 1024 * 1024);
        let first = cache
            .prefetch_toward(0, true, 4)
            .expect("valid test fixture");
        assert!(cache.prefetch_toward(0, true, 4).is_none());
        assert_eq!(cache.desired_offset_from_bottom(), 0);
        let _ = cache.set_target(80);
        let next = cache
            .install_accounted_window(response(first), 0)
            .expect("valid test fixture");
        assert_eq!(
            next.request
                .expect("valid test fixture")
                .target_offset_from_bottom,
            80
        );
        cache
            .install_accounted_window(response(next.request.expect("valid test fixture")), 0)
            .expect("valid test fixture");
        assert!(cache.visible_rows().is_some());
        assert!(cache.set_target(4).render_local); // completed speculative page was kept
    }

    #[test]
    fn warmup_fills_four_screens_without_duplicate_reads() {
        let mut cache = paged(4096, 16 * 1024 * 1024);
        let mut queries = 0;
        while let Some(query) = cache.prefetch_toward(0, true, 4) {
            queries += 1;
            assert!(queries <= 3);
            assert!(response(query).rows.len() <= MAX_HISTORY_WINDOW_ROWS);
            cache
                .install_accounted_window(response(query), 0)
                .expect("valid test fixture");
        }
        assert_eq!(queries, 2);
        for offset in 4..=16 {
            assert!(cache.set_target(offset).render_local);
        }
    }

    #[test]
    fn epoch_change_rejects_joining_but_preserves_frozen_pins_and_accounting() {
        let mut cache = paged(24, usize::MAX);
        fetch(&mut cache, 12, 0);
        let pin = cache.pin_visible_window().expect("valid test fixture");
        let changed = TerminalHistoryWindowAnchor {
            epoch: Revision::new(2),
            ..anchor(2, 100)
        };
        assert!(!cache.observe_anchor(changed));
        assert!(!cache.set_target(12).render_local);
        assert_eq!(cache.retained_usage().expect("valid test fixture").rows, 12);
        assert_eq!(
            pin.visible_rows(12).expect("valid test fixture"),
            &[88, 89, 90, 91]
        );
        cache.invalidate();
        assert_eq!(cache.retained_usage().expect("valid test fixture").rows, 12);
        drop(pin);
        cache.invalidate();
        assert_eq!(cache.retained_usage().expect("valid test fixture").rows, 0);
    }

    #[test]
    fn cache_hit_slices_exact_height_without_a_request() {
        let mut cache = ViewportCache::new();
        assert!(cache.observe_anchor(anchor(1, 100)));
        assert!(cache.set_target(50).request.is_some());
        let installed = cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 100),
                target_offset_from_bottom: 50,
                first_row_from_live_top: -54,
                rows: (-54_i32..-42).collect(),
            })
            .expect("valid response");
        assert!(installed.render_local);
        assert_eq!(cache.visible_rows(), Some(&[-50, -49, -48, -47][..]));
        let update = cache.set_target(51);
        assert!(update.render_local);
        assert!(update.request.is_none());
        assert_eq!(cache.visible_rows(), Some(&[-51, -50, -49, -48][..]));
    }

    #[test]
    fn pending_request_keeps_only_the_latest_absolute_target() {
        let mut cache = ViewportCache::<i32>::new();
        cache.observe_anchor(anchor(1, 20));
        assert!(cache.set_target(2).request.is_some());
        assert!(cache.set_target(12).request.is_none());
        let installed = cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 20),
                target_offset_from_bottom: 2,
                first_row_from_live_top: -10,
                rows: (-10_i32..2).collect(),
            })
            .expect("valid stale response");
        assert!(!installed.render_local);
        assert_eq!(
            installed
                .request
                .expect("latest follow-up")
                .target_offset_from_bottom,
            12
        );
        assert!(cache.request_pending());
    }

    #[test]
    fn monotonic_append_translates_pinned_coordinates() {
        let mut cache = ViewportCache::new();
        cache.observe_anchor(anchor(1, 8));
        let _ = cache.set_target(3);
        cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 8),
                target_offset_from_bottom: 3,
                first_row_from_live_top: -8,
                rows: (-8_i32..1).collect(),
            })
            .expect("valid response");
        let presented = cache
            .commit_visible_presentation()
            .expect("complete slice is presented");
        assert!(cache.observe_anchor(anchor(2, 10)));
        assert_eq!(cache.desired_offset_from_bottom(), 5);
        assert_eq!(cache.visible_rows(), Some(&[-3, -2, -1, 0][..]));
        assert_eq!(cache.presented_slice_identity(), Some(presented));
        assert_eq!(
            cache.visible_slice_identity(),
            Some(ViewportSliceIdentity {
                epoch: Revision::new(1),
                revision: Revision::new(1),
                first_row_from_live_top: -3,
                viewport: crate::terminal::TerminalSize::new(4, 10),
            })
        );
    }

    #[test]
    fn anchor_observation_stages_metadata_until_explicit_commit() {
        let mut cache = ViewportCache::new();
        cache.observe_anchor(anchor(1, 8));
        let _ = cache.set_target(3);
        cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 8),
                target_offset_from_bottom: 3,
                first_row_from_live_top: -8,
                rows: (-8_i32..1).collect(),
            })
            .expect("valid response");
        let presented = cache
            .commit_visible_presentation()
            .expect("complete slice is presented");
        let pending_before = cache.request_pending();
        let mut changed = anchor(2, 6);
        changed.epoch = Revision::new(2);

        let observation = cache.preview_anchor_observation(changed);
        assert!(!observation.is_compatible());
        assert_eq!(observation.presented_slice_identity(), None);
        assert_eq!(cache.anchor(), Some(anchor(1, 8)));
        assert_eq!(cache.presented_slice_identity(), Some(presented));
        assert!(cache.presented_rows().is_some());

        assert!(!cache.commit_anchor_observation(observation));
        assert_eq!(cache.anchor(), Some(changed));
        assert_eq!(cache.desired_offset_from_bottom(), 3);
        assert!(cache.presented_rows().is_none());
        assert_eq!(cache.request_pending(), pending_before);
    }

    #[test]
    fn incompatible_identity_invalidates_hits_but_keeps_transport_correlation() {
        let mut cache = ViewportCache::<i32>::new();
        cache.observe_anchor(anchor(1, 8));
        let _ = cache.set_target(3);
        let mut changed = anchor(2, 8);
        changed.epoch = Revision::new(2);
        assert!(!cache.observe_anchor(changed));
        assert!(cache.visible_rows().is_none());
        assert!(cache.request_pending());
    }

    #[test]
    fn full_window_range_must_fit_even_when_the_target_slice_would_fit() {
        let window = CachedViewportWindow {
            disposition: TerminalViewportDisposition::Exact,
            anchor: anchor(1, 8),
            target_offset_from_bottom: 8,
            first_row_from_live_top: -9,
            rows: (-9_i32..0).collect(),
        };
        assert!(window.visible_rows(8).is_some());
        assert!(!window.is_valid());
    }

    #[test]
    fn cache_hit_near_an_edge_renders_and_schedules_one_bounded_prefetch() {
        let mut cache = ViewportCache::new();
        cache.observe_anchor(anchor(1, 20));
        assert!(cache.set_target(4).request.is_some());
        cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 20),
                target_offset_from_bottom: 4,
                first_row_from_live_top: -12,
                rows: (-12_i32..0).collect(),
            })
            .expect("valid narrow response");

        let update = cache.set_target(5);
        assert!(update.render_local);
        let query = update.request.expect("edge low-water prefetch");
        assert_eq!(query.target_offset_from_bottom, 5);
        assert!(
            u32::from(query.older_margin_rows) + u32::from(query.newer_margin_rows)
                <= u32::from(query.anchor.viewport.rows) * 2
        );
        assert!(cache.set_target(6).request.is_none());
    }

    #[test]
    fn multipage_edge_queries_also_cover_the_waiting_full_viewport() {
        for (target, edge) in [(1, 0), (4, 0), (96, 100), (99, 100)] {
            let mut cache = ViewportCache::<i64>::with_budget(ViewportCacheBudget {
                rows: 128,
                bytes: 64 * 1024,
            });
            cache.observe_anchor(anchor(1, 100));
            let query = cache.set_target(target).request.expect("edge query");
            assert_eq!(query.older_margin_rows + query.newer_margin_rows, 8);
            let window = response(query);
            assert!(
                window.visible_rows(edge).is_some(),
                "handoff retains a complete edge viewport"
            );
            cache
                .install_accounted_window(window, 0)
                .expect("bounded window");
            let cached = cache.set_target(edge);
            assert!(cached.render_local && cached.request.is_none());
        }
    }

    #[test]
    fn query_margins_are_directional_and_never_exceed_two_screens() {
        let near_live = make_query(anchor(1, 100), 1);
        assert_eq!(
            (near_live.older_margin_rows, near_live.newer_margin_rows),
            (8, 0)
        );
        let middle = make_query(anchor(1, 100), 50);
        assert_eq!((middle.older_margin_rows, middle.newer_margin_rows), (4, 4));
        let near_oldest = make_query(anchor(1, 100), 99);
        assert_eq!(
            (near_oldest.older_margin_rows, near_oldest.newer_margin_rows),
            (0, 8)
        );
        assert!(near_live.is_valid() && middle.is_valid() && near_oldest.is_valid());
    }

    #[test]
    fn stale_same_identity_response_keeps_latest_anchor_and_translates_slice() {
        let mut cache = ViewportCache::new();
        cache.observe_anchor(anchor(1, 8));
        assert!(cache.set_target(3).request.is_some());
        assert!(cache.observe_anchor(anchor(2, 10)));

        let installed = cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 8),
                target_offset_from_bottom: 3,
                first_row_from_live_top: -8,
                rows: (-8_i32..1).collect(),
            })
            .expect("valid stale snapshot");

        assert!(installed.render_local);
        assert_eq!(cache.anchor(), Some(anchor(2, 10)));
        assert_eq!(cache.desired_offset_from_bottom(), 5);
        assert_eq!(cache.visible_rows(), Some(&[-3, -2, -1, 0][..]));
    }

    #[test]
    fn newer_live_revision_discards_an_unpatchable_live_prefetch() {
        let mut cache = ViewportCache::new();
        cache.observe_anchor(anchor(1, 8));
        assert!(cache.set_target(0).request.is_some());
        assert!(cache.observe_anchor(anchor(2, 8)));

        let installed = cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 8),
                target_offset_from_bottom: 0,
                first_row_from_live_top: -8,
                rows: (-8_i32..4).collect(),
            })
            .expect("structurally valid stale prefetch");

        assert!(!installed.render_local);
        assert!(installed.request.is_none());
        assert_eq!(cache.anchor(), Some(anchor(2, 8)));
        assert!(cache.visible_rows().is_none());
    }

    #[test]
    fn stale_incompatible_response_preserves_latest_and_refetches() {
        let mut cache = ViewportCache::<i32>::new();
        cache.observe_anchor(anchor(1, 8));
        assert!(cache.set_target(3).request.is_some());
        let mut latest = anchor(2, 9);
        latest.epoch = Revision::new(2);
        assert!(!cache.observe_anchor(latest));

        let follow_up = cache
            .install_window(CachedViewportWindow {
                disposition: TerminalViewportDisposition::Exact,
                anchor: anchor(1, 8),
                target_offset_from_bottom: 3,
                first_row_from_live_top: -8,
                rows: (-8_i32..1).collect(),
            })
            .expect("structurally valid old-identity response");

        assert!(!follow_up.render_local);
        assert_eq!(cache.anchor(), Some(latest));
        assert_eq!(
            follow_up.request.expect("latest identity refetch").anchor,
            latest
        );
    }

    #[test]
    fn stale_anchor_observation_never_regresses_latest_cache_metrics() {
        let mut cache = ViewportCache::<i32>::new();
        assert!(cache.observe_anchor(anchor(5, 12)));

        assert!(cache.observe_anchor(anchor(4, 8)));

        assert_eq!(cache.anchor(), Some(anchor(5, 12)));
        assert_eq!(cache.desired_offset_from_bottom(), 0);
    }

    #[test]
    fn response_must_match_the_full_outstanding_query_description() {
        let mut cache = ViewportCache::<i32>::new();
        cache.observe_anchor(anchor(5, 8));
        let query = cache.set_target(3).request.expect("bounded query");

        let predating = CachedViewportWindow {
            disposition: TerminalViewportDisposition::Exact,
            anchor: anchor(4, 8),
            target_offset_from_bottom: 3,
            first_row_from_live_top: -8,
            rows: (-8_i32..1).collect(),
        };
        assert!(predating.is_valid());
        assert!(cache.install_window(predating).is_err());
        assert_eq!(cache.anchor(), Some(anchor(5, 8)));
        assert!(cache.request_pending());

        let wrong_margin_range = CachedViewportWindow {
            disposition: TerminalViewportDisposition::Exact,
            anchor: anchor(5, 8),
            target_offset_from_bottom: query.target_offset_from_bottom,
            first_row_from_live_top: -7,
            rows: (-7_i32..1).collect(),
        };
        assert!(wrong_margin_range.is_valid());
        assert!(cache.install_window(wrong_margin_range).is_err());
        assert!(cache.request_pending());
    }
}
