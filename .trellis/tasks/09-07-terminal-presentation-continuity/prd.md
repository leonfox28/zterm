# Cross-client terminal presentation continuity

## Goal

Keep unchanged visible terminal content stable through ordinary output, local viewport changes,
and resize on the existing desktop CLI and Android client. Show real changes correctly and keep
healthy interaction responsive. Remote programs continue receiving the actual usable terminal size.

Product decisions converged on 2026-09-07. The user's latest “按你说的来” accepts the recommended
policy: no additional end-of-frame guessing delay for output without DEC 2026 markers.

## Scope

In scope:

- Existing desktop-to-desktop CLI and Android-to-host connections, with application-independent behavior.
- Local clipping/minimum panning during height transitions and coherent handoff to authoritative updates.
- Android integer-row content layout, with rounding remainder **below the toolbar**.
- Host DEC 2026 synchronized-output markers, detection/state query, bounded recovery, and consistent read paths.
- Desktop correct diff/coverage, Android coherent complete draws, and healthy-resize input/IME continuity.
- Focused source, protocol, geometry, input and visual acceptance for the changed boundaries.

Keep Iroh, the host Alacritty engine and semantic snapshot/delta transport. The design uses the existing
wire format; internal host publication and local client/FFI state may change. Cross-size wire deltas,
predictive echo, disconnected input replay, a replacement transport, a second terminal parser,
per-program handling, iOS and a new desktop GUI are outside this task.

## Requirements

| ID | Contract |
| --- | --- |
| PRES-1 | Compare final display meaning, including text, colors/styles, wide cells, cursor, selection and client chrome. An executed Shell line remains mutable. Update every actual changed region, not only an assumed TUI top bar. |
| PRES-2 | Preserve usable displayed content during resize. Compare the old frame after its legitimate clip/pan with the candidate at screen coordinates. Commit content, geometry and source ownership coherently. A new-size snapshot, its ACK, or the first ESU after resize does not prove that the application has completed its new layout. |
| PRES-2a | Within existing viewport limits, Android stable content height equals rows times cell height. Keep the terminal directly adjacent to the fixed-height toolbar; place remaining pixels below the toolbar, above the system navigation safe area or docked IME. Preserve system insets without double-counting overlapping IME/navigation space. Measurement, drawing, clipping, handoff, hit testing and IME anchors share one geometry. Animation may use smooth intermediate pixel translations/clipping; endpoints align to whole rows. Retain existing capped/tiny-window fallback; large excess beyond the row cap is not rounding remainder. |
| PRES-3 | Distinguish ordered protocol application, display eligibility and actual platform submission. Validate required delta dependencies even when coalescing display candidates. ACK only the exact installed update required by its origin; never wait for a later repaint to ACK an already valid snapshot. |
| PRES-4 | No artificial delay to guess completion of unmarked output. Use existing presentation opportunities to coalesce latest candidates. Some child-origin partial redraws may remain visible. Real final clear and layout changes must converge correctly; blank content is not evidence of an incomplete frame. |
| PRES-4a | Support DEC 2026 begin/end and capability/state query. Keep parsing, resource checks and necessary PTY replies progressing. Normally closed batches do not expose internal redraw states. Independent bounded timeout, reset, resize or exit recovery may end an unfinished batch and expose its current state; never freeze indefinitely. |
| PRES-4b | Capture/freeze eligible state at actual parsed boundaries, not merely by suppressing notifications. Ordinary updates, explicit snapshots, new attachments and history reads must not bypass that boundary or combine incompatible metadata. Complete A plus incomplete B in one ingest must retain a coherent eligible state. One public revision denotes one exact state. |
| PRES-5 | Share semantic correctness and transition rules, with small pure helpers only when both clients actually consume them. Keep ANSI/outer-emulator behavior on desktop and Canvas/insets/IME behavior on Android. Retain desktop viewport pacing and Android native frame scheduling; add no global debounce. |
| PRES-6 | Fast/reversed geometry changes, ongoing output, history/selection, reconnect/takeover and old input sources must not produce stale transitions, wrong-coordinate clicks or implicit replay. Actual geometry changes retire affected selection/copy sources. |
| PRES-6a | A healthy resize of the same live attachment preserves local composing text and keyboard continuity where the current controller remains valid. Coordinate-dependent interaction is separately fenced. Actual reconnect, loss of control, Session change or end still retires stale input. Accepted text/key units are sent once or receive an explicit failure; no silent discard or reconnect replay. |
| PRES-7 | Bound model/published/candidate state and retained sources. Reuse existing cache/queue budgets; no per-revision frame queue, screenshot stream, second parser or unconditional full-history copy. Optimize repeated projection/copy only where necessary for the accepted behavior or supported by measurements. |

## Observable acceptance

| Case | Required result | Requirements |
| --- | --- | --- |
| Same-size Shell output with an unchanged prefix | Desktop emits only needed cell runs when the physical baseline is valid; Android's unchanged screen content remains stable. | PRES-1/5 |
| High/low cursor, keyboard opening and closing | Clip if the cursor fits, otherwise pan only enough; submit settled actual dimensions; hand off without an extra reset of content position. | PRES-2/2a |
| Non-integral available height | At 17 px per row, 1000 to 600 px available space gives 986 to 595 px content and 14 to 5 px padding below the toolbar. Old cursor row 39, zero-based, has y=578 px before and after the 5-row handoff. Controls retain normal size; touch/IME match the draw. | PRES-1/2/2a |
| Unknown desktop physical baseline, including X becoming blank | Cover all owned target cells that may be stale, including blanks and retired chrome; do not assume unknown cells are already empty. | PRES-1/2 |
| Width/font changes or new rows without cached history | Show correct authoritative reflow and valid available content; do not fabricate missing rows or promise unchanged positions across true layout changes. | PRES-2/6/7 |
| DEC 2026 query, split markers and A-complete/B-incomplete input | Correct query replies and complete eligible states, independent of read chunking. | PRES-4a/4b |
| Snapshot, attach and history request during a batch | Return one eligible snapshot/read, or wait in a bounded cancellable path; no partial-state leak, actor/PTY blockage or ACK cycle. | PRES-3/4b/7 |
| Begin with no further output; repeated begins; reset/resize/EOF | Timer/lifecycle handling makes progress, metadata stays coherent, and terminal exit is not hidden behind a batch. | PRES-4a/6 |
| Unmarked output and a real final clear | No added guessing timer; ordinary updates remain responsive and the final empty display is not suppressed. | PRES-4 |
| Rapid A to B to A geometry and late responses | Apply required protocol states but do not restart obsolete visual transitions; last desired dimensions win and snapshot ACKs remain correctly correlated. | PRES-2/3/6 |
| Composing Chinese while healthy keyboard geometry changes | Preserve composing text and usable input connection; committed text arrives exactly once. Keep mouse/selection on valid geometry. | PRES-3/6/6a |
| Reconnect, takeover, Session end or stale UI callback | Existing stale-input/lease protections remain effective; no queued input is replayed into a replacement connection. | PRES-6/6a |
| Release of old frames and sources | Bounded ownership with explicit retirement; no accumulation proportional to update count. | PRES-7 |

Model/byte tests do not substitute for desktop and Android visual acceptance. Record observed blank
frames, extra displacement, input interruption and frame-budget misses separately. Do not claim a
runtime improvement based only on source inspection or the arithmetic fixture.

## Technical evidence and root-cause classification

Baseline: `be66a16` on main. Detailed evidence is retained in the two research documents.

| Finding | Evidence / owner | Classification |
| --- | --- | --- |
| Same-size host row deltas exist; geometry/screen changes use snapshots. | `crates/terminal/src/model.rs:265` | Existing mechanism; full snapshot alone is not a visible-clear defect. |
| Desktop skips identical frames and diffs cell runs, but incompatible layout/size invalidates baseline. ED2 is already inside outer 2026. | `crates/cli/src/terminal_ui/ansi_presenter.rs:176`, `:185`, `:238`, `:421` | Coverage change belongs to this presenter; actual flicker cause still needs visual evidence. |
| Android has IME animation fencing and local clip/pan, but old/new bottom remainder can differ. | `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:63`, `:182`, `:183`, `:288`; 17 px arithmetic fixture in research | Local geometry contract defect; counterexample is arithmetic, not a new device reproduction. |
| Resize currently changes native input epoch; non-Active projection hides cursor and View resets composition. | `crates/android/src/terminal.rs:594`, `:957`; `TerminalView.kt:183`; `AppRepository.kt:304` | Interaction/geometry boundary needs distinct ownership, not just cosmetic hiding. |
| Host explicitly rejects child 2026 to preserve incremental resource checks. | `crates/terminal/src/ingress.rs:515`; migration research `alacritty-terminal-integration.md:64` | Missing presentation-batch ownership; forwarding into upstream raw buffering is insufficient. |
| Direct snapshot/history reads bypass revision notifications; same ingest may contain several batch boundaries. | `crates/daemon/src/terminal_driver.rs:744`, `:777`, `:783`; `crates/terminal/src/model.rs:149`, `:205` | Host publication boundary change required across equivalent reads. |
| Desktop viewport pacing is about 16 ms, not a blanket timer for live PTY output. | `crates/cli/src/terminal_ui.rs:1587` | Preserve existing scheduling rather than infer new latency approval. |

Network savings, CPU/GPU work and visible continuity are separate measures. The host's height-only
resize rule supports clipping/minimum pan; real TUI layout changes can still affect any region.

## Deferred work and limits

- Earlier submission of a reliably known final size to overlap IME animation with remote resize is a
  separate optional optimization. First implementation retains settled-size submission.
- Android row/RenderNode caches and broader projection optimization require a demonstrated need.
- Unmarked application frames cannot be identified perfectly; no all-program zero-flicker guarantee.
- Outer desktop emulators can reflow independently and may not support synchronized output.
- Explicit batch timeout/recovery prioritizes progress over retaining a never-completed frame.

## Planning status and artifacts

Product choices are resolved. `design.md` specifies ownership, compatibility and behavior;
`implement.md` specifies change order and validation. Product code is implemented;
whole-transition visual acceptance remains incomplete. `research/runtime-acceptance.md`
records the Android keyboard-close blank/history-refill observation and desktop GUI gap.
On 2026-09-08, after reviewing those limits, the user instructed “先走发布流程吧”.
Proceed with the current implementation as v0.1.27 through the canonical commit/PR/CI/
merge/tag/signing/publication workflow. Preserve the visual issue as open follow-up;
release authorization does not convert incomplete flicker evidence into a pass.
Do not install/update the running local product or restart its daemon.
The user approved implementation on 2026-09-08. Android emulator testing is authorized; testing must
not touch the existing macOS `main` Session or restart the user's running macOS daemon. Prefer
independent test hosts and state directories. The unrelated e2e-hardening work remains separate.

Research: `research/cross-client-presentation.md` and `research/second-review.md`. Historical open
questions in those records are superseded by this converged PRD and its design.
