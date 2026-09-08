# Cross-client presentation continuity — brainstorm evidence and alternatives

Date: 2026-09-07. Source baseline: `be66a16` on `main`.
Status: planning research; DEC 2026 support was accepted in subsequent discussion as a needed
capability in the proposed scope. Other recommendations and concrete implementation remain planning.
Scope: existing desktop CLI (inside an outer terminal emulator) and Android Canvas client.

Subsequent review: `second-review.md` corrects assumptions about clear-as-root-cause,
resize completion, publication boundaries, pixel remainder handoff, and input continuity.
Use its explicit corrections over provisional recommendations in this first research pass.

Convergence note (2026-09-08): [the PRD](../prd.md) and [design](../design.md) now own the
resolved product choices and implementation direction. This document retains the initial evidence
and alternatives; its historical questions are not additional decisions for the user.

## 1. Outcome and evidence limits

The user wants unchanged screen regions to remain visually stable during ordinary output,
local viewport motion and resize. Real terminal output and real layout changes must still
become visible. Android retains final host resize after local keyboard avoidance.

This analysis inspects source, retained research and primary documentation. It does not
claim a reproduced flicker root cause, benchmark improvement or device acceptance. Current
Android state changes, geometry handoff and child repaint boundaries are separate candidate
contributors. Do not count every Canvas repaint as a visible blank frame.

The existing M10 task is release acceptance and does not own this cross-client design.
The new task is a planning child of the product roadmap. No product code changed.

## 2. Evidence inventory

| Owner / anchor | Current behavior | Implication |
| --- | --- | --- |
| `crates/terminal/src/model.rs:149` | Each nonempty ingest advances model revision. | A revision is not an application frame boundary. |
| `crates/terminal/src/model.rs:205` | Resize updates the authoritative grid/history epoch immediately. | A valid resized grid can precede the child's repaint. |
| `crates/terminal/src/model.rs:265` | Same geometry yields changed-row replacements; different size/screen yields snapshot. | Existing wire already avoids unchanged rows; cross-size delta is not prerequisite to visual continuity. |
| `crates/daemon/src/terminal_driver.rs:480` | PTY resize, model resize and publication share the commit transaction. | Native size and model size must remain consistent; a presentation fix must not delay PTY correctness. |
| `crates/daemon/src/terminal_driver.rs:739` | One attachment checkpoint; capture merges to latest model. | Do not add a revision queue or consume shared engine damage for multiple clients. |
| `crates/daemon/src/session_wire.rs:1365` | Writer consumes revision watch and writes complete framed updates. | Current latest-state coalescing happens before writes; protocol byte/patch dependencies remain ordered. |
| `crates/client/src/surface.rs` | Shared validated semantic surface and candidate delta application. | Correctness belongs here, independently of desktop/Android rendering. |
| `crates/cli/src/terminal_ui/composition.rs:135` | Composed frame includes child content, chrome, cursor, colors and layout. | Local chrome changes must participate in final diff. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:176` | Identical frame skips output; compatible baseline skips equal rows/cell runs. | Ordinary desktop output already implements the user's principle. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:181` | Physical-size/layout mismatch rejects old baseline and emits ED2. | Need to distinguish unknown cell positions from the separate choice to clear before repaint. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:238` | Physical baseline/input modes commit only after write and flush succeed. | Never use an unsubmitted desired frame as desktop physical truth. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:421` | Dirty spans include both halves of wide characters. | Useful pure helper candidate if Android has a real consumer. |
| `crates/cli/src/terminal_ui.rs:1587` | Local viewport pacer uses approximately 16 ms. | This is not a blanket delay for all live PTY output. |
| `crates/cli/src/terminal_ui/session.rs:498` | Snapshot candidate is presented/installed before ACK. | Desktop's existing presentation/ACK gate cannot silently change. |
| `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:63` | IME animation fence; one final measured resize. | Reuse lifecycle tracking rather than replace with timeout guessing. |
| `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:183` | New input epoch clears composition and restarts InputConnection. | Visual and input disruption must be measured separately. |
| `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:212` | Complete Canvas draw with clip/translation and retained pending/drawn sources. | Drawing only a subset without retained display-list/row content would be incorrect. |
| `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:288` | Geometry shift and hit testing share a transform; selection/history disable live pan. | A presentation handoff must preserve geometry and source identity together. |
| `crates/android/src/terminal.rs:594` | Resize changes input epoch, resets navigation and enters Synchronizing. | Existing cancellation rules cannot simply be removed to hide a progress indicator. |
| `crates/android/src/terminal.rs:657` | Snapshot is applied/ACKed by native actor before Kotlin's next draw. | Android protocol-applied and actually drawn baselines are already distinct. |
| `crates/android/src/terminal.rs:957` | Cursor glyph hidden while not Active. | Cursor disappearance is an independently observable visual change. |
| `crates/android/src/terminal.rs:1076` | Source origin, epoch, screen and viewport validate pointer/selection. | Never attach new-size hit testing to old-size displayed content. |
| `crates/terminal/src/ingress.rs:515` | Child synchronized-output mode 2026 is rejected. | Child batch preservation is currently missing, distinct from desktop outer synchronized output. |
| `crates/terminal/tests/security_policy.rs:223` | Tests explicitly expect immediate processing of rejected sync sequences. | Supporting 2026 changes an intentional security/processing contract. |

Current host/Android dimensions and input contracts are documented in
`.trellis/spec/backend/{terminal-model,terminal-driver,shared-client,local-daemon-ipc}.md`
and `.trellis/spec/frontend/android-app.md`.

## 3. Shared invariants, not one shared physical frame

A useful common lifecycle is:

    ordered validated terminal state
      -> eligible complete presentation state
      -> client viewport/chrome composition
      -> candidate visual difference against last committed display
      -> platform submission
      -> commit displayed geometry/source at that platform's defined boundary

This describes roles, not a required set of new objects or a refactor. Protocol-applied
revision, display generation and local geometry generation have different meanings.
A local pan can change the picture with no new terminal revision; a revision can advance
without changing any displayed cell. Desktop flush is not proof of physical monitor
scanout; Android onDraw is not proof of final GPU scanout. Preserve truthful boundaries.

Shared rules:

1. Compare final visual meaning, including palette resolution, text style, wide/combining
   glyphs, cursor and selection. A revision number alone is not damage.
2. Compare displayed coordinates after legitimate clipping/translation. Do not compare
   old logical row 35 with new logical row 35 when the old content is now on screen row 25.
3. An unchanged command line is not immutable; arbitrary child cursor addressing can edit it.
4. Receive and validate all required deltas even when only the latest candidate is painted.
5. Keep content, geometry, source identity and input-coordinate ownership coherent.
6. Preserve a complete usable display while waiting; never synthesize missing history or
   treat an arbitrary blank target as a disposable intermediate state.
7. Retain bounded state and explicit release; no per-revision snapshot collection.
8. Geometry/connection/lease changes invalidate the right interaction source. A visual
   optimization must not silently introduce input replay or weaken controller admission.

Already shared: semantic DTOs, surface validation, terminal/cache/selection semantics,
protocol recovery and key encoding. Possible additional shared code: a pure style-aware
row/cell damage helper and source/geometry transition values, only if both adapters need
them. Do not move CLI chrome into core or force native pixels into shared protocol types.

## 4. Platform-specific responsibilities and tradeoffs

| Concern | Desktop CLI | Android |
| --- | --- | --- |
| Physical output owner | External emulator receives ANSI; may alter cells on resize itself. | App owns Canvas layout, clips and transforms. |
| Update granularity | Exact cell runs directly reduce output bytes and visible writes. | Vsync/display lists decide GPU work; retained rows/RenderNodes may reduce CPU but need profiling. |
| Commit | One composed byte transaction; successful write/flush advances baseline. | One coherent source/geometry is recorded on draw, with retained source lifetime. |
| Resize | External reflow, wrapping, status/gutter positions and physical dimensions can invalidate pixel-position knowledge. | IME changes local visible area; no-pan/minimal-pan bridge to final host geometry. |
| Atomicity | Outer DEC 2026 helps only where supported; write_all/flush alone cannot force external atomic display. | One valid Canvas frame avoids exposing intra-draw clear as a separate frame; distinct semantic frames can still show a child clear. |
| Cadence | Existing approximately 16 ms viewport pacing; ordinary live updates are not globally debounced. | System frame callbacks plus actual IME animation progress; do not impose desktop frame rate. |
| Interaction | Terminal-reported cells, outer mode negotiation, mouse capture and local prefix commands. | Touch pixels, long press, handles, preedit, IME anchors and Activity/View lifecycle. |
| Recovery | Partial stdout failure invalidates physical knowledge; reconstruct faithfully. | Detach/recreation must retire handles and composition safely while App-owned connection persists. |

For desktop resize, keeping the old logical frame does not prove where the outer emulator
has placed its pixels. A safe candidate is full target coverage without a preliminary
clear when geometry is uncertain, within a synchronized-output transaction where supported.
All newly empty areas and retired chrome still require correct cleanup. Do not blindly
reuse cross-width cell offsets or promise that an external emulator never reflows.

For Android, the app can retain and transform old content precisely. However, a full-height
pixel screenshot per update is unnecessary. Keeping a candidate and displayed semantic
source plus coherent geometry is the starting point. Simply skipping unchanged draw calls
inside a rebuilt Canvas display list does not retain those omitted cells.

## 5. Two independent synchronized-output boundaries

    child TUI --2026 markers--> host semantic engine / publication
    desktop client --2026 markers--> user's external emulator

Desktop already implements the second boundary. It cannot reconstruct a child transaction
that the host split into several semantic states. Android has no external ANSI boundary;
it still benefits from correctly grouped host publication.

Prior research (already local; no need to retrieve the same conversation through trellis mem):

- `.trellis/tasks/09-02-migrate-alacritty-terminal/research/herdr-flicker-rendering.md`
  records pinned Herdr emitting outer synchronized updates and avoiding ED2 on resize when
  a prior frame exists. Its old descriptions of Zterm are historical, not current code.
- `.trellis/tasks/09-02-migrate-alacritty-terminal/research/alacritty-terminal-integration.md:64`
  records deliberate rejection of child 2026 to prevent delayed upstream raw parsing from
  bypassing incremental resource limits.
- Local pinned dependency `vte-0.15.0/src/ansi.rs:35` defines a 150 ms sync timeout and
  2 MiB byte buffer; `:304`, `:327`, `:370` show deferred processing/drain. These are upstream
  facts, not recommended product values or a claim Zterm drives the required timer today.
- `crates/terminal/src/engine.rs:200` feeds the parser through the bounded Zterm ingress.
  Merely forwarding 2026 would alter the timing assumptions of that path.

User follow-up on 2026-09-07: supporting DEC 2026 when needed is acceptable; treat support
as in scope rather than another product permission question. Support includes DECRQM state
query/replies so programs that probe the capability can activate it. This does not resolve
the separate UX tradeoff for output without markers.

Candidate host-owned batch publication would keep parsing/resource checks/PTY replies
progressing and retain the last published display until end or bounded expiry. This is a
candidate architecture, not yet a proven minimal patch. It must define revision/snapshot
publication, resize during a batch, ongoing history requests, initial attach, terminal end,
repeated begin markers, and malformed/missing end. One must not wait indefinitely, stop
PTY drain, or block ACK waiting for an update the host refuses to send before that ACK.

For unmarked output, no generic algorithm can distinguish final clear from partial repaint
solely by looking at an empty grid. A short collection interval can reduce visible stages,
but neither quiet-time heuristics nor diff can guarantee complete-frame detection.

## 6. Risks and concrete mitigations to design

| Risk | Needed contract / tradeoff |
| --- | --- |
| Fast open/close or repeated window sizes | Geometry intent identifies the current target; decode intermediate states but do not restart obsolete local animation. Latest dimensions alone cannot identify every transition. |
| Old layout displayed while remote layout changed | Coordinate-dependent input is fenced until source mapping is valid. Ordinary typing/preedit needs its own epoch policy, not a blanket visual workaround. |
| Keyboard closes and reveals new rows | Use only valid retained content; new authoritative rows may arrive later. Stable existing pixels do not imply zero latency for newly exposed content. |
| Theme/font/selection/cursor-only changes | Include resolved visual attributes and overlay damage. Wide glyph halves and old cursor cell require cleanup. |
| Palette changes after historical page capture | Preserve latest palette-resolution contract; semantic row equality alone cannot prove visual equality. |
| Output continues while old display is retained | Coalesce candidates, not PTY bytes. Avoid freezing all updates for an entire slow RTT. |
| Selection across actual resize | Existing geometry changes retire selection/source; retaining screen pixels must not accidentally claim old selection remains valid. |
| Hidden or transient TUI cursor | Cursor visibility differs from anchor position; keyboard avoidance must avoid chasing temporary repaint positions. No program/glyph heuristic. |
| Waiting for batch completion | Bound time and resources; report actual recovery separately, and guarantee progress if child crashes or omits end. |
| Input readiness confused with paint scheduling | A cosmetic improvement must not ACK an uninstalled state or accept stale commands. Desktop/Android ACK timing differs today. |
| Over-generalized renderer | Extra FFI types, CPU copies and ownership make a grand shared renderer costlier than shared rules and a few pure helpers. |
| Diff costs more than draw | Preserve a complete redraw path; compare row identities/semantic runs where useful. Do not add per-pixel image comparisons. |

## 7. Recommended options pending user decision

Common recommendation: keep existing Iroh/semantic protocol and client architecture; first
remove avoidable physical clearing/state churn and make resize geometry handoff coherent.
Treat child-batch support as a separate shared-host deliverable within the design discussion,
not an incidental Kotlin change or raw-parser flag flip.

For TUI output without explicit frame boundaries:

A. Preserve responsiveness: use existing presentation opportunities/coalescing, add no
extra wait to guess end-of-frame, and explicitly accept some child-origin intermediate
repaints. Recommended initial default. It avoids introducing a new typing/output delay.

B. Add a short bounded collection interval: may suppress more partial frames, at the cost
of added latency and delayed true clear; cannot guarantee zero flicker. Its placement and
budget must be decided without stacking host, network and client delays blindly.

The user's choice is still required. No numerical wait budget is approved. This is not a
question about whether to investigate: evidence work is already done.

Proposed implementation boundaries after convergence:

1. Shared behavior fixtures and source/geometry contract; reproduce the visible defect.
2. Android geometry/display/input-lifetime fixes and desktop no-unnecessary-clear resize
   behavior, each respecting platform constraints.
3. Child marked-update publication only with explicit resource/lifecycle/compatibility design.
4. Additional damage caches or unmarked batching only if the accepted scope/evidence needs them.

This ordering is provisional research, not an implement.md or an implementation approval.

## 8. Validation ownership

- Pure shared fixtures: unchanged prefix/new suffix, cursor-only/color-only changes, wide
  glyph replacement, same-size snapshot, different-size target, and required delta order.
- Desktop: exact ANSI transactions, absence of unnecessary ED2, complete retired-region
  cleanup, failed-flush baseline behavior, external emulator resize with/without 2026.
- Android: production View/IME animation, drawn geometry and hit source, preedit behavior,
  font/rotation, fast reverse keyboard transitions, history/selection source retirement.
- Host batching if in scope: split writes, actual clear, missing end/timeout, resize/reset/exit,
  history/snapshot consistency and incremental resource-limit tests.
- Visual acceptance: same fixture on desktop and Android, plus real Shell/Herdr; correlate
  geometry/revision/state with frames. Synthetic test-owned content only in persisted evidence.
  A semantic/byte test cannot by itself establish no visible flicker.
- Use focused checks required by actual modifications. Current brainstorm changes only docs;
  do not run builds or create expansive benchmarking infrastructure for this research turn.

## 9. Sources and verification limits

- Android display-list drawing model (retrieved 2026-09-07):
  https://developer.android.com/develop/ui/views/graphics/hardware-accel
- Synchronized-output semantics, detection, missing-end/timeout discussion (retrieved):
  https://github.com/contour-terminal/vt-extensions/blob/master/synchronized-output.md
- Historical pinned Herdr source link:
  https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/render_ansi.rs
  Live refetch failed in this turn; use retained project research as historical evidence,
  not a fresh claim about latest Herdr. No current Herdr runtime was instrumented.
