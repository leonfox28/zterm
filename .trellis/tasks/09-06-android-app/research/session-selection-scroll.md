# Session Management, Selection, and Scrolling

Researched on 2026-09-06 after the user explicitly added all three capabilities
to the first release. This supersedes the prior first-release exclusions in
the task's PRD/design/plan. No Android implementation or runtime check has run.
Updated after the user's follow-up: the visible-viewport-only selection proposal
is superseded by cross-screen selection and an explicit native multi-page cache.

## Existing Session Contract

- `proto/zterm/v2/session.proto` already defines list, operation lease, create,
  rename, close, and takeover. Create supplies a name, optional directory,
  viewport, and base colors; rename/close target exact Session IDs. No new wire
  message is needed for the requested management operations.
- `crates/daemon/src/client/ipc.rs:320` owns list decoding; `:383`, `:420`, and
  `:451` construct create/rename/close with one shared mutation request helper.
  These currently live in a Unix client and need portable extraction together
  with the attachment client, not a Kotlin reimplementation.
- `crates/daemon/src/client/ipc.rs:469` serializes mutation state per exact target,
  allocates operation IDs under the host lease, and resets its lease when the
  result is `OperationOutcomeUnknown`. Transport retry classification and
  exact operation identity belong to this owner. A UI retry must not blindly
  call Create again after an ambiguous response.
- `crates/daemon/src/session.rs` owns the actual login-shell Session, name
  collision checks, host working-directory validation, rename, and closure.
  Android remains a remote controller; switching/detaching leaves that host
  process alive, whereas confirmed Close deliberately terminates it.

## Existing History Contract

- `.trellis/spec/backend/local-daemon-ipc.md:270` specifies a single bounded
  semantic history operation: kind 317 request and correlated kind 318 response
  validated against the exact saved query. No server-side attachment viewport,
  extra pager API, fallback representation, or new wire version is needed.
- `crates/core/src/viewport_cache.rs` exposes the reusable window/cache reducer,
  anchor translation, request coalescing, and separate desired/presented state.
  A cache hit resolves locally; at most one query is pending and newer movement
  coalesces. Android draws at native display cadence rather than awaiting a
  round trip for every touch move.
- Inspection of `ViewportCache` storage and `install_window` confirms that the
  current implementation retains only one window. `preview_anchor_observation`
  also invalidates that whole window on a newer live revision at offset zero.
  It is useful shared logic, but is not already a multi-page mobile history
  store. The Android plan must extend its storage/identity/prefetch policy while
  preserving one correlation owner and desktop behavior.
- `crates/core/src/terminal.rs:21` bounds a history window at 240 rows. Query
  margins are also bounded relative to the viewport. This is a window bound,
  not a 240-row limit on the host's entire scrollback: subsequent queries can
  browse other retained windows.
- `.trellis/spec/backend/local-daemon-ipc.md:499` distinguishes received cache
  targets from actually presented rows/metrics and preserves complete pixels
  through pending queries, resync, and return-live. A frame token across FFI
  must preserve this distinction without tying protocol ACK to visible drawing.
- `.trellis/spec/guides/cross-platform-thinking-guide.md:78` makes pixel/touch
  physics and native vsync Android policy; desktop ANSI timing/gutter rules do
  not belong in core or the mobile View.
- `crates/terminal/src/model.rs:311` exposes live history extent/epoch;
  `:324` projects explicit history windows. Live deltas alone are not a complete
  ordered scrollback feed, so recording old visible frames would fabricate an
  incomplete local history. Reuse semantic history queries to populate cache.
- `crates/terminal/src/model.rs:396` can advance the history epoch when retained
  history decreases or reaches capacity. At a full buffer, continuing grid input
  may cause frequent invalidation. Test this explicitly; do not promise that all
  old pages remain joinable forever or weaken identity checks to reduce reads.
  Already captured selection rows can remain a labeled local immutable source
  for copying; incompatible new pages cannot be stitched onto it.

## Selection and Gesture Ownership

- `crates/core/src/terminal_selection.rs` already normalizes forward/reverse
  visible-row ranges, expands wide glyph endpoints, handles combining text and
  wrapped-row joining, and extracts under the atomic shared clipboard cap.
  Reuse these APIs rather than selecting Android-shaped glyph positions or
  flattening/reflowing the host grid into a TextView.
- `.trellis/tasks/09-04-terminal-selection-copy/prd.md` and
  `.trellis/spec/backend/local-daemon-ipc.md:416` establish attachment-local
  selection and exact presented-source identity. They exclude selection state
  from daemon model/wire and require one pointer-event owner, with invalid
  coordinates cleared instead of reused after resize/reconnect/screen changes.
- The Android design uses long-press/handles and an explicit Select text action
  for local ownership even under child mouse reporting. Native scrolling follows
  the shared mode-based routing semantics; local history fling must not send
  inertia-driven input to a remote program. Selection pins captured semantic
  rows, while live processing/ACK and viewport movement continue independently.
- The old visible-viewport-only restriction allowed scrolling first and then
  selecting, but prevented keeping one endpoint offscreen and extending the
  other via edge scrolling. It was a scope reduction, not an Android limit.
  The revised first release includes cross-screen/page selection and edge
  auto-scroll, with stable content anchors and selected-row pinning.
- Core's current `u16` visible coordinates cannot identify a row across moving
  pages. Factor a bounded content-row accessor/range into the shared extraction
  owner; keep desktop's facade and Unicode/wrap/cap behavior. A page boundary
  must not insert a newline or allow a missing middle range to be skipped.
- Mobile uses multiple validated pages with bounded eviction, recent-history
  warmup, and directional prefetch. Cache hits render locally, including reverse
  travel; request counts follow missing ranges rather than gesture/frame counts.
  Initial 4,096-row/16-MiB budgets and a four-screen warmup are proposed tuning
  values for execution measurements, not guaranteed device performance. Pins
  count toward the same semantic-store budget. Full-history export and an
  unlimited/offline-complete archive remain out of scope.
- Host-driven kind-322 effects are a distinct existing capability. Local Copy
  is now supported through Android's clipboard, while unsolicited host effects
  remain discarded/nonpersistent in this increment as previously scoped.

## Android Primary Sources

Android provides gesture classification for long-press, scroll, and fling.
Custom Views can use `OverScroller` to calculate animation positions; the App
still decides how those positions map to semantic rows and input ownership.
The native scroller computes offsets locally and does not fetch terminal data.
Use fractional pixel translation/overscan for touch smoothness while retaining
host semantic row geometry for requests, selection identity, and hit testing.
These APIs support the proposed native View approach, not a runtime proof of
terminal smoothness or correct nested-TUI routing.

Sources: [Gesture detection](https://developer.android.com/develop/ui/views/touch-and-input/gestures/detector),
[Scroll animation](https://developer.android.com/develop/ui/views/touch-and-input/gestures/scroll).

Android's clipboard framework writes a `ClipData` value through
`ClipboardManager`; Android 13 and later provide system feedback on copying.
The custom terminal owns its handles/range UI, and should not duplicate system
copy feedback. Clipboard contents remain local unless the user explicitly
pastes them somewhere.

Source: [Copy and paste in Views](https://developer.android.com/develop/ui/views/touch-and-input/copy-paste).

## Required Execution Evidence

Extend emulator and Xiaomi acceptance with Session creation/rename/close,
duplicate/ambiguous mutation outcomes, explicit detach/switch, history across
multiple windows, cached reverse travel without duplicate requests, delayed
prefetch/misses, append/full-buffer epoch churn, and each child mode branch.
Copy a selection crossing three screen heights and two network-window boundaries,
including wrap/Unicode at page seams, late replies after cancelled auto-scroll,
and pin/memory pressure. Record request/cache counters, frame timings, and actual
retained memory. Existing Rust tests prove shared semantics; they do not certify
native touch, clipboard, or phone behavior.
