# Android App

## Goal

Deliver the Android controller App for Zterm so users can continue terminal work
on an existing remote host from their phone.

The agreed first release covers pairing, Session management, terminal input,
touch selection/copy, cached scrolling, recovery, language/theme/font settings,
landscape, and saved-device removal, delivered as an APK. The requirements below
include the accepted completeness review. The user approved implementation and
confirmed the existing macOS/Linux host baseline on 2026-09-07. The task is now
`in_progress`; execution evidence is in `research/implementation-progress.md`.

## Background

- The existing roadmap identifies Android as the second stage and a remote
  controller only, without an Android-local general-purpose shell
  (`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:28`).
- The repository currently provides Rust core/protocol/platform/terminal/daemon/CLI
  crates and the semantic protobuf v2 contract; no Android App project was found
  in the repository inspection (`Cargo.toml`, `README.md`, `proto/zterm/v2/`).
- Existing host authorization is directional. Android is expected to use the
  same host-owned persistent Sessions and controller-lease rules as desktop
  clients, including handoff to the host's local CLI
  (`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:137`, `:242`).
- The Android boundary is a consumer of semantic surfaces, deltas, history,
  and renderer-neutral cache/selection values. It must not parse desktop ANSI
  or inherit the host Alacritty model, local PTY, or desktop presenter
  (`.trellis/spec/guides/cross-platform-thinking-guide.md:108`).

## Requirements

### Product constraints

- Android controls existing remote Zterm hosts; remote terminal work belongs to
  the host and survives loss of the phone connection.
- Preserve the existing device identity, pairing, authorization, Session,
  and explicit takeover semantics.
- Keep terminal behavior generic rather than special-casing individual programs.

### Core journey

- **CORE-1:** pair with a host and retain it across App restarts. Tapping a
  saved host connects directly into its terminal view, without an intermediate
  Session picker. Restore the exact previous Session when one is recorded.
  Session selection expands from the terminal title (UI-4). Without a recorded
  previous Session: a sole Session is the target; multiple Sessions open the
  terminal container with its title list automatically expanded; no Sessions
  shows an explicit New Session entry. Connecting/recovering never creates a
  Session or implicitly takes over an occupied one. If a recorded ID has ended,
  show that outcome and let the user choose/create; never silently substitute
  another ID. The same rules apply after pairing.
- **CORE-2:** display and interact with the live terminal, including mobile
  keyboard input and viewport resizing.
- **CORE-3:** show clear connection/synchronization, occupied, revoked, and
  Session-ended states. Permit explicit takeover and explicit detach; detach
  returns to home without closing the host Session.
- **CORE-4 — Remove a saved device:** long-press a home device card to confirm
  local removal. Ordinary tap still connects. Remove its phone-local known-host,
  route/cache, and matching recent-Session/resume references atomically, without
  closing any host Session or revoking host-side authorization. Cancel changes
  nothing; a storage failure retains the card and permits retry. A removed
  device must not reappear through late events or restart; explicit pairing
  can add it again. Do not reset the App identity or other saved devices.

### Session management

- **SESSION-1 — List and create:** show the host's Sessions and their current
  occupancy. Provide explicit creation with a name and optional host working
  directory; an omitted directory uses the host default. Open the returned
  exact Session after successful creation. Invalid/duplicate names, invalid
  directories, and host limits yield actionable errors without duplicate work.
- **SESSION-2 — Rename:** rename a Session from its row's overflow menu in the
  expanded title panel. Its identity, running process, and attachment remain
  unchanged; use the host's authoritative result to refresh the displayed name.
- **SESSION-3 — Close:** expose a row-menu Delete action with confirmation that
  it terminates that host Session and its running work. Cancel sends no close
  request. Successful closure updates the list and exits that Session's view;
  returning from the terminal or detaching never invokes close.
- **SESSION-4 — Switch:** switch hosts directly from home, or Sessions through
  the terminal title's expanded list, with one active
  terminal attachment in the App. An intentional switch detaches the previous
  view while leaving its host Session running. Entering the App background is
  not an intentional switch and continues to follow LIFE-1/2.
- **SESSION-5 — Mutation ownership:** reuse the existing operation lease and
  replay rules for create/rename/close. Repeated taps and ambiguous network
  replies must not issue a second logical creation or retarget by name. Show
  unresolved outcomes and refresh authoritative state; never report success
  from an optimistic list update alone.

### Pairing experience

- **PAIR-1 — Camera scanning:** the first release opens a custom QR-scanning
  screen for pairing, with live camera preview as the primary entry.
- **PAIR-2 — Album entry:** the lower-left corner of that screen opens the
  system image selector. Selecting an image decodes its pairing QR code and
  continues the same pairing flow as camera scanning.
  Use Android's single-image picker (or system document-picker fallback),
  without an App-owned gallery, media grid, or album-navigation screen.
- **PAIR-3 — Ticket entry:** the lower-right corner opens a modal dialog over
  the scanner for entering or pasting the pairing ticket and submitting it.
  It is not a separate page or navigation route. Show only the text field,
  a wide centered Connect button aligned with the field, and a top-right close
  button. Omit the title and dedicated Paste action; pasting remains available
  through the standard text field. Empty input disables
  Connect; invalid input shows a short inline error and retains the value.
  Dismiss returns to the same scanner. Fit the dialog above the IME when editing.
- **PAIR-4 — One pairing contract:** camera results, image results, and manual
  tickets use the same existing ticket validation and host authorization flow.
  Host-side QR presentation must encode the existing ticket text, preserving
  the roadmap's single ticket format and avoiding a second pairing protocol
  (`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:134`).
- **PAIR-5 — Recoverable entry errors:** canceling image selection returns to
  the scan screen. An unreadable image, unrelated QR code, invalid/expired
  ticket, or failed pairing shows a useful error and allows another attempt.
  Denying camera permission leaves the album and manual-ticket entries usable.
- **PAIR-6 — Single attempt ownership:** repeated camera detections must not
  launch concurrent pairing attempts. Leaving the scan screen or opening the
  album/manual entry pauses camera recognition.

### Terminal input

- **INPUT-1 — System input method:** use the Android system keyboard for text
  input, with the terminal viewport responding to keyboard visibility changes.
  The rightmost button in the bottom shortcut row explicitly shows/hides the
  IME; Android Back also hides it. Content taps never open the keyboard. Do not
  reserve a keyboard toggle in the app bar. Child clicks, local scrolling and
  selection keep their declared gesture ownership.
- **INPUT-2 — Fixed shortcut row:** keep one fixed row at the terminal bottom with
  `Esc`, `Tab`, `Ctrl`, `Alt`, and the four directional arrow keys. The user
  confirmed these eight terminal keys plus a rightmost keyboard visibility
  button (2026-09-07). The keyboard button sends no terminal input.
  It remains visible with the keyboard hidden, while scrolling, and during
  selection. Anchor it above the system bottom inset; when the IME appears,
  move the same row directly above it. Reserve its height in the viewport so
  it does not cover terminal text. Disable input when no Session is ready.
- `Ctrl` and `Alt` act as terminal modifiers; they must work with the next
  relevant key rather than insert their button labels into the terminal.
- Additional shortcut buttons, configurable layouts, and macro controls are
  deferred. Existing synchronization-before-input rules apply equally to text,
  shortcut keys, and modifier combinations.

### Scrolling and selection/copy

- **SCROLL-1 — History:** vertical dragging and inertial scrolling naturally
  browse retained main-screen output in the same terminal surface. Do not show
  dedicated History/Return to live labels, distance counters, explanatory copy,
  or a separate history/status footer. The INPUT-2 shortcut row stays visible.
  Scrolling to the bottom resumes live following; a small
  transient scrollbar is sufficient position feedback. Incoming output does
  not pull a pinned view to the bottom. Loading retains the last complete frame.
- **SCROLL-2 — Gesture ownership:** ordinary retained output and active text
  selection own local touch scrolling. At the synchronized live grid, programs
  declaring mouse reporting receive a completed tap as press/release and vertical
  drags as wheel steps at the gesture's starting cell. Alternate-screen programs
  declaring alternate-scroll receive cursor-key steps when mouse reporting is
  off. Long press starts native selection without a preceding child click;
  selection handles and browsing pinned history never also send remote input.
  Lock ownership for the gesture, fence stale source/mode/geometry/attachment,
  and route by terminal modes, never application names. IME access is through
  the separate bottom button, never a side effect of child clicks.
- **SCROLL-3 — Return and recovery:** a healthy visual scroll to the bottom uses the
  already maintained live surface locally, without a new network sync. An input-
  triggered return from browsing/selection keeps the existing sync fence. During a healthy same-attachment return from browsing/selection,
  retain the triggering complete key/text/paste and subsequent admitted input
  within the shared fixed resume bound. Send once, in order, only after sync
  acknowledgement and Active; do not lose the first character. True reconnect,
  failed/canceled return, Session switch, takeover, or invalid attachment clears
  retained input. Input while disconnected is never queued for later replay.
  Resource rejection is explicit and atomic; never send a partial paste.
  Resize, reconnect, screen changes, and history gaps reconcile the viewport
  against authoritative data rather than applying stale row coordinates.
- **SCROLL-4 — Local interaction and prefetch:** retain multiple pages of
  semantic history in bounded App memory and proactively warm recent history.
  Dragging, animation, revisiting cached pages, and selection adjustment work
  locally; they do not require a request/response for each swipe or frame.
  Prefetch toward the movement direction before reaching the cached edge.
  Only missing/invalidated ranges need network reads. Reaching uncached content
  on a slow connection shows loading without clearing the existing view; full
  offline history or an unlimited local archive is not promised.
- **SELECT-1 — Native selection:** long-press terminal text to enter a local
  linear selection, adjust its endpoints with touch handles, and use an explicit
  Android floating Copy action to write to the system clipboard. Prefer native
  Android selection conventions and system action UI; no selected-line counts,
  selection instructions, persistent toolbar, or content-layout shift.
  App-owned long press remains reachable in mouse-reporting programs without a
  menu action. Both live and historical text support selection. Dragging a
  handle to the screen edge automatically scrolls and extends the same selection
  across screens and cached page boundaries. The offscreen endpoint stays
  anchored to its content, rather than being clamped to the current viewport.

- **SELECT-2 — Exact text:** copy semantic terminal text rather than a screen
  image, preserving CJK/combining characters, wide-cell boundaries, spaces, and
  hard versus soft line breaks. Forward and reverse selections yield the same
  text. Reuse the shared 512 KiB clipboard limit and reject oversized copies
  atomically without silently truncating them.
- **SELECT-3 — Local ownership:** selecting or copying does not send Ctrl+C,
  mouse input, or selected text to the host. Range coordinates belong to the
  current attachment and captured content only. Scrolling within selection
  mode preserves the range and pins its source rows until Copy/Cancel. New
  output cannot silently replace selected text. Clear selection on Session
  navigation, resize/reflow, screen replacement, reconnect, or takeover. If
  older content is missing, trimmed, or cannot be joined with proven identity,
  pause extension with a useful explanation and preserve already captured
  text; never skip a gap or copy a guessed range. Copy failure leaves the
  remote Session running and allows a retry.

### UI layout and copy

- **UI-1 — Home:** use a settings icon at top right instead of an Android badge.
  Remove the large home heading and slogan. If there is a previous connection,
  show its quick-resume card as the first content below the app bar; otherwise
  omit the card and let the saved-host list move up. Retain saved-host cards and
  the Add host action.
- **UI-2 — Content first:** terminal content is the terminal's character grid,
  with minimal padding and only compact navigation chrome. Demonstration text
  density is not a requirement to rewrite or space out program output. Keep
  the largest practical viewport with keyboard hidden, shown, and during
  selection; floating controls must not reserve permanent rows.
- **UI-3 — Concise text:** all screens use short titles, labels, and actions.
  Remove slogans, obvious usage instructions, and repeated status descriptions.
  Scanner chrome needs only its short title, camera area, lower-left Album,
  and lower-right Ticket. Keep actionable errors and concise destructive-action
  consequences. Settings provides the language, theme, and font controls in
  UI-7/8/9.
- **UI-4 — Sessions in the terminal title:** remove the standalone Session page.
  Tap the whole terminal title, marked with a small chevron, to expand a list
  from the app bar. It overlays the terminal instead of pushing/resizing the
  grid. Mark the current Session and occupied Sessions; selecting another
  switches and collapses, while selecting the current one only collapses.
  Tapping the title again, outside the panel, or Android Back dismisses it.
  Opening/dismissing the panel does not detach, send terminal input, reset the
  scroll/selection, or issue a PTY resize. Back dismisses the panel before
  affecting the IME or terminal navigation. Keep New Session in the panel and
  rename/confirmed close in Session actions. Bound a long list's height and
  scroll within it; opening the panel must not scroll the terminal underneath.
- **UI-5 — Minimal management:** remove the terminal's top-right overflow menu.
  Do not add Select text, History, or Disconnect actions elsewhere. Direct
  scrolling/long press/Copy provide browsing and selection. Every Session row,
  current or not and including occupied Sessions, has its own visible overflow
  button with exactly Rename and Delete. Row tap switches; overflow tap opens
  management for that exact ID without changing the current attachment. Rename
  or delete of another Session must not switch into it. Delete uses SESSION-3
  confirmation and terminates the host Session/processes; it is not merely
  removing a saved label. Back to home detaches the view without ending work;
  App background continues to follow LIFE-1/2.
- **UI-6 — One scrollable terminal:** present retained output and the current
  terminal screen in one continuous scrollable viewport, with one selection
  model and the same fixed shortcut row. There is no separate History route,
  mode switch, or chrome change when the reading position leaves the bottom.
  At the bottom, follow new output; away from it, preserve the reading anchor.
  Reaching the bottom resumes following. Internal live-screen/scrollback data
  and synchronization states are implementation details, not separate pages.
- **UI-7 — Language:** settings offers exactly Follow system, Chinese, and
  English as a single choice; default to Follow system. Apply the choice
  immediately and persist it across App restarts. Follow system resolves to
  supported device languages, falling back to English for unsupported ones.
  Translate App chrome, controls, errors, and accessibility labels, while
  preserving host/Session names, credentials, paths, and terminal output.
- **UI-8 — Theme:** settings offers exactly Follow system, Dark, and Light
  as a single choice; default to Follow system. Apply immediately and persist
  independently of language. Following system responds to device theme changes;
  explicit choices remain fixed. Cover App screens, dialogs, system-bar icon
  contrast, and the terminal's default foreground/background. Preserve explicit
  program-supplied terminal colors. Language/theme changes do not reset pairing,
  detach a Session, clear cached output, or issue terminal input. Preserve
  selection unless a real geometry change requires SELECT-3 reconciliation.
- **UI-9 — Terminal readability:** settings includes terminal font size with
  immediate preview and persistence independent of language/theme. Initial
  control is a horizontal slider from 8 through 16 in integer steps of 1,
  default 12. Preview while dragging and persist the chosen size; verify on the emulator and
  phone before delivery. Support portrait and landscape under system rotation
  preferences. Recompute the grid from available bounds/insets and shortcut row;
  keep the same Session and connection. IME animation only pans/clips locally;
  submit one final PTY size after it settles, including hide/cancel. Coalesce PTY resize, clear invalid
  selection coordinates under SELECT-3, and reconcile the reading position.
  Do not shrink/clip the grid to fake extra columns or add a zoom gesture.
- **UI-10 — Complete state feedback:** illustrate and implement loading,
  reconnecting/failure, unreachable/revoked host, empty/ended Session, camera
  denial, invalid/expired credential, missing-image-QR, and picker cancellation
  as states of existing routes/dialogs. Keep text concise, preserve entered
  credentials and the last complete terminal frame where applicable, provide
  the relevant retry/return action, and prevent duplicate submissions. A fresh
  unconnected host has no fabricated terminal output to preserve.

### Background and recovery behavior

- **LIFE-1 — Preserve the live connection:** moving the App to the background
  does not actively close its connection, detach its terminal attachment,
  release its controller lease, or reset its Session state. The user requested
  that the first release leave the existing connection alone on backgrounding.
- **LIFE-2 — Reuse on return:** when the existing connection and synchronized
  attachment are still healthy, returning to the foreground reuses them. Do
  not force disconnect/reconnect or create a replacement attachment merely
  because the App became visible again.
- **LIFE-3 — Recover actual interruption:** system suspension, process loss,
  network changes, or transport failure can still interrupt connectivity.
  Recover the same host and exact Session ID, synchronize its screen, then
  enable input. Never silently create a new Session under the previous name
  or automatically take control away from another controller.
- **LIFE-4 — First-release limit:** preserve connectivity on a best-effort
  basis using the normal connection owner. Do not add a persistent foreground
  service, wake lock, battery-exemption flow, or special background keepalive
  mechanism. Continuous background connectivity is not a product guarantee.
- Retaining a background attachment also retains its existing controller
  authority until real disconnect, explicit detach, revocation, or takeover;
  desktop handoff continues to use the existing explicit takeover contract.

### Development and acceptance environments

- Use a local Android emulator during development, as requested by the user.
  The existing `Pixel_9` AVD uses an API 36 arm64-v8a system image and is the
  initial development target.
- Use the user's Xiaomi 17 Pro Max for final real-device acceptance.
- Record the phone's actual Android/HyperOS version when it becomes available
  for testing; do not infer an installed OS version from its model name.
- Emulator checks cover the development loop; device-specific keyboard,
  background/foreground, and network behavior require separate phone evidence.

### Delivery

- **DELIVERY-1:** deliver an installable APK for emulator development and Xiaomi
  phone acceptance. The user explicitly deferred app-store publication.
- Keep application identity and signing stable across test APK updates so
  installing a later build does not require discarding existing pairing state.
- **DELIVERY-2 (2026-09-07):** release uses `io.github.leonfox28.zterm`; debug
  development uses `io.github.leonfox28.zterm.dev` with launcher label `zterm Dev`.
  Both install concurrently with independent pairing identities, hosts and
  preferences. Preserve the existing release ID and signing key.

- **DELIVERY-3 (2026-09-07):** publish the signed Android ARM64 APK in the same
  immutable GitHub Release as macOS/Linux, from the exact green main candidate.
  Retain package identity/certificate and include verified build metadata and
  authenticated checksums. App-store publication remains deferred.

## Acceptance Criteria

- [ ] **CORE-1/UI-4:** without a previous ID, test zero/one/multiple Sessions:
  explicit New Session / direct target / auto-expanded title list respectively.
  Selection attaches the exact chosen ID; occupied needs confirmation. A stale
  previous ID shows ended instead of choosing or creating a replacement.
- [ ] **CORE-4:** ordinary card tap connects and long-press opens local-removal
  confirmation. Cancel and failed storage preserve records; successful removal
  clears its card/resume references across restart, without host close/revoke,
  App identity changes, or resurrection from late events. Re-pair can add it.
- [ ] **UI-9:** font preview responds immediately and the chosen size persists.
  Portrait/landscape with IME shown/hidden keeps controls reachable, increases
  columns when width permits, and retains the same host Session/connection.
  Font/rotation resize reconciles viewport and invalidates stale selections.
- [ ] **UI-10/CORE-3/PAIR-5/6:** required empty/error/loading states remain in
  existing routes/dialogs; short feedback offers a usable recovery path and
  repeated taps/results never duplicate pairing or creation.
- [ ] **CORE-1/UI-1:** tapping a known device with a recorded Session opens its
  terminal directly; the resume card reaches the same exact Session. A home
  state without connection history has no resume card. Settings opens from the
  top-right icon and returns home; the badge, heading, and slogan are absent.
- [ ] **UI-7/8:** fresh install selects Follow system for both preferences.
  All nine language/theme combinations retain independent selections and
  apply without a Save step. Choices survive restart and navigation. Chinese
  and English App strings are complete; system changes affect only preferences
  following the system. Dark/light surfaces and control contrast remain legible,
  including dialogs, terminal defaults, selection, shortcut keys, and system bars.
- [ ] **PAIR-3/6:** scanner credential entry opens a dialog over the scanner;
  no standalone credential page exists. It contains only the text field,
  wide centered Connect, and top-right close; no title or Paste button.
  Standard field paste remains usable. Dismiss/Back resumes scanning after
  IME handling without pairing. Empty input cannot submit; typed/pasted valid
  input uses the existing pairing contract, and failures retain editable input.
- [ ] **UI-2/3/SCROLL-1/SELECT-1:** terminal scrolling/selection preserves a
  compact viewport without History/Return to live labels, distance/line-count
  counters, or instructional paragraphs. Selection uses handles and floating
  Copy; scanner has no promotional/instructional paragraphs.
- [ ] **UI-4/INPUT-1:** title tap expands the Session list over an unchanged
  terminal grid; current/title/outside tap and Back dismiss without detach,
  resize, scroll reset, selection reset, or input. Switching closes the panel
  and opens the chosen exact ID. No standalone Session page or top keyboard
  button exists; the rightmost bottom button toggles IME and system Back hides
  it. Content taps leave keyboard visibility and terminal geometry unchanged.
- [ ] **UI-5/SESSION-2/3/4:** no top-right terminal overflow or separate
  Select text/History/Disconnect action exists. Every Session row has a
  separate menu hit target showing only Rename/Delete for its exact ID.
  Opening/canceling it leaves the current Session and scroll position intact.
  Managing a different Session does not attach to it; deletion requires
  confirmation and updates only the identified Session.
- [ ] **DELIVERY-1/CORE-1/2:** the App builds, installs, and runs on the local
  `Pixel_9` emulator, where pairing, Session connection, input, and recovery can
  be exercised repeatedly.
- [ ] **CORE-1/2:** a user can install the App, pair with an existing host, open
  a Session, see its terminal contents, and send keyboard input.
- [ ] **CORE-1/3:** known hosts survive App restart; empty, occupied, revoked,
  and ended states give an actionable result. Detach leaves the host Session
  running, and takeover only occurs after the user's explicit action.
- [ ] **SESSION-1/2:** create from an empty or populated list, attach the exact
  returned Session, then rename it while its process continues. Verify its ID
  and running work do not change and desktop clients observe the same result.
- [ ] **SESSION-1/5:** invalid/duplicate names, invalid host directories, double
  submission, and a lost mutation response yield no duplicate Session or silent
  retry under a new operation ID; ambiguous outcomes remain visibly unresolved.
- [ ] **SESSION-3/4:** canceling close leaves the Session intact; confirmed close
  ends that exact Session. Switching/detaching keeps previous host work running,
  releases the intentional old attachment, and never affects another Session.
- [ ] **PAIR-1/4:** a fresh host-generated QR code can be scanned by the App
  and completes the existing directional pairing flow.
- [ ] **PAIR-2:** the scan screen's lower-left entry selects a QR image from
  the system image selector and pairs with the same behavior as camera input.
  No custom App gallery route or media-grid implementation exists; canceling
  the system picker returns to the scanner without changing pairing state.
- [ ] **PAIR-3:** the lower-right entry accepts a manually entered or pasted
  ticket and completes pairing without camera access.
- [ ] **PAIR-5/6:** picker cancellation, camera denial, invalid/unreadable QR
  input, ticket expiry, and repeated camera frames leave the three-entry flow
  recoverable and do not create duplicate concurrent pairing operations.
- [ ] **INPUT-1/2:** system-keyboard text and the eight shortcut controls reach
  the remote terminal correctly; exercise shell completion with Tab,
  interruption with Ctrl+C, cursor movement, Escape, and an Alt combination in
  a suitable terminal fixture.
- [ ] **INPUT-1/2:** showing/hiding the keyboard adjusts the terminal viewport;
  initial-connection and reconnect input are not replayed after recovery.
  Healthy same-attachment return from scrolling follows SCROLL-3: the first
  character, modifiers, ordered typing, and a complete paste arrive once after
  synchronization. Cancel/failure/new attachment drops the queued input.
- [ ] **INPUT-2/UI-6:** the eight keys and keyboard button remain visible at the terminal bottom
  while dragging/flinging and selecting with IME hidden, and directly above
  the IME when shown. It never covers output. Crossing between retained output
  and the current screen keeps the same route, controls, and selection model.
- [ ] **SCROLL-1/3:** drag and fling through output spanning multiple viewports
  and cache windows, including a delayed remote response. New output preserves
  the reading position, no blank/partial history frame flashes, and return to
  the bottom restores synchronized input with healthy-return retention under
  SCROLL-3; actual reconnect must never replay disconnected input.
- [ ] **SCROLL-4:** after history warmup, repeated forward/back gestures within
  fully cached coverage render without waiting for history responses and cause
  no duplicate range fetches. With delayed history responses, cached scrolling
  remains interactive; prefetch starts before the loaded edge and a miss retains
  complete content. Record cache-hit/miss, request, memory, and frame evidence.
- [ ] **SCROLL-2:** exercise main history, child mouse reporting, alternate-scroll,
  and an alternate screen with neither mode. Live child-mode gestures send
  exactly the declared mouse/cursor input; local history and long-press selection
  emit none and require no menu. Canceled/retired gestures cannot replay.
  Browse only available retained content; missing alternate-screen history
  is never synthesized. Keyboard program control remains mode-aware.
- [ ] **SELECT-1/2:** long-press and handle-adjust a range in live text and after
  scrolling to historical text, then paste into another Android text field to
  verify exact Unicode, spaces, and wrapped-line behavior. A reversed drag gives
  the same text, and an oversized range leaves the previous clipboard unchanged.
- [ ] **SELECT-1/3/SCROLL-4:** select across at least three screen heights and
  two network-window boundaries using edge auto-scroll, reverse direction, and
  copy the exact continuous text. The initial offscreen endpoint survives;
  scrolling inside loaded coverage needs no range fetch. A delayed missing page
  pauses extension without discarding the existing selection or skipping text.
- [ ] **SELECT-1/3:** explicit selection works in a mouse-reporting terminal;
  selection/copy/cancel produce no remote input. Stale handles disappear after
  resize, screen replacement, reconnect, takeover, or Session switching. Normal
  selection scrolling and new output preserve captured text. Host trimming or
  cache pressure cannot silently evict selected rows or change copied contents.
- [ ] **LIFE-1/3:** disconnecting or leaving the App does not terminate the host
  Session; returning restores that same Session before accepting input.
- [ ] **LIFE-1/2:** background and foreground transitions on a live connection
  cause no App-initiated disconnect, detach, lease release, or attachment
  replacement; a healthy synchronized connection is reused on return.
- [ ] **LIFE-3/4:** after actual network loss or system suspension/process
  recreation, foreground recovery targets the same Session, fences input until
  synchronized, and handles occupied/ended Sessions without silently taking
  over or creating another Session. Record what the OS actually interrupted;
  do not require an uninterrupted background connection as the pass condition.
- [ ] **CORE-3/LIFE-1/3:** phone-to-desktop handoff preserves the same Session
  and obeys the existing controller ownership rules.
- [ ] Android runtime evidence is recorded separately from Rust compilation
  and desktop test results.
- [ ] The approved flow is verified on the Xiaomi 17 Pro Max with its actual
  OS version recorded; emulator success is not reported as phone acceptance.
- [ ] **DELIVERY-1:** an APK is supplied with installation/update instructions
  and identifiable build/version information; no app-store release is required
  for acceptance.
- [ ] **DELIVERY-1:** an update signed with the same test-distribution identity
  preserves the installed App's known hosts and device identity. Uninstall and
  reinstall start with a new identity and require pairing again.

## Out of Scope

- Hosting a general-purpose local shell on Android.
- Implementing Windows, iOS, or desktop GUI clients in this task.
- Concurrent terminal tabs/split panes; the first release manages multiple host
  Sessions through lists with one active terminal attachment.
- Rectangular selection, full-history export/search, unlimited local history,
  and guaranteed offline access to history that was never cached.
- Advanced or customizable terminal keyboard controls beyond the essential
  input needed for the approved increment.
- Dedicated background keepalive services, wake locks, battery-exemption
  setup, and guarantees of continuous connectivity while backgrounded.
- Google Play or other app-store publication.
- Android-local terminal parsing, a new wire protocol, and new Agent-specific
  integrations. Additional terminal gestures, custom theme/palette editing, and host-driven
  clipboard integration are deferred; explicit local selection/copy is in scope
  and existing desktop support is preserved.

## Readiness and Evidence

- The accepted post-v7 completeness review is recorded in
  `research/ui-completeness-review.md`; CORE-1/4, SCROLL-3, and UI-9/10 own its
  requirements. There are no remaining blocking product questions from this
  review. Revision v8 is recorded in `research/ui-design.md`; the final planning
  summary was approved on 2026-09-07, with implementation/checking inline and
  without sub-agents.
- Related roadmap: `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md`.
- Current code and local build-environment findings are recorded in
  `research/planning-baseline.md`. The latest mainline refresh to `de25a38`
  (`0.1.24`) and its compatibility impact are in `research/mainline-refresh.md`.
- Scanner, album-picker, and shared pairing-flow research is recorded in
  `research/qr-pairing.md`, with official Android/ML Kit sources.
- Session mutation, history-cache, pointer ownership, and semantic selection
  evidence for the expanded scope is in `research/session-selection-scroll.md`.
- The user confirmed completed macOS/Linux basic validation on 2026-09-07.
  Accept this as the existing host prerequisite for Android work and use local
  `zterm` / remote `zterm connect dev` for integration. The evidence record
  distinguishes that report and successful read-only access checks from
  independently observed six-path tests; no unobserved routes are marked passed.
- `design.md` owns technology selection, dependency boundaries, technical
  compatibility floors, and artifact signing. `implement.md` owns ordered
  execution and checks. Framework publication/version checks are research
  evidence, not successful Android-build or phone-runtime evidence.
- Broad device certification, the phone's installed OS version, and actual
  sustained rendering/network measurements remain evidence to collect during
  execution. The acceptance targets above remain the committed device scope.
