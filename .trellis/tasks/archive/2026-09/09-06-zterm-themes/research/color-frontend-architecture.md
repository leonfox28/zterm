# Research: color acquisition and desktop presentation architecture

- Query: Design the CLI side of complete color/theme protocol compatibility: acquire the physical terminal environment, separate terminal replies from keyboard input, refresh on protocol notifications, and render Session colors without contaminating the outer terminal.
- Scope: mixed; current repository source and official protocol references, with proposed designs clearly distinguished from existing behavior.
- Date: 2026-09-06
- Task: `.trellis/tasks/09-06-zterm-themes`
- Baseline: source inspected in the current checkout. No production code, tests, specs, or task lifecycle were changed. No physical terminal or Herdr conformance test was performed.

## Findings

### 1. Recommendation and boundary

Use a typed physical-terminal color environment supplied by the CLI to the Session's existing single controller. Keep application overrides, resets, stacks, effective colors, and child query replies host-authoritative. The CLI resolves semantic colors using the effective state received in the authoritative surface. It must not forward application color OSC directly or install the Session palette in the physical terminal.

Selection is already a local overlay and can support fixed/dynamic selection colors without any physical selection-color setter. Use the native cursor when the application inherits the outer cursor unchanged. Use a software block cursor when an application explicitly overrides cursor or cursor-text color. This avoids an otherwise unsound attempt to restore a native dynamic cursor from only a queried RGB value. The parent explicitly agreed with this cursor design in dispatch follow-up; block shape is a review-visible limitation, not an assertion that arbitrary native cursor shapes are emulated.

Two correctness problems need explicit design work rather than a new OSC handler alone:

1. The input stream currently treats unknown OSC as an Alt-key introducer plus ordinary payload, and the Active transition destroys queued raw bytes. Reply framing must be preserved across that lifecycle.
2. A color-only update must repaint the retained visible history with current effective colors. The current pinned-history delta path deliberately emits only input-mode changes.

### 2. Files found and current code patterns

| File / anchor | Responsibility and relevant current behavior |
| --- | --- |
| `crates/cli/src/terminal_ui.rs:179` | `run_guarded_terminal`: installs signals, enters raw/alternate mode, runs UI, restores guard. |
| `crates/cli/src/terminal_ui.rs:238` | `run_view`: starts sole stdin pump, performs prepare, presents initial snapshot, acknowledges it, starts `TerminalUiSession`. |
| `crates/cli/src/terminal_ui.rs:422` | `await_while_inactive`: temporary input codec while create/attach/initial ACK is pending; normal input is suppressed but local detach remains usable. |
| `crates/cli/src/terminal_ui.rs:527` | `transition_input_state`: inactive epoch invalidation and Active reader fence. |
| `crates/cli/src/terminal_ui.rs:616` | `prepare`: invokes `LocalRuntime::attach` or creates then attaches; supplies viewport only. |
| `crates/cli/src/terminal_ui.rs:860` | `TerminalGuard`: owns termios and duplicated descriptors; restoration covers normal exit, error, signal, and panic. |
| `crates/cli/src/terminal_ui.rs:1006` | `StdinEvent`: only raw epoch-tagged bytes, EOF, or error. |
| `crates/cli/src/terminal_ui.rs:1070` | `StdinPump`: sole reader, bounded channel, cancellation pipe, joined shutdown. |
| `crates/cli/src/terminal_ui.rs:1150` | `replace_after_active_fence`: drop old receiver, join reader, `tcflush(TCIFLUSH)`, advance epoch, install fresh reader. |
| `crates/cli/src/terminal_ui.rs:2241` | `HostInputEvent` and `HostInputCodec`: keys/mouse/paste/opaque CSI; no terminal reply type or OSC string framing. |
| `crates/cli/src/terminal_ui.rs:3005` | `apply_delta_with_writer`: stage candidate state; live view presents it, pinned history only calls `sync_input_modes`; commit occurs after output succeeds. |
| `crates/cli/src/terminal_ui.rs:3093` | `present_cached_viewport_stdout`: existing bounded history presentation scheduler. |
| `crates/cli/src/terminal_ui/session.rs:7` | `TerminalUiSession`: attachment, stdin, command mode, surface, presenter, selection, viewport, and input codec owner. |
| `crates/cli/src/terminal_ui/session.rs:165` | Input handling checks raw epoch before decoding, then routes through selection/copy and command mode. |
| `crates/cli/src/terminal_ui/session.rs:456` | `handle_event`: snapshot/delta/history/transport staging, exact revision application and ACK decisions. |
| `crates/cli/src/terminal_ui/session.rs:689` | Transport transition: existing reader fences and resize/resume ordering. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:20` | `DesktopPresenter`: single active output owner, committed input modes, selected overlay, last successfully flushed frame. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:129` | Selected cells currently XOR `style.inverse`; frame equality can skip all output. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:193` | Dirty runs compare semantic cells, including wide-cell repair. Palette changes alone are not currently representable. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:204` | Cursor currently emits its **pen style**, position, and visibility. Pen style is not the underlying cell's glyph style or an independent cursor color. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:320` | SGR encoder emits defaults, indexed/RGB fg/bg, bold/dim/italic, Boolean underline, inverse. |
| `crates/cli/src/terminal_ui/composition.rs:124` | `ComposedCursor`, `ComposedFrame`, and `compose_inner` combine live/history rows and chrome. |
| `crates/cli/src/terminal_ui/composition.rs:197` | History construction retains semantic rows when available but has a fallback to rows in the previous composed frame; never feed already resolved/selected/cursor-painted cells back into this semantic fallback. |
| `crates/cli/src/terminal_ui/selection.rs` | Selection source identity/range and wide-glyph normalization already exist; selected text extraction is independent of presentation color. |
| `crates/cli/src/terminal_ui/surface.rs:16` | Validated attachment candidate built from snapshot or contiguous delta. |
| `crates/core/src/terminal.rs:135` | `TerminalColor` preserves Default/Indexed/RGB; `TerminalStyle` lacks underline style/color; `TerminalSurface` lacks palette/appearance. |
| `crates/core/src/viewport_cache.rs` | Renderer-neutral history cache stores semantic rows; color changes must not manufacture history identity changes. |
| `crates/daemon/src/client/view.rs:256` | `PreparedTerminalView`: private transport handles, initial semantic snapshot, exact ACK, optional takeover. |
| `crates/daemon/src/client/view.rs:499` | `TerminalViewCommandWriter`: typed commands, no color-environment operation. |
| `crates/daemon/src/client/session.rs:307` | Initial attach wire request; `:520` constructs reconnect attach from saved local state. |
| `proto/zterm/v2/terminal.proto:7` | Attach request has viewport/resume state; field 9 is reserved and must not be reused. |
| `proto/zterm/v2/terminal.proto:146` | RGB/style/surface/delta/history DTOs have no environment or effective color state. |

### 3. Related specs

- `.trellis/spec/backend/terminal-model.md`: sole host parser/grid; semantic downstream values; no upstream engine type leakage; bounded hostile-input handling; current color/underline exclusions must be updated by the main session when implementation changes the contract.
- `.trellis/spec/backend/local-daemon-ipc.md:309`: cancellation and input fences. `:412` selection source/commit rules. `:441` input-mode projection. `:460` sole buffered presenter. `:535` history/paste transitions.
- `.trellis/spec/backend/terminal-input-commands.md`: one codec -> selected-copy priority -> `CommandMode` -> existing forwarding gates. Terminal replies must bypass command mode entirely, including during inactive waits.
- `.trellis/spec/backend/session-service.md:108`: one controller; takeover commits a new generation atomically. A pending takeover's observed physical colors must not mutate the old controller's effective Session state.
- Frontend guideline files were read but remain placeholders. Concrete CLI contracts above are the relevant source-backed guidance.
- Workspace forbids unsafe Rust and denies Clippy failures. No new raw/FFI terminal backend is needed.

### 4. Physical environment: proposed types and acquisition

Use a transport-neutral, fixed-size environment type in core. Suggested logical fields (names remain parent design choices):

```text
ColorObservation = Unknown | Fixed(Rgb) | Dynamic
OuterEnvironment = {
  palette[256]: Unknown | Fixed(Rgb),
  foreground, background: Unknown | Fixed(Rgb),
  cursor, cursor_text, selection_foreground, selection_background: ColorObservation,
  appearance: Unknown | Dark | Light
}
```

`Dynamic` and `Unknown` are not interchangeable. An unanswered request is Unknown; a terminal's explicit dynamic report is Dynamic. Palette entries cannot be Dynamic. Appearance preference is also separate from the measured background and from whole-screen reverse video. Do not manufacture an OS preference by classifying RGB brightness or reading terminal-brand environment variables.

Keep client-only capabilities/provenance outside the Session's effective colors: whether mode 2031 was reported set/reset/permanently-set/permanently-reset/unknown, whether OSC21 understood a key, which selectors replied, and whether an acquisition boundary was established. The Session does not need raw response bytes or terminal identity strings.

Initial collection should finish or expire **before** `prepare` submits create/create-main, so a newly spawned child has the best available base at first output. This local collection must not block daemon PTY drain, and it must keep signals/local detach usable. Pass the resulting environment through create/attach APIs instead of sending fake child input after attachment becomes Active. For an existing Session, the host stages the new controller's environment and applies it at the agreed lease boundary; host-side ordering belongs to the parent design.

Recommended probe set, emitted by one output owner:

- OSC 10/11/12/17/19 queries: default foreground/background, cursor, selection background/foreground.
- OSC 4 queries for all indices 0–255. Emit one selector per request or small bounded batches; do not assume indices 16–255 match a built-in cube/grayscale table.
- OSC21 queries for cursor, cursor_text, and both selection roles (optionally defaults as well). This distinguishes Dynamic from fixed RGB. Prefer a valid OSC21 observation over a legacy reply that cannot represent dynamic state. Unknown keys can still use legacy fixed-color evidence if available.
- `CSI ?996n` appearance inquiry, and `CSI ?2031$p` mode inquiry.
- Optionally an ordinary status inquiry after the set as an ordered-stream completion marker, subject to the limitations in section 5. It must never be treated as a transaction identifier.

Query frames must be short enough for the receiver's bounded parsing. A useful implementation bound is one 1 KiB outer OSC frame and one 64 KiB aggregate acquisition budget, with fixed selector bitsets and one named local response deadline. These are **proposed limits**, not existing constants or protocol requirements. Specify the final timeout in code/tests as a local collection deadline, not a periodic polling policy. One missing color must not prevent supported observations from being retained.

No environment, terminal name, process name, screen-text, or inferred Herdr/Pi behavior participates in acquisition. No automatic timer repeatedly polls the palette. New rounds are triggered only by entry, an actual outer notification, an explicit lifecycle refresh, or a user-requested refresh if such a command is later approved.

### 5. Reply demultiplexing and the no-ID limitation

Integrate reply framing into the sole host-input decoder (a dedicated implementation module is reasonable, but not a competing stdin reader). Add a typed event such as `HostInputEvent::TerminalReply(OuterReply)`. Extract it before selected-copy, prefix cancellation, history retention, or child encoding. A terminal reply must not invalidate a selection or complete/cancel a pending Ctrl+] command. Inactive prepare/ACK waits must share this owner rather than constructing a fresh decoder which discards its state.

Recognize only exact expected reply grammars and valid bounded values. Handle fragmented introducers/terminators, BEL and ESC-ST, multiple replies plus keys in one read, duplicate/out-of-order selectors, cancellation and overflow. Preserve unrelated CSI/Alt/SS3/key behavior. An OSC reply frame cannot be split into `Opaque(ESC ])` and printable payload as the current codec does. Malformed/oversized recognized color reports are consumed to their framing boundary with content-free classification, never forwarded as a suffix. Unrelated complete controls retain the established opaque-input policy.

Paste remains opaque: bytes inside a bracketed paste, including apparent OSC, DSR, Ctrl+], and mouse reports, are user text. The decoder must resolve paste ownership before control-reply recognition. Do not add a regex over raw read chunks. If accepting raw C1 replies, distinguish them from UTF-8 continuation bytes; queries can remain canonical 7-bit, and C1 support must be tested rather than assuming every 0x9d byte begins OSC.

Maintain one bounded outstanding collection and one coalesced refresh-needed bit. Collect individual selector observations; never append unlimited responses or launch overlapping rounds for a notification burst. A reply arriving after a closed round is consumed, not keyboard input, and must not silently complete a different acquisition generation.

**Protocol limitation:** these outer replies have no request ID. A local generation counter identifies ZTerm's own work but does not prove which query produced an incoming selector/value. There is no strictly portable way to identify an arbitrarily delayed old response after issuing an identical new query. An ordered response marker can delimit a round only for terminals whose response stream follows processed request order; it cannot turn a missing marker into proof of completion. Do not claim that a timeout authenticates subsequent replies.

Conservative recommendation: serialize acquisition rounds; after deadline keep unresolved reply slots in consume-only draining state. Do not reuse that unresolved selector generation until its ordered boundary is observed or the physical stream is freshly established. A notification can queue one subsequent round but cannot force reuse of an ambiguous outstanding generation. Unsupported/missing-response terminals retain Unknown for unavailable observations and a documented lack of reliable live refresh. Exact treatment of permanently missing boundary replies is a parent design decision, and must be explicit in acceptance tests rather than hidden by a millisecond delay.

### 6. Input epoch fences require a narrow redesign

The current Active fence (`terminal_ui.rs:1150`) discards the receiver and calls `tcflush(TCIFLUSH)`. It is correct for the existing byte-oriented keyboard admission contract, but cannot preserve a partly consumed terminal response. Simply resetting the codec at that point is insufficient: the prefix may have been read before the fence while a late payload suffix arrives after it. Conversely, preserving an unterminated OSC state after the kernel discarded its terminator may swallow the next real key.

Recommended implementation direction for a strict reply/input guarantee:

1. Give terminal-control framing a physical-stream lifetime, distinct from the keyboard admission epoch. Frames retain their beginning epoch; stale keyboard/paste units stay inadmissible.
2. Keep the sole-reader join boundary. Replace destructive unparsed flushing with a bounded safe drain through that same decoder while rejecting stale keyboard events. The reader must return/handoff the decoder state so no partial reply is forgotten.
3. Establish the new keyboard epoch only after old-reader shutdown and the kernel drain boundary, then start the replacement reader with retained control framing. Do not retain ordinary stale keyboard input for later replay.
4. A reply outstanding across an epoch transition is observation work, not child input; it remains typed and obeys collection generation rules. On transport reconnect, keep the latest completed local environment, coalesce changes, and send it under the current attachment's lease only.
5. A partial frame without a terminator still requires the same explicit size/cancellation/deadline policy. No decoder may wait forever or store unbounded bytes. Literal keystrokes are not proof of protocol completion.

This is a change to the input-fence spec and regression fixtures, not an implementation detail to conceal. It must retain the existing stale-reader race guarantees, copy/prefix behavior, and complete-paste-on-resume rule. Keeping destructive `tcflush` instead would require accepting a documented best-effort limitation for replies split across the fence; merely adding sequence tombstones does not solve a suffix whose introducer was removed.

### 7. Runtime mode 2031 ownership and cleanup

The CLI's outer subscription is independent of each child application's virtual subscription. The CLI needs physical notifications to maintain base colors even when the child has not subscribed, or has explicit overrides covering the currently visible defaults. Continue recording fresh base observations under overrides so a later reset reveals the current outer environment.

Probe the outer subscription mode before changing it. Preserve an already enabled mode. Enable a reported reset mode and record that ZTerm owns the change; restore it to reset on exit. A reported permanently enabled mode needs no change; a permanently disabled/unknown mode must not be enabled speculatively. Do not unconditionally send `?2031l` on exit and disable a pre-existing owner. The guard should retain the compact restoration action and execute it before releasing raw-mode ownership on every normal/error/signal/panic path.

An incoming appearance notification requests a coalesced refresh of the actual palette/default/special colors, including changes which keep the same Dark/Light value. It is not a complete palette snapshot. Read the updated values, send the typed environment through the normal controller command boundary, and render only the authoritative effective-state update returned by the host. Do not locally mutate the effective Session state in advance of that update. A physical theme may visibly change before the serialized update arrives; this is an asynchronous external event, not grounds to bypass Session ordering.

Do not forward the raw outer notification into the child. The host decides whether the child subscribed and writes its canonical notification after installing its effective/base state. Child application setters should not feed back into outer observation because rendering never modifies the outer palette.

### 8. Semantic resolution and full/history repaint

Add effective color data to core surface/delta and therefore `AttachmentSurface`; a palette-only revision can have zero row patches. Keep semantic rows unmodified in the surface and history cache. Resolve defaults and indices in the presentation path, using the same effective values used by host queries. Explicit RGB values remain explicit. For unavailable inherited slots, preserve semantic default/index output and unknown reporting instead of inventing RGB values. Keep bold/dim as rendition attributes; do not introduce brand-specific brightening or gamma heuristics.

Use separate semantic composition and resolved physical comparison. In particular, `DesktopPresenter::baseline` currently also supplies semantic fallback rows while history is loading. If it becomes a resolved/selection/cursor-painted frame, that fallback would permanently bake old colors/overlays into future composition. Retain a semantic baseline plus a resolved baseline, or remove the dependency by retaining the last semantic viewport in its existing owner. Resolution must never write back into semantic cells.

When color state changes, either compare newly resolved cells to the resolved baseline or explicitly invalidate affected presentation rows. The former naturally changes only cells that reference modified slots. Full repaint is acceptable as a simpler initial optimization boundary given the 240x80 child limit, provided it repairs styled blanks and history. Palette-only state cannot be skipped by the existing `baseline == desired` check.

Pinned-history handling must be changed deliberately: `apply_delta_with_writer` currently calls only `sync_input_modes`. Ordinary hidden live text changes still should not repaint visible history, but an effective-color change must recompose the **retained visible history rows**, selection, and chrome as needed. It must not replace history with live rows or create a history epoch just to redraw colors. Preserve the staged surface/viewport/selection commit and one successful outer transaction before marking the revision/presentation applied.

The metadata needs an effective-color identity (or equivalent equality) which survives a zero-row delta. A history response captured at an older revision supplies semantic text, not an old color theme; apply the latest authoritative effective colors when it is shown. Late history responses cannot roll back the current palette. Existing row/anchor validation remains authoritative.

Child surface and ZTerm chrome need separate resolution contexts. Child OSC default changes should recolor child cells, including erased blanks, without implicitly recoloring the status bar and gutter. Chrome can continue following the observed outer base until product theme roles are separately defined. No physical OSC palette/default setter is required for either region.

Whole-screen inverse must be a Session presentation transform confined to child content, not physical `CSI ?5h` which would also affect chrome/other outer state. The exact legacy transform and its interaction with default/explicit colors belongs to the shared protocol contract; this research does not assert that blindly XORing every cell's SGR7 matches every xterm case. Apply the agreed screen transform, then per-cell rendition, then selection/cursor overlays exactly once. Keep appearance preference separate.

### 9. Selection and cursor details

Selection overlay:

- Read the existing `SelectionSourceIdentity` and expanded range; recolor only a cloned desired child frame.
- Resolve explicit selection foreground/background independently. Dynamic selection follows the agreed reverse-video mapping of the displayed underlying cell; Unknown inherited selection should retain the existing local reverse-video behavior as the explicit fallback policy, not pretend that an outer fixed color was measured.
- Apply selection after ordinary cell/screen inversion. Two inversions should not accidentally double-apply. Explicit underline color stays explicit; default underline color follows the final displayed text foreground.
- Repainting changes no extracted text, selection coordinates, clipboard payload, keyboard elevation, or source identity. If only color state changed, do not cancel a still-valid selection as though its text had been replaced. Current live delta selection cancellation is unconditional and needs an explicit color-only preservation decision.

Software cursor:

- `TerminalCursor.style` is the pen for subsequently printed cells. Draw the cursor using the actual cell at its current row/column, after ordinary rendition and selection; do not replace the cell's glyph/style with the pen template.
- A fixed cursor color is the block background. A fixed cursor-text color is the glyph foreground. Dynamic cursor/background uses the agreed underlying displayed foreground, and dynamic cursor-text uses its displayed background. Keep source colors and these resolved display roles distinct.
- Explicitly setting a dynamic cursor role is still an application override: its dynamic mapping follows the virtual Session, not a native outer cursor whose own defaults may differ.
- Hide the physical cursor in the same transaction that paints the software cursor, but still position the physical cursor with CUP for input-method/preedit positioning. Do not emit native cursor-color OSC.
- When switching back to inherited cursor behavior, erase the software overlay by repainting the underlying cell, then enable the physical cursor according to the existing child visibility policy.
- A hidden cursor, history view, retained history during ResumePending, or coordinates outside the displayed child area never gets a software overlay. Child cursor movement while history is shown must not draw over history or chrome.
- Normalize a cursor on a wide continuation to the wide head and paint/dirty the whole grapheme span, including both columns. Never write a continuation as an independent glyph. Preserve combining characters and styled blanks. Test the chosen wide-block width policy explicitly.
- Cursor movement dirties both old and new spans; selection/color changes under a stationary software cursor must redraw that span. Toggle visibility and native/software changes also invalidate those spans. The overlay belongs in physical-frame equality and dirty runs.
- No cursor shape/blink fields exist in the current core DTO. The software fallback is a steady block for explicit color overrides. Native inherited shape/blink remains outer behavior. Adding shape protocols or a blink timer would be a separate scope change.

For unresolved semantic defaults that need swapping, retain an explicit source token or inverse encoding; blindly swapping `TerminalColor::Default` loses whether the source was the outer foreground or background. Unknown must not be converted to black/white to make a software overlay convenient. Document any combination which cannot be represented on the physical SGR target with unknown source colors, rather than claiming an exact measured result.

### 10. Physical restoration hard limitation

Legacy OSC12 reports a current RGB but does not reliably establish whether the physical cursor is configured as fixed RGB or dynamically derives from text. Restoring a queried RGB can freeze a formerly dynamic cursor. Resetting with OSC112 returns configured defaults and can discard a pre-existing runtime override. A color stack is also a physical mutation with unrelated owners and unsupported-terminal behavior. Query/set support is not safely inferred from TERM strings.

Therefore the approved recommendation does not use native color setters, write-then-query probes, synthetic RGB restoration, or outer color-stack fallback. Selection and all Session palette/default changes are painted locally. The only new outer mutable state needed here is a capability-probed mode-2031 subscription whose prior mode is recorded. `TerminalGuard` keeps its existing termios, input-mode, cursor-visibility, and alternate-screen cleanup plus that owned subscription restoration.

### 11. Test plan and existing test seams

No tests were run for this research-only task. Implementation should add meaningful behavior tests at the existing seams:

| Area | Required cases |
| --- | --- |
| Decoder | Whole/one-byte/random chunking; mixed keyboard+OSC/CSI replies; BEL/split ST; duplicate/out-of-order selectors; malformed RGB/index/appearance; oversized strings; UTF-8/C1 ambiguity; replies inside paste stay exact paste. |
| Input routing | Color reply between Ctrl+] and `.` does not cancel detach; between selection copy press/release does not alter lease; reply never enters child input/history resume buffer. Unrelated Alt/CSI/mouse/enhanced keys retain exact behavior. |
| Collection | Partial support, no replies, unknown OSC21 key, explicit dynamic vs fixed black, all 256 entries, precedence of OSC21 dynamic over legacy RGB; fixed total bound and deadline; notification burst coalescing; late reply and missing marker cannot complete a new generation. |
| Fence | Reader-observed-but-not-enqueued reply; old queued reply; OSC split before/during/after kernel drain; stale keyboard stays rejected; post-fence keyboard remains accepted; reconnect during incomplete paste retains existing exact-once behavior. |
| Subscription | Initially set/reset/permanently set/permanently reset/unsupported; only owned enable is reversed; normal detach, signal, error, and panic restore correctly; outer mode restoration precedes raw release. |
| Attach/controller | Initial child color query sees supplied base; pending takeover cannot affect current controller; successful takeover applies replacement base; reconnect reuses latest completed environment without replaying stale commands. |
| Renderer | Default/index/RGB coexist; changed palette entry with zero row patches redraws references and styled blanks; unchanged RGB remains unchanged; same-appearance palette change; base refresh beneath a child override followed by reset. |
| History | Color-only update redraws retained history without moving offset or changing text; old history response cannot restore old palette; incomplete cache retains prior complete semantic rows; visible selection remains valid across pure palette changes. |
| Underline/inverse | Full underline enum and indexed/RGB/default underline color; explicit underline color survives reverse video; default follows displayed foreground; SGR58/59 mixed with ordinary fg/bg is preserved; whole-screen inversion leaves chrome unchanged. |
| Cursor | Inherited native vs fixed/dynamic explicit software; distinct cursor background/text; pen differs from underlying glyph; old/new wide spans, combining/blank cells, selected cell, hidden cursor/history, return to native; no native color OSC in output. |
| Atomic commit | Partial write/flush failure leaves candidate surface/colors/selection/viewport/cursor uncommitted, clears physical baseline, and next frame fully repairs. Successful color frame remains one buffered DEC2026 transaction and one flush. |

Existing anchors: `terminal_ui.rs:4450` atomic presenter; `:4578` selection/mode commit; `:4646` hidden-history mode-only delta; `:5211` delta failure staging; `:5328` flush failure recovery; `:5400` partial write recovery; `:5512` fragmented host codec; `:6192` guard lifecycle. `terminal_ui/session_tests.rs` provides live UI event/input fixtures; `crates/cli/tests/daemon_autospawn.rs` provides production multiprocess PTY coverage. Extend the existing generic fixtures instead of adding Herdr-specific behavior or test-only renderer markers.

Suggested validation after implementation: `cargo test -p zterm-cli --lib`, the relevant `daemon_autospawn` scenarios, core/proto/terminal/daemon color tests from the parent plan, `cargo fmt --all --check`, and repository-required Clippy/check gates. Pure PTY fixtures establish byte/protocol behavior; real terminals are still needed to assess native IME and underline glyph rendering.

### 12. Primary external references

- [xterm control sequences, patch 411 dated 2026-08-23](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html): OSC4 indexed colors; OSC10/11/12 and selection 17/19; corresponding reset controls; private mode inquiry; legacy screen inverse. Used to identify protocol families and wire framing, not to infer universal terminal support.
- [Contour appearance/palette notification extension](https://contour-terminal.org/vt-extensions/color-palette-update-notifications/): `?996n` inquiry, `?997;1n` Dark / `?997;2n` Light, mode 2031 notifications. Notifications also cover a profile palette change without a Dark/Light transition; they do not contain the complete palette. The specified notification cause is the terminal's own palette/profile change.
- [Kitty color control](https://sw.kovidgoyal.net/kitty/color-stack/): OSC21 distinguishes fixed color, empty dynamic value, and unknown key reported with `?`; cursor-text and selection roles exist. Dynamic role examples are advisory, so ZTerm must define its own exact overlay mapping. Legacy xterm replies cannot express every dynamic/unset state.
- [Kitty colored/styled underlines](https://sw.kovidgoyal.net/kitty/underlines/): 4:0–5 style values, 58 color, 59 reset. Explicit underline color remains unchanged by reverse video; inherited underline follows foreground. These are output capabilities of the outer renderer too; ZTerm cannot create missing native underline glyph support through ANSI alone.

## Caveats / Not Found

- This research does not prove the Herdr screenshot's root cause and does not inspect that user's remote runtime configuration.
- No exact, portable recovery of an unknown native dynamic cursor is possible from a legacy RGB query alone. Software block fallback avoids that problem and is explicitly accepted by the parent.
- No-ID outer queries cannot provide cryptographic or perfectly portable per-round freshness; arbitrary delayed old replies remain ambiguous without a proven response boundary. Unknown/degraded cases must remain explicit.
- The existing destructive input fence and partial reply safety conflict. Strong safety needs the narrow shared-decoder/drain redesign described above and an updated executable input-fence contract.
- An outer terminal that neither reports relevant colors nor emits notifications cannot be made to report true measured values by ZTerm. Preserve unknown/inherited behavior; do not advertise invented RGB or infer appearance from a terminal brand.
- Whole-screen reverse-video details, exact appearance policy under child overrides, color stack semantics, and host protocol schema/versioning are parent-owned decisions. This document identifies the frontend obligations and does not settle them independently.
- Actual underline shapes/colors and software cursor IME behavior need physical-terminal verification; fake PTY tests alone establish encoded sequences, not glyph appearance.
