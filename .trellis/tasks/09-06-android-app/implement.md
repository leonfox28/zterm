# Android App Implementation Plan

Status: in_progress; complete PRD/design/plan approved and existing host baseline
confirmed by the user on 2026-09-07.
Repository baseline refreshed: `de25a387fb512dd06ae0bb7f8e3afcd4bbf5f6db`.
Current implementation and runtime evidence: `research/implementation-progress.md`.

Execute inline in the main session. No implement/check sub-agent dispatch.
JSONL reference indexes are populated for task validation; they do not change
inline execution or authorize sub-agents. Read `trellis-before-dev` and the affected specs before
product edits. This task remains the implementation target; the steps below
have explicit ordering, not an implied parent/child dependency graph.

## Phase 0 — Review and Existing Host Prerequisite

- [x] Capture all agreed product decisions, including the three pairing entries,
  Session management, cross-screen touch selection/copy, cached native history
  scrolling with prefetch, eight shortcut
  keys, background connection retention, local emulator, Xiaomi 17 Pro Max,
  APK delivery without store publication, input continuity, font size/landscape,
  local saved-device removal, and first-connection Session targeting.
- [x] Research the current client/host boundary and official Android/UniFFI
  integration constraints; persist sources under `research/`.
- [x] Produce `prd.md`, `design.md`, and this plan for final review.
- [x] Receive explicit approval of the latest final planning summary:
  "好，开始开发吧，不要用子代理" (2026-09-07). Work inline, without sub-agents.
- [x] Task activated by the user with `task.py start --allow-empty-context`.
  The current workflow is in_progress; no further phase approval is required.
- [x] Refresh this branch onto current `origin/main` without losing the approved
  task/prototype or project Penpot setup. Review relevant upstream changes;
  `research/mainline-refresh.md` records the `0.1.24` baseline and impact.
- [x] Reconcile the host prerequisite with the user's 2026-09-07 confirmation
  that macOS/Linux basic functionality is already verified. Accept that report
  for Android development, with local `zterm` and remote `zterm connect dev`
  as the available integration entries. Read-only Session listing succeeds on
  both. Record this in the existing migration owner; do not claim separately
  observed six-path runs or require their repetition before App work.

Existing constraint:
`.trellis/spec/guides/cross-platform-thinking-guide.md:100-115`.
Historical detailed matrix and user-confirmation addendum:
`.trellis/tasks/09-02-migrate-alacritty-terminal/implement.md:1350-1353`.

## Phase 1 — Reproducible App and Native Bridge Gate

Depends on Phase 0's host prerequisite.

- [x] Add `apps/android/` with Gradle wrapper/checksum, version catalog, App
  manifest, dependency locks, and generated-source/native-library task wiring.
  Use the design's SDK/ABI/version baseline, with no developer-specific paths.
- [x] Install/select NDK 28.2.13676358 alongside the existing SDK contents and
  pin cargo-ndk 4.1.2. Keep Rust 1.98.0 and Iroh 1.0.3 unchanged.
- [x] Add the minimal `zterm-android` bridge and workspace-local UniFFI binding
  generator. Build host metadata for Kotlin generation and the arm64 Android
  `.so`; include the matching JNA Android AAR.
- [x] Verify native load, typed round-trip, async operation completion, explicit
  cancellation, error translation, and native object lifetime on the emulator.
  Canceling a UI subscriber must not dispose the App runtime.
- [x] Audit all packaged `.so` ELF load-segment alignment and APK zip alignment.
  Verify 16 KB readiness for prebuilt dependencies as well as the Rust library.
- [x] Add repeatable entry points in `tools/android/` and `Justfile` for build,
  bindings, checks, and local emulator installation. Register proper task inputs
  and outputs rather than depending on a manually copied `.so`.
- [x] Resolve any leaf dependency/AGP/FFI compatibility issue at this gate and
  update the exact locks/source record. A change to architecture or user-visible
  scope requires a revised design review, not an undocumented fallback.

Gate: an installable arm64 scaffold loads its actual native library and passes
the bridge checks. Do not perform the large shared extraction before this gate.

## Phase 2 — Shared Client Ownership and Desktop Regression

Depends on Phase 1. Keep the changes reviewable by separating extraction from
new Android behavior.

- [x] Add `zterm-client` and move the attachment protocol/reconnect owner out of
  the daemon client module. Introduce only the transport factory, typed DTOs,
  errors, and local-lifecycle capabilities required by real callers.
- [x] Keep Unix sockets, opaque local envelopes, daemon restart, and local-only
  path events in the desktop adapter. Preserve daemon facade imports while
  migrating tests/callers to one shared implementation.
- [x] Extract Session unary list/create/rename/close and per-target operation
  lease/replay handling from `client/ipc.rs`. Preserve pre-/post-write failure
  classification and `OperationOutcomeUnknown`; keep local admin APIs separate.
- [x] Extract reusable controller pairing, transcript/normal-confirmation, and
  Hello/Welcome validation. Keep daemon inbound authorization/Session/store
  ownership intact. Android and desktop must invoke the same extracted rules.
- [x] Implement the Android outbound Iroh adapter with one App Endpoint,
  connection reuse, proper ALPN separation, authorized relay hints, and the
  current frame/deadline/resource limits.
- [x] Move or expose the small semantic surface owner and mode-aware keyboard
  encoding needed by the App. Preserve desktop mouse routing. Reuse core cache
  and
  text-range values directly. Keep desktop parsing/prefix/presentation code in
  CLI; do not import an ANSI parser or host engine into the App.
- [x] Extend `tests/terminal-dependency-policy.sh` for client/bridge and mobile
  target graphs. Preserve its one authoritative Alacritty dependency path.
- [x] Run existing desktop pairing, client IPC/tunnel, mutation/replay,
  snapshot/delta/history, clipboard, takeover, and reconnect tests against the
  moved owner. Add focused tests only for adapter equivalence and new Android
  transport boundaries; preserve the existing desktop selection/copy behavior.

Gate: existing desktop behavior and protocol fixtures pass, and the Android
dependency graph contains neither host/PTY crates nor the desktop presenter.
Rollback unit: the extraction plus its desktop facade/caller changes together;
never retain a second competing Session state machine after a partial revert.

## Phase 3 — Pairing and Durable Mobile Identity

Depends on Phase 2.

- [x] Implement the App identity store using Keystore-wrapped seed material,
  atomic no-backup state, and explicit backup/transfer exclusions. Persist the
  identity before any pairing request; do not regenerate on read/decrypt errors.
- [x] Persist known hosts and the last selected exact host/Session for recovery.
  Complete normal authorization confirmation after ambiguous pairing outcomes;
  announce success only after known-host storage succeeds.
- [x] Add the explicit `--qr` and `--qr-image` host output modes described in the
  design. Preserve default text stdout, ticket format, expiry, and one-time
  semantics. Test that decoded QR/image content equals the original ticket.
- [x] Build the custom scan screen with camera preview, **lower-left album**,
  and **lower-right manual ticket** actions above system insets.
- [x] Implement manual credentials as a scanner-owned dialog, not a route.
  Include only the field, wide centered Connect aligned with it, and top-right
  close. Omit title and Paste button; keep standard text-field paste.
  Cover typed/pasted input, empty/invalid input, inline error retention, close/
  outside/Back dismissal, IME insets, and camera pause/resume. Use the existing
  pairing owner; opening the dialog must not read the clipboard or submit.
- [x] Integrate CameraX, bundled QR recognition, and image-only system selection.
  Launch the Android picker directly; rely on its system fallback when needed.
  Do not build an App album route, gallery grid, or custom album navigator.
  Handle camera permission denial, picker cancellation, unreadable/multiple
  codes, invalid/expired tickets, and single-attempt ownership.
- [x] Connect saved-host taps and pairing completion directly toward terminal;
  restore a recorded Session by exact ID. Without one, implement zero/one/many
  as explicit create / sole target / expanded title list. A stale recorded ID
  shows ended. Handle unreachable, revoked, and occupied states without implicit
  create, retarget, or takeover; preserve selection identity during list races.
- [ ] Add card-long-press confirmation for local saved-device removal. Atomically
  remove its known-host/route/recent records, cancel stale callbacks, and retain
  App identity/other hosts. Verify cancel, storage failure, restart, late events,
  re-pair, and that no remote close/revoke operation is sent.
- [x] Build the conditional top resume card and settings navigation. Remove
  the home badge/heading/slogan and scan-page instructional paragraphs.
- [x] Add independent language (`system / zh / en`) and theme
  (`system / dark / light`) preferences, both defaulting to system. Show two
  single-choice groups; apply immediately and persist. Localize App resources
  and accessibility labels without changing remote content/identifiers.
- [ ] Apply semantic dark/light colors across screens, dialogs, system bars,
  terminal default colors, selection, and shortcut keys, preserving explicit
  program colors. Verify all nine preference combinations, OS-follow changes,
  unsupported-locale fallback, restart persistence, and connection/scroll/
  selection retention during locale/theme recomposition or recreation.
- [x] Add a horizontal 8–16 terminal font slider, step 1 (default 12), with immediate
  sample preview. Support portrait/landscape under system rotation settings;
  test IME shown/hidden, measured grid/shortcut insets, coalesced resize,
  selection invalidation, and Session/connection retention on rotation.
- [ ] Cover concise loading/retry/error states from UI-10 in the existing
  routes/dialogs, including camera denial with album/manual still usable,
  expired/invalid credential with retained input, and no-QR image retry.
- [x] Exercise camera, saved-image, and manual-ticket routes through the actual
  Rust pairing adapter in the emulator. Configure a real QR camera fixture;
  decoder unit tests or album selection alone do not prove the camera route.

Gate: all PAIR requirements pass; an App restart retains the same identity and
paired host. Use synthetic or ephemeral test material in evidence, avoiding
live bearer values in committed logs/screenshots.

## Phase 4 — Session Management, Terminal, and Background Retention

Depends on Phase 3.

- [x] Implement the host Session list, explicit create/open, rename, confirmed
  close, detach, and takeover. Creation supplies name, optional host directory,
  base colors, and measured viewport; attach the exact returned ID once.
  Put Session selection and New Session in the title's expanded top panel;
  put Rename/Delete in every row's own overflow menu. Bind actions to that row's
  exact ID and never switch attachments merely to manage another Session.
  Do not add a standalone Session route or terminal-level overflow menu.
- [x] Match the compact v5 terminal layout: minimal app bar and grid insets,
  no history footer/counters, and a persistent bottom row with eight terminal
  keys and a rightmost keyboard show/hide button. Place it
  above system insets with IME hidden and above the IME when visible. Check viewport
  size with IME hidden/shown and selection active.
- [x] Make the whole title tappable with a chevron. Overlay the bounded Session
  list without moving/resizing the terminal. Mark current/occupied Sessions;
  current-row/title/outside tap and Back dismiss without attachment, viewport,
  scroll, selection, or input side effects. Test Back priority and pointer
  consumption. The rightmost bottom button shows/hides IME; content taps never
  open it. System Back hides it. Remove the app-bar keyboard button.
- [x] Keep mutations single-owned across repeated taps and Activity recreation.
  Display host validation/occupancy errors and ambiguous outcomes. A failed
  attachment after successful creation retries attach, not create; stale close
  confirmation remains bound to its exact Session ID.
- [x] Implement switching with one active attachment. Retire the previous view's
  input/gesture/reconnect owner and detach without ending its host process.
  Expanding/dismissing the Session panel retains the attachment, as do OS
  background transitions; only an actual switch/home detach retires it.
- [ ] Verify create/rename/close from Android against desktop-observed Session
  IDs/process state. Check invalid/duplicate names, invalid directory, lost
  mutation response, double submission, cancellation, and concurrent host edits.
- [x] Build the native terminal View, complete-frame slot, native display
  scheduling, semantic colors/cursor, Unicode/wide-cell handling, and resize
  calculation after system/keyboard insets.
- [x] Implement InputConnection composition/commit/deletion/Enter behavior and
  the fixed eight-key bar, with visible one-shot Ctrl/Alt state. Route key
  encoding through synchronized terminal modes in shared Rust logic.
- [x] Preserve semantic cursor coordinates for native IME anchoring while its
  glyph is hidden; verify hidden cursor-only movement on main/alternate screens
  and retain the upstream desktop regression from `8416f4d` during extraction.
- [x] Verify snapshot application/ACK and delta continuity across FFI. Retain
  bounded state while the display is stopped; no per-cell calls, unbounded
  frame/event backlog, or UI-thread network operations.
- [x] Keep the native connection owner outside Activity/Composable visibility
  lifetime. Background entry must not disconnect, detach, release the lease,
  or clear the current Session.
- [x] Reconcile on return: reuse a healthy attachment, recover actual transport
  loss, and fence input until synchronized. Do not send dummy terminal input
  as a liveness check or unconditionally replace a healthy connection.
- [ ] Check process recreation, actual network loss, host revocation, Session
  termination, and competing desktop takeover. Resume the exact Session when
  it still exists; never create another Session or steal its lease implicitly.
  Include Session-ended recovery after host restart/update without importing
  the new desktop updater owner into the shared client or Android graph.

Gate: CORE, SESSION, INPUT, and LIFE behavior passes in the emulator with a real host.
Record representative CJK/combining/color/cursor/alternate-screen fixtures and
keyboard-visible layout. Check sustained output remains bounded and responsive;
do not claim universal performance numbers from one emulator run.

## Phase 5 — Native History, Selection, and Copy

Depends on Phase 4.

- [x] Extend the shared cache/reducer to a policy-configured bounded multi-page
  store with one query owner; retain desktop's current single-window policy and
  tests. Keep v2 query/window limits and validation unchanged. Add stable history
  row identities, separately fenced mutable-live rows, and selection pin leases.
- [x] Wire the store into the Rust Android view owner with Live/History/
  ResumePending, separate desired/presented tokens/metrics, one in-flight query,
  and bounded latest target/frame. Account retained row/string/page/pin memory
  under the initial 4,096-row/16-MiB budgets; evict only unpinned coverage.
- [x] Warm recent history after foreground synchronization without delaying
  input, then direction/velocity/latency-based prefetch before the cached edge.
  Prioritize actual misses and selection extension; deduplicate covered ranges
  and stop speculation when hidden or budget-limited. Preserve proven historical
  pages across ordinary live-cell revisions instead of refetching all of them.
- [x] Add native drag/fling scrolling and a transient scrollbar; reaching the
  bottom returns live. No History/Select text/Disconnect menu. Use fractional
  pixel offsets, validated overscan
  rows, native vsync, and matching hit testing. Stop animation when hidden;
  cache miss/loading/Gap must retain the last complete pixels.
- [x] Keep bottom-following and scrolled positions within one terminal route
  and viewport. Crossing current-screen/retained-output boundaries preserves
  shortcut visibility, selection, and chrome. Validate pinned reading under
  incoming output, then automatic following after scrolling to the bottom.
- [x] Implement one native gesture reducer for chrome, long-press/selection
  handles, and local history. The live child-mode branch sends completed taps
  and bounded wheel/cursor steps; selection/history remain local. Long press
  emits no earlier child click. Fence source, mode, geometry and input epoch.
  Updated after the user reported Herdr touch input on 2026-09-07.
- [x] Add a renderer-neutral content-row range/accessor sharing extraction with
  the existing desktop visible-range API. Verify continuity, wide cells, and
  wrap/newline behavior across pages without flattening an unbounded row set.
- [x] Add long-press entry, handles, edge auto-scroll, and Copy/Cancel.
  Integrate the native floating Copy action and Android selection conventions;
  no line-count toolbar or reserved content rows. Validate custom-grid hit
  testing instead of assuming SelectionContainer selects Canvas content.
  Keep endpoint content identities independent of viewport movement. Pin every
  row in the range, preserve captured live text under incoming output, and pause
  extension at missing pages or explicit resource limits without dropping the
  valid range. A canceled gesture must not resume on a late network response.
- [x] Write once to the native system clipboard per Copy using the shared
  atomic text cap. Test stale/canceled FFI results before any clipboard effect.
- [x] Clear selection and pending copy on invalid source/attachment, resize,
  screen change, navigation, reconnect, and takeover. Reject late FFI draw/copy
  results by generation; never apply handles to an unpainted or replaced frame.
  Scrolling preserves the selection. A host history epoch change forbids joining
  new pages but preserves captured text for Copy/Cancel until the user exits.
- [x] Reuse the shared bounded resume-input owner for healthy same-attachment
  history/selection return: retain the first and subsequent admitted complete
  input units and send once in order after acknowledgement/Active. Test typing,
  modifiers, IME commits, complete paste, overflow, cancellation, failed sync,
  reconnect, takeover, and navigation; offline/new-epoch input must not replay.
  Preserve desktop behavior and avoid a second mobile input queue.
- [x] Add focused reducer tests for multi-page hits/eviction, initial warmup,
  deduplicated/adaptive prefetch, stable history vs mutable live rows, delayed/
  coalesced replies, append/trim/Gap/rebase/resize/reconnect, source tokens,
  cross-page selection pins, resource accounting, one owner, and cancellation.
  Reuse core Unicode/wrapped/space/wide-cell/cap tests; add tests only for new
  Android boundaries rather than duplicating the shared extraction suite.
- [ ] Verify on the emulator with a real host: multi-window history under
  sustained output, slow reply/fling/return-live, repeated cached backtracking,
  child versus local touch ownership under mouse/alternate modes, CJK/wrapped selection
  across at least three
  screen heights and two network-window boundaries, reverse handle drag, exact
  paste into another Android field, and no remote input during Copy/Cancel.
- [ ] Record content-free history request/cache-hit/miss counters, frame timing,
  and accounted peak cache/pin memory. Show cached gestures need no awaited read
  and issue no duplicate range fetches; delay/miss cases remain interactive and
  retain selection. Include full host scrollback under ongoing output to test
  epoch churn rather than assuming retained-row identity lasts forever.

Gate: SCROLL and SELECT acceptance passes with actual native gestures and
clipboard results. Captured selection rows, drawn viewport, and live ACK remain
distinct and bounded. Recheck background retention with history/selection open;
do not let stopping gesture/display work dispose the Session connection.

## Phase 6 — Integrated Checks, APK, and Phone Acceptance

Depends on Phase 5.

- [x] Add one Android build/lint/unit-test CI job using the pinned environment;
  label cross-built artifacts correctly. Local arm64 emulator runtime evidence
  remains distinct from hosted build results.
- [x] Run the authoritative Rust check once on the final extraction/integration
  scope, then Android lint/unit/instrumentation checks. Repeat only after a
  relevant fix or new failure.
- [x] Exercise Android-to-host direct and relay routes, phone/desktop handoff,
  and background-return behavior, recording actual host/path/commit evidence.
  Do not equate the old desktop matrix with Android transport acceptance.
- [x] Establish a persistent external APK signing key, produce the signed APK
  with an explicit Android versionCode, verify its signature and native
  packaging, and supply SHA-256/build/installation details. Keep signing
  credentials and machine-specific configuration outside the checkout.
- [x] Separate release `io.github.leonfox28.zterm` and debug `.dev` packages;
  distinguish launcher labels and verify simultaneous installation with independent
  identities/host stores. Verify retained-Session handoff applies the new controller
  geometry; 1006 evidence is recorded in implementation-progress.md.
- [x] Install a later same-signature build over the earlier test build and
  verify identity/known hosts remain. Separately verify uninstall/reinstall
  starts a new pairing identity without restoring it from backup.
- [ ] Deliver the APK and complete Xiaomi 17 Pro Max acceptance: record its
  actual OS/page size, physical camera recognition, album/manual input,
  Session management, system IME, touch scrolling, selection/system copy,
  direct/relay behavior, and background/foreground recovery.
  User-operated phone checks are acceptable evidence when the phone is not
  attached locally; retain explicit pending rows until results are supplied.
- [x] Update actual project Android/client guidelines and affected existing
  contracts, plus `docs/android.md`. Remove any stale claim that no App exists.
  Preserve unrelated platform publication exclusions.
- [ ] Perform the inline Trellis quality/finish steps. Do not archive the task
  or claim full Android completion while required phone/host evidence is absent.

## Validation Entry Points

The following build/test entry points now exist. See the implementation evidence
for executed commands; connected tests require the documented explicit fixtures.

```sh
# Shared extraction and existing product quality owner
just check

# Android wrapper: generate bindings, build native library, assemble APK
just android-build

# Android Gradle project; Rust generation/build is wired into its graph
apps/android/gradlew -p apps/android :app:lintDebug :app:testDebugUnitTest
apps/android/gradlew -p apps/android :app:connectedDebugAndroidTest

# Existing local AVD; run only when the Android build is ready
/Users/huyuanzhe/Library/Android/sdk/emulator/emulator -avd Pixel_9
adb -e install -r apps/android/app/build/outputs/apk/debug/app-debug.apk

# Delivery helper wraps signing, signature verification, SHA-256,
# version/commit information, and 16 KB ELF/zip alignment checks
just android-apk <version-code>
```

Use the selected device explicitly if more than one emulator/device is present.
Keep one evidence file under this task's `research/` with case, host/device,
transport, source commit, artifact, action, and observed result. Product secrets
are not evidence fields.

## Check Ownership and Risk Map

| Contract / risk | Validation owner |
| --- | --- |
| Existing host baseline | User-confirmed complete on 2026-09-07; evidence boundaries in `research/mainline-refresh.md`, Phase 0 |
| No host engine/PTY in mobile graph | Existing dependency-policy script, extended in Phase 2 |
| Pairing, revisions, replay, takeover, adapter equivalence | Existing Rust suites plus focused shared-client tests |
| Session mutations, duplicate taps, unknown outcomes, switch vs close | Shared unary/replay tests plus Android/desktop integration, Phase 4 |
| Native ABI, JNA load, async cancellation and lifetime | Narrow emulator bridge tests, Phase 1 |
| QR entry positions and three source paths | Android screen/instrumentation tests plus physical camera check |
| IME, shortcut bytes, viewport and frame identity | Shared input tests plus Android emulator integration |
| Multi-page cache, prefetch, memory, gestures/drawn frame vs desired target | Core cache tests plus Android request/frame/memory evidence, Phase 5 |
| Cross-screen range identity/pins, exact clipboard text and cancellation | Shared extraction suite plus native edge-scroll/clipboard integration, Phase 5 |
| Background retention vs actual interruption | Android lifecycle/transport scenarios; record actual disconnect cause |
| APK identity/update/native packaging | APK helper and signed install-over-existing check |
| Xiaomi-specific behavior | Recorded Xiaomi 17 Pro Max acceptance, Phase 6 |

Highest-risk edits are `crates/daemon/src/client/`, pairing/broker extraction,
the Rust/Kotlin frame/ACK/selection boundary, shared multi-page cache/range
extensions, gesture ownership, and generated
native packaging. Keep those
changes small enough to inspect independently. An incompatible native bridge
blocks dependent steps; it does not justify weakening
the wire contract, removing approved scan entries, or adding a host engine to
the App.

## Acceptance status for APK delivery

Implementation checkboxes above distinguish completed owners from remaining
compound acceptance matrices. Runtime evidence is in
`research/implementation-progress.md`. Signed APK delivery does not close the
physical Xiaomi acceptance row. Remaining unchecked compound rows include
fault injection / uncommon device permutations not established by build success;
do not retroactively mark them passed from adjacent successful tests.


## Public release extension — completed 2026-09-07

- [x] Publish v0.1.26 through the canonical prepare/PR/merge/main/tag/sign flow.
- [x] Add the retained ARM64 Android APK to the exact-main candidate and protected
  signed inventory while preserving the existing native targets and installer.
- [x] Verify the public APK's signer, versionCode, source, 16 KB packaging, and
  emulator upgrade with identity/install history retained.

See `research/android-public-release.md` for release/CI/artifact provenance. This
completes public distribution, not the separately pending physical acceptance rows.
