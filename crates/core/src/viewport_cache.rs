//! Renderer-neutral bounded viewport-window cache and request coalescer.

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
            desired_offset_from_bottom: 0,
            presented_offset_from_bottom: None,
            pending_query: None,
        }
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
        let window = self.window.as_ref()?;
        let target = target_in_window_coordinates(
            self.latest_anchor?,
            window.anchor,
            self.desired_offset_from_bottom,
        )?;
        window.visible_rows(target)
    }

    /// Returns the immutable identity of the latest desired full-height slice.
    #[must_use]
    pub fn visible_slice_identity(&self) -> Option<ViewportSliceIdentity> {
        let window = self.window.as_ref()?;
        let target = target_in_window_coordinates(
            self.latest_anchor?,
            window.anchor,
            self.desired_offset_from_bottom,
        )?;
        window.visible_rows(target)?;
        slice_identity(window, target)
    }

    /// Returns the last complete slice committed for presentation.
    #[must_use]
    pub fn presented_rows(&self) -> Option<&[Row]> {
        let window = self.window.as_ref()?;
        let target = target_in_window_coordinates(
            self.latest_anchor?,
            window.anchor,
            self.presented_offset_from_bottom?,
        )?;
        window.visible_rows(target)
    }

    /// Returns the immutable identity of the last locally presentable slice.
    #[must_use]
    pub fn presented_slice_identity(&self) -> Option<ViewportSliceIdentity> {
        let window = self.window.as_ref()?;
        let target = target_in_window_coordinates(
            self.latest_anchor?,
            window.anchor,
            self.presented_offset_from_bottom?,
        )?;
        window.visible_rows(target)?;
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
            cache_action = AnchorCacheAction::InvalidateRows;
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
            AnchorCacheAction::InvalidateRows => self.window = None,
            AnchorCacheAction::InvalidateAll => {
                self.window = None;
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
            self.window.as_ref().and_then(|window| {
                let target = target_in_window_coordinates(
                    latest_anchor?,
                    window.anchor,
                    presented_offset_from_bottom?,
                )?;
                window.visible_rows(target)?;
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
        let render_local = self.visible_rows().is_some();
        let request = if self.pending_query.is_none()
            && (!render_local || self.needs_prefetch(self.desired_offset_from_bottom))
        {
            let query = make_query(anchor, self.desired_offset_from_bottom);
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
        if desired == 0 && response_anchor.revision.get() < latest_anchor.revision.get() {
            return Ok(ViewportCacheInstall {
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
        if covers_latest {
            self.window = Some(window);
        }
        let request = (!covers_latest).then(|| {
            let query = make_query(latest_anchor, desired);
            self.pending_query = Some(query);
            query
        });
        Ok(ViewportCacheInstall {
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
            let query = make_query(latest_anchor, self.desired_offset_from_bottom);
            self.pending_query = Some(query);
            query
        });
        ViewportCacheInstall {
            render_local: false,
            request,
        }
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
