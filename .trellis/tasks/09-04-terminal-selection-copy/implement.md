# Terminal selection and clipboard implementation plan

## Execution model

- Remain in planning until the user approves the final planning summary in a fresh turn.
- After approval, run `task.py start`, then dispatch one `trellis-implement` worker for the coherent
  cross-layer slice and one independent `trellis-check` reviewer. The implementation is sequential
  because terminal model, Session routing, protocol, and the single CLI event loop share ordering
  contracts and would conflict under file-level parallel edits.
- Preserve the unrelated existing modification to
  `.trellis/tasks/09-02-migrate-alacritty-terminal/task.json`.
- Do not add compatibility adapters, configuration, application/terminal-name branches, native
  clipboard commands, unsafe Rust, or a second VT parser.

## Step 0: Start and lock the baseline

- [x] After fresh approval, start task `09-04-terminal-selection-copy` and load `implement.jsonl`.
- [x] Record git status and current focused test results; keep the pre-existing task metadata edit out
  of this task's commit.
- [x] Confirm `alacritty_terminal = 0.26.0`, v2 wire kind 322 availability, the 1 MiB control payload
  limit, the 80x240 surface bound, controller/takeover transitions, and all terminal guard cleanup
  paths before modifying code.
- [x] Map every PRD acceptance item to an owner test. Treat Cmd+C delivery, effect routing, and pointer
  ownership as architecture contracts, not manual-only polish.

## Step 1: Core domain values and visible-source identity

- [x] Add `MAX_TERMINAL_CLIPBOARD_BYTES`, validated/redacted `TerminalClipboardWrite`, and
  `TerminalHostEffect`; give `TerminalUpdate` one optional transient effect outside
  `TerminalSideEvent`.
- [x] Add a validated five-bit `TerminalKeyboardFlags` domain value to `TerminalModes`, with no engine
  dependency in core and strict rejection of unknown bits at protocol boundaries.
- [x] Add renderer-neutral `TerminalTextRange` normalization/extraction in core, returning the shared
  clipboard value and containing no desktop gesture, ANSI, or clipboard backend behavior.
- [x] Expose the minimum renderer-neutral immutable presented-slice identity from `ViewportCache` so a
  history selection can survive monotonic append only when the exact cached rows remain visible.
- [x] Add unit tests for empty/exact-cap/over-cap/NUL/Unicode redaction, keyboard flag validation, and
  stable-versus-invalid history slice identities.

## Step 2: Terminal ingress and Alacritty keyboard state

- [x] Add direct `base64` use to `zterm-terminal`; keep `Osc52::Disabled` but enable Alacritty's Kitty
  keyboard state machine.
- [x] Split OSC 52 into a dedicated streaming state only after exact command recognition. Preserve the
  generic 1,024-byte string cap, bound canonical Base64 to 699,052 bytes, consume overflow through the
  terminator, and never leak residue to the grid.
- [x] Accept only non-empty `c` writes with standard padded canonical Base64, valid UTF-8, no NUL, and
  decoded size <= 524,288. Reject read/other selector/empty/malformed/overflow atomically and retain at
  most the latest valid effect per ingest.
- [x] Whitelist only valid Kitty keyboard set/push/pop/query CSI-u controls. Feed state changes to the
  sole Alacritty engine, answer query with a bounded PTY reply, and continue rejecting unrelated CSI-u.
  Do not independently track or cap the engine's keyboard-stack depth.
- [x] Project all five keyboard flags through checkpoint/snapshot/delta and bump the checkpoint format
  only if its semantic layout contract requires it.
- [x] Cover whole/chunked, BEL/ST/C1/cancel, exact/over bounds, trailing-bit/padding, mixed valid/invalid
  burst, no-render/no-reply, and ordinary keyboard stack/screen/reset/query behavior in terminal
  unit/security/corpus tests.

## Step 3: Protocol direct cutover

- [x] Add protobuf `TerminalClipboardWrite`, message kind 322, and `TerminalModes.keyboard_flags`; leave
  retired kinds 319-321 absent and add no capability negotiation.
- [x] Register kind/decode/encode mappings exactly once in `zterm-proto`. Convert clipboard text through
  the core validator and keep diagnostics content-free.
- [x] Update compatibility/registry/surface fixtures for the new field and kind. Test missing/invalid
  IDs, empty/NUL/over-cap text, unknown keyboard bits, exact maximum frame, request_id zero convention,
  and the existing 1 MiB gate.

## Step 4: Session-scoped transient effect routing

- [x] Add one mutex-protected controller target plus one replaceable pending host effect and a
  payload-free `watch<()>` wakeup beside `SharedTerminal`; publish effects after releasing the model
  lock and never block PTY drain.
- [x] Give the Session actor one centralized reconciliation helper for effect eligibility. Cover first
  snapshot activation, later resync, prepared/committed takeover, remote resume, detach, principal
  removal, lease loss, and Session end; every target change clears stale pending content.
- [x] Extend `SessionAttachment`/`attachment_writer` to take only effects targeted to its own ID and emit
  kind 322 with the existing absolute write deadline. Never deliver to observers or final/reconnect
  update paths.
- [x] Add deterministic tests that linearize publication against activation/takeover/disconnect; prove
  no-controller drop, observer exclusion, no replay, latest-wins burst, one in-flight plus one pending,
  stalled-writer PTY progress, and redacted Debug/error behavior.

## Step 5: Remote bridge, same-UID IPC, and CLI event delivery

- [x] Decode and validate kind 322 in `remote_attachment` only for the exact active or previously-active
  synchronizing controller epoch; rewrite only the attachment ID and do not retain it in reconnect
  state.
- [x] Decode/revalidate it in `LocalAttachmentClient` and expose a redacted `LocalAttachmentEvent` and
  `TerminalViewEvent` domain value.
- [x] Bypass the capacity-eight ordinary operations event queue with a second latest-only transient
  slot. Make `TerminalViewEventReader` multiplex normal events and clipboard wakeups without retaining
  payload in `watch` or allowing terminal lifecycle outcomes to be masked.
- [x] Test local/remote ID mismatch, wrong epoch/phase, same-controller resync, takeover, stream loss,
  multiple writes, slow/no reader, lifecycle precedence, and absence of content from every formatted
  diagnostic.

## Step 6: Host keyboard gateway and desktop sink

- [x] Move/extend the sole host input codec into a focused module if that reduces the current
  `terminal_ui.rs` ownership surface. Parse bounded Kitty CSI-u codepoint/alternate/modifier/event/text
  forms while preserving original bytes and existing mouse/Page/paste fragmentation behavior.
- [x] Add one terminal-guard Kitty stack entry: push disabled on entry, update only its top value from
  the presenter, and pop on normal/error/signal/cancellation/panic exits.
- [x] Derive outer flags from child flags plus finalized-selection state. Use flags 7
  (`DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS`) for the temporary
  zero-child-mode local-selection elevation, so physical event phase and layout identity stay
  explicit. Preserve raw bytes when modes agree; implement the complete standard legacy downgrade,
  including Ctrl/Alt/Escape, cursor/application-cursor, keypad, Page, function, repeat, and release.
- [x] Recognize Ctrl/Super+C only with a finalized local selection. Consume one press and its matching
  repeat/release lease; otherwise clear selection and forward through the existing prefix/history/live
  path. Preserve Ctrl+] detach and bracketed paste under enhancement.
- [x] Add direct `base64` use to `zterm-cli` and one `DesktopPresenter::write_clipboard` operation that
  emits canonical `ESC ]52;c;<padded base64>BEL` with one write/flush, content-free errors, and no frame
  baseline mutation. Use it for both remote effects and local copy actions; add no backend/config.
- [x] Test fragmented/malformed CSI-u, macOS Super+C, Linux Ctrl+Shift+C, legacy Ctrl+C, modifiers,
  leases, mode equality/downgrade, raw unknowns, outer stack restoration, exact OSC bytes, output
  failure, and baseline preservation.

## Step 7: Selection, extraction, and one pointer router

- [x] Add a CLI selection controller with Idle/Dragging/Finalized/CancelledUntilRelease, attachment-
  local anchor/focus, exact successfully-presented source identity, and no daemon/model state; reuse the
  core range/extractor rather than duplicating mobile-neutral text rules.
- [x] Replace scattered mouse conditionals with one exhaustive typed owner result. Preserve active
  gutter capture first, then selection capture, gutter hit, history, child mouse, alternate-scroll,
  live history, and eligible unmodified left selection; execute exactly one outcome per event.
- [x] Invalidate selection on incompatible snapshot/delta, resize/reflow, screen/mouse-owner change,
  reconnect, gap/rebase/navigation, normal non-copy input, or new click. Swallow the remainder of a
  cancelled drag and never synthesize/forward an orphan child release.
- [x] Extract reading-order text from the currently presented semantic rows: wide head once,
  continuation skipped, combining preserved, selected blanks as spaces, wrapped join, non-wrapped
  newline, and incremental atomic 512 KiB enforcement.
- [x] Apply inverse-style XOR only to cloned selected cells in `ComposedFrame` before gutter/status and
  final cursor. Reuse the existing presentation pacer and one DEC 2026 transaction; do not create a
  second draw loop or mutate surface/cache rows.
- [x] Test forward/reverse/pure-click/multiline/wide/combining/blank/wrap/cap cases; live/history source
  changes; background append with stable cached slice; mid-drag invalidation; gutter/status bounds;
  every main/alternate/live/history/mouse-mode route; and highlight/chrome/cursor flicker regressions.

## Step 8: Cross-layer acceptance and knowledge capture

- [x] Extend deterministic terminal/daemon and CLI PTY fixtures across the real boundaries: parser
  corpus covers chunked OSC 52 and Kitty mode controls, a real child PTY proves structured controller
  routing, and the outer-PTY CLI fixture proves a local drag plus enhanced copy press/repeat/release
  emits exactly one canonical OSC 52 without reaching the child or changing the presenter baseline,
  then restores its owned keyboard stack entry.
- [ ] Run the Herdr black-box fixture through generic mouse/keyboard/OSC paths. Verify its scrollbar,
  selection, wheel routing, and copy effect without matching its name in product code.
  The existing Herdr 0.8.2 alternate-screen/resize/detach/resync/cleanup fixture passes; its current
  assertions do not constitute desktop selection or clipboard evidence. Its direct `TerminalDriver`
  harness intentionally has no Session controller/effect sink and cannot observe normalized OSC 52;
  a raw-output recorder proxy is not admissible because the extra PTY changes the child's resize from
  the asserted 47x123 geometry. Keep this acceptance item manual rather than adding timing, theme,
  screen-text, application-name, or duplicate parser coupling.
- [ ] Manually validate macOS Ghostty: live/history drag, reverse drag, Cmd+C into an external app,
  shell Ctrl+C with no selection, nested Herdr copy, reconnect/takeover, and terminal cleanup. Record
  the real environment; do not replace it with compile-only evidence.
- [ ] Validate Linux Ctrl+Shift+C with a Kitty-keyboard-capable outer terminal when available; retain a
  deterministic byte-level CI fixture for headless runners.
- [x] Update `terminal-model.md`, `terminal-driver.md`, `session-service.md`,
  `local-daemon-ipc.md`, `core-wire-domain.md`, and the relevant cross-layer/root-cause guide with only
  the implemented executable contracts. Remove the old “OSC 52 always rejected / Kitty disabled” text.
- [x] Run focused and broad gates and inspect diffs/log formatting for clipboard leakage.
- [x] Dispatch the independent `trellis-check` reviewer with `check.jsonl`; fix concrete findings and
  rerun affected plus final broad gates. The review fixed cross-epoch clipboard retention, premature
  attachment eligibility, stale selection/presenter identity, checkpoint-version drift, raw unknown-key
  forwarding, and protocol coverage gaps. Its identified Alacritty keyboard-stack depth edge is left
  to the pinned engine by explicit scope decision; no Zterm depth tracker or dedicated defense remains.
- [x] Correct the final architecture finding that hidden deltas behind pinned history could advance
  child input modes without advancing physical outer encoding. The sole presenter now projects
  application cursor/keypad, bracketed paste, focus reporting, and derived Kitty flags into one
  changed-controls transaction and excludes routed mouse/alternate-scroll state. Before either the
  hidden mode-only path or live full-frame path writes, it stages the candidate surface, compact
  viewport/cache-anchor metadata, and a reconciled Copy-sized selection candidate. Only successful
  output commits all candidates; write/flush failure preserves every pre-delta semantic value while
  invalidating the presenter baseline. No history row buffer is cloned for this preview and no
  post-commit second sync exists. It preserves flags-7 finalized-selection elevation and treats all
  other keyboard mismatches as raw rather than speculative legacy input. Application-neutral tests
  require exact controls for all five categories, unchanged/routed-only no-I/O, epoch/extent selection
  invalidation in the sole transaction, live and hidden failure atomicity, no visual repaint, accurate
  committed state, and full-frame recovery.

## Focused validation commands

```sh
cargo test -p zterm-core terminal
cargo test -p zterm-core viewport_cache
cargo test -p zterm-terminal
cargo test -p zterm-proto
cargo test -p zterm-daemon terminal_driver
cargo test -p zterm-daemon session
cargo test -p zterm-daemon session_wire
cargo test -p zterm-daemon remote_attachment
cargo test -p zterm-daemon local_ipc
cargo test -p zterm-daemon operations
cargo test -p zterm-cli terminal_ui
sh tests/foundation/terminal-blackbox.sh --mode herdr
```

## Broad quality gates

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo doc --workspace --no-deps
sh tests/source-policy.sh
sh tests/terminal-dependency-policy.sh
sh tests/secret-scan.sh
git diff --check
```

## Risky boundaries and rollback points

- `crates/terminal/src/ingress.rs`: parser framing and secret-size bounds. Land corpus/security tests
  with the new state before routing effects anywhere.
- `crates/daemon/src/terminal_driver.rs` and `session.rs`: model-thread/controller linearization. The
  broker must remain transient and PTY-independent; revert this slice rather than queueing effects in
  revisions.
- `proto/zterm/v2/*.proto`, `crates/proto/src/lib.rs`, and remote/local adapters: kind 322 is a direct
  cutover. Never reuse retired kinds or add a silent fallback.
- `crates/cli/src/terminal_ui.rs` and new input/selection modules: raw input fidelity and terminal
  restoration are release blockers. Do not ship Cmd+C by leaving global keyboard enhancement on.
- `composition.rs`/`ansi_presenter.rs`: only the composed copy may be highlighted, and clipboard output
  must not advance a visual baseline.

## Stop conditions

- Stop before implementation until fresh approval of this converged plan.
- During implementation, stop and report if reliable Cmd+C would require a terminal-brand branch, if
  effect target ordering cannot be proven with one broker lock, or if a selection path needs a second
  parser/shared model mutation.
- Do not expand into copy mode, search, auto-scroll, rich clipboard, policy UI, Android UI, or backward
  compatibility while closing this task.
