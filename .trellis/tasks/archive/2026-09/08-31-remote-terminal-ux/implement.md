# Remote terminal resilience and UX implementation plan

## Execution model

- Use one native Codex/Trellis implementation worker for the coherent slice, followed by one independent native Codex/Trellis checker.
- Do not invoke Trellis channel, Claude Code, DeepSeek, or any provider billed outside the current Codex session without a new explicit user approval.
- Keep implementation sequential because `terminal_ui.rs`, attachment events, and protocol state overlap; parallel workers would create ownership conflicts rather than reduce time.
- Run focused gates while editing, one broad workspace gate before independent review, then rerun only affected gates plus one final broad gate after concrete fixes.

## Step 0: Start and contract baseline

- [x] After the user approves the final planning summary, run `task.py start` and load the curated implementation context.
- [x] Record the clean/dirty worktree boundary and preserve all unrelated user changes.
- [x] Confirm current `vt100 0.16.2`, Iroh selected-path RTT API, protocol capability negotiation, frame limits, Session/controller ownership, and terminal restoration tests before editing.
- [x] Keep the PRD's three deliverables mapped to named tests; do not add copy/search/history persistence or configurable chrome.

## Step 1: Core history and terminal-mode projection

- [x] Add zterm-owned bounded history cursor/page/outcome types with redacted Debug and no public `vt100` types.
- [x] Add the minimum history epoch state derived from checked revisions; preserve epochs for safe monotonic append below capacity and invalidate conservatively on resize/clear/capacity ambiguity.
- [x] Render bounded formatted row pages from a cloned screen view without mutating the authoritative model's scrollback offset.
- [x] Recognize DECSET/DECRST 1007 at the existing safe callback boundary and round-trip `alternate_scroll` through terminal state, snapshot, delta, and checkpoint.
- [x] Add pure tests for page order/style/Unicode, cursor range, append, eviction, clear, resize, alternate screen, and fixed bounds.

## Step 2: Protocol and host Session history path

- [x] Add additive history request/page messages and explicit ok/changed/gap outcome; register exact wire mappings and activate `HISTORY_PAGING` only with complete support.
- [x] Validate attachment identity, requested bounds, capability, main-screen eligibility, and cursor epoch once at the Session/terminal owner.
- [x] Extend retained attachment/Session actor commands to return a read-only page without altering controller lease, PTY lifetime, checkpoint, or live revision delivery.
- [x] Add protobuf compatibility, malformed/oversize, unsupported capability, and local/remote Session routing tests.

## Step 3: Remote bridge and selected-path status

- [x] Bind one redacted exact-candidate observer to every opened epoch for capability, selected path kind, and RTT; do not expose or persist addresses, relay URLs, candidates, or IDs.
- [x] Resolve and freeze the validated local Device alias next to the exact DeviceId without using it for retry/routing.
- [x] Add one complete local-only connection-status event and validate it in same-UID IPC/operations projection.
- [x] Emit unknown initially, current status on activation, changed samples at no more than 1 Hz, and clear display semantics through reconnect/path migration.
- [x] Forward at most one correlated history request through the existing bridge pending-control budget and never replay it across reconnect epochs.
- [x] Test direct/relay/unknown, RTT rounding/clamping, migration, frozen alias, correlation, reconnect, exact-candidate replacement, and redaction.

## Step 4: Closure-race classification

- [x] Preserve OS error kind long enough to distinguish command-side attachment closure from ordinary command failure.
- [x] Add one bounded terminal-driver correlation drain for EPIPE/reset-equivalent closure, prioritizing buffered typed lifecycle/service events.
- [x] Normalize unmatched closure after the bound; do not include the raw OS `Broken pipe` text, retry resize/input, or create another reconnect owner.
- [x] Add deterministic schedule tests for resize versus SessionEnded, LeaseLost, remote reconnect/resync, buffered typed error, and plain EOF/daemon stop.
- [x] Correlate a top-level `TerminalViewCommandWriter` close with the already
  queued terminal event before the CLI reports `terminal attachment driver
  closed`; the lower local-stream correlation does not cover a ready SIGWINCH
  winning after the driver task has delivered its final event and exited.

## Step 5: CLI geometry, viewport, and chrome

- [x] Split physical versus child viewport sizing before initial prepare and on every resize; reserve one row only for remote views with the one-row fallback, and clamp the child projection to the shared Session viewport maximum while retaining uncapped physical geometry for status placement.
- [x] Introduce one exhaustive live/history/resume-pending viewport controller and one-outstanding-page prefetch state.
- [x] Add the bounded SGR mouse/Page gesture codec and mode-derived routing; forward unknown keyboard bytes unchanged and avoid program-name branches.
- [x] While pinned, drain live events without applying their ANSI; on return request the existing full snapshot, restore/ack it, then forward bounded retained key/paste bytes exactly once after Active.
- [x] Render explicit history changed/gap notice inside the history viewport, never as a fourth status field.
- [x] Add `StatusRenderer` for display-width-safe `<device> | <mode> | <latency>` text, complete-row reverse background, cursor/style preservation, old-row cleanup, and redraw after child output/status/resize.
- [x] Extend terminal guard enter/restore for UI-owned mouse modes and prove every signal/error/panic path restores them.

## Step 6: Integration and black-box coverage

- [x] Extend local IPC/remote bridge/operations fixtures for the new event and history commands without content-bearing diagnostics.
- [x] Extend the production multiprocess PTY fixture for rapid resize, narrow/one-row geometry, Unicode alias, status SGR isolation, and restoration; keep scroll/Page/paste/mode semantics in deterministic owner tests where the local fixture cannot observe remote history without product seams.
- [x] Add a failing-before pure regression for oversized initial/resize physical geometry; keep Session/wire rejection strict for independently supplied oversized viewports.
- [x] Add the exact CLI scheduling regression where a legal-size SIGWINCH and a
  ready authoritative terminal event race; assert the event's typed outcome
  wins instead of the command-channel fallback.
- [x] Run nested tmux 3.7c and Herdr 0.8.2 through the generic mode path; verify alternate screen and child mouse ownership receive their input.
- [x] Manually validate Ghostty: long shell output scrolls, rapid resize stays attached, status is visually distinct, direct/relay and RTT update, and Ctrl+] detach remains unchanged.
  - 2026-09-01: the user accepted the published v0.1.9 build in macOS Ghostty after the final rapid-resize regression retest and reported no remaining issue.

## Step 7: Specs, review, and completion

- [x] Update the owning backend specs only with executable contracts learned or changed by implementation: terminal model, Session service, local IPC, transport auth, and core wire domain.
- [x] Run the focused and broad validation commands below and `git diff --check`; inspect source/status output for terminal content, Device ID, IP, relay URL, ticket, and raw Broken pipe leakage.
- [x] Dispatch one independent native Trellis checker with `check.jsonl`; fix concrete findings only.
- [x] Re-run affected focused gates and one final broad gate, then present code/test results for commit approval.

## Bug analysis: oversized physical TTY closed the attachment

### 1. Root cause category

- **B/D/E — cross-layer contract, coverage gap, implicit assumption**: the CLI
  reserved the status row but assumed every physical TTY size was a valid
  Session viewport. The strict Session boundary correctly rejected `300×100`
  against its `240×80` limit.

### 2. Why the earlier fix did not cover it

- The first investigation followed the reported `Broken pipe` and fixed the
  closure-result race. Its resize tests used only admitted sizes, so they could
  not expose the separate oversized resize request. The resulting deterministic
  `300×100` reproduction does not explain the user's intermittent legal-size
  `140×40` closure; the shared fallback message is not a causal diagnosis.

### 3. Prevention mechanisms

| Priority | Mechanism | Action | Status |
|---|---|---|---|
| P0 | Architecture | Use one physical-to-child projection for initial and every resize path | done |
| P0 | Regression | Prove `300×100` projects to `240×80` while direct Session requests remain strict | done |
| P0 | Live evidence | Resize a real remote attachment to `300×100`, assert it stays live, and observe remote `stty size` | done |

### 4. Systematic expansion

- All four CLI producers—initial, post-prepare, active resize, and inactive
  resize—already converge on the same helper; no second clamp or retry owner is
  needed. Status rendering continues to consume the uncapped physical size.
- Future resize investigations must reproduce the exact failing dimensions and
  compare them with service admission limits before attributing a close to a
  timing race.

### 5. Knowledge capture

- [x] Update the PRD/design acceptance contract.
- [x] Update `backend/local-daemon-ipc.md` with the executable projection and
  strict-boundary rule.
- [x] No template sync is applicable because this repository has no
  `src/templates/markdown/spec/` mirror.

## Bug analysis: writer closure masked an already queued outcome

### 1. Root cause category

- **B/D — cross-layer contract and coverage gap**: lower same-UID socket
  closure correlation was correct, but the later driver-to-CLI command/event
  boundary could close its mpsc/oneshot owner after delivering the terminal
  event. A simultaneous SIGWINCH then exposed the generic command fallback
  before the CLI consumed that event.

### 2. Why the earlier fix did not cover it

- Tests stopped at `LocalAttachmentClient -> run_terminal_driver`; they did not
  schedule `TerminalViewCommandWriter -> CLI event reader` after the driver had
  sent its final event and exited. The shared error text hid that these were two
  different closure boundaries.

### 3. Prevention mechanisms

| Priority | Mechanism | Action | Status |
|---|---|---|---|
| P0 | Architecture | Set one latest-only latch only after the final event enters the bounded event queue | done |
| P0 | Regression | Submit legal `31×97` resize after command-owner close, then require original typed event | done |
| P0 | Negative | Drop the event reader and require normalized failure rather than false success | done |

### 4. Systematic expansion

- Command-send and oneshot-response closure use the same bounded correlation;
  resize, input, snapshot acknowledgement, sync, and history therefore cannot
  drift into separate fixes. Commands are suppressed only when an existing
  terminal outcome is already consumable; they are never replayed.

### 5. Knowledge capture

- [x] Update PRD/design with the two distinct race boundaries.
- [x] Update `backend/local-daemon-ipc.md` with latch ordering, fallback, and
  positive/negative test assertions.
- [x] No template sync is applicable because this repository has no
  `src/templates/markdown/spec/` mirror.

## Bug analysis: rapid resize crossed the snapshot barrier

### 1. Root cause category

- **B/D/E — cross-layer contract, coverage gap, implicit assumption**: a
  successful `TerminalViewCommandWriter::resize` future proves that the frame
  entered the local duplex stream, not that Session has completed the resulting
  snapshot cycle. The CLI could still consider the view `Active` and submit a
  later `SIGWINCH` while Session correctly rejected controller operations in
  `Awaiting` state.

### 2. Why earlier fixes did not cover it

1. Closure correlation preserved typed terminal outcomes but did not serialize
   ordinary successful resize commands against snapshot synchronization.
2. Physical viewport clamping fixed deterministic oversized rejection but the
   reported legal `140x40` case remained a scheduling race.
3. The first local fence prevented the second changed resize, but an identical
   coalesced signal could submit a semantic no-op, enter `Synchronizing`, and
   wait forever because no size change meant no replacement snapshot/`Active`
   barrier.
4. Even after no-op suppression, a newly received `Active` could become stale
   before the next client frame arrived: terminal output and client commands
   travel independently on the duplex stream, so Session could already be
   awaiting a newer snapshot.

### 3. Prevention mechanisms

| Priority | Mechanism | Action | Status |
|---|---|---|---|
| P0 | Architecture | Immediately fence after a changed resize and coalesce at the sole CLI owner | done |
| P0 | Session boundary | Admit replaceable resize only for the exact current controller during an Active-target snapshot; keep input/history/takeover strict | done |
| P0 | No-op rule | Track the last submitted viewport and suppress identical active or pending observations | done |
| P0 | Regression | Deterministically prove pending/deduplicated state transitions and repeatedly exercise rapid real PTY `SIGWINCH` delivery | done |
| P1 | Documentation | Record queue-write versus authoritative-state semantics in the reusable cross-layer guide and owning IPC spec | done |

### 4. Systematic expansion

- **Similar issues**: every byte-bearing one-way terminal command must be gated
  by the authoritative transport state, not by completion of its local write
  future. Replaceable state needs an explicit, separately reviewed admission
  rule when visual synchronization and commands are full-duplex.
- **Design improvement**: effective state plus pending resize are decided by
  one coalescer transition; callers cannot independently reopen input while a
  changed viewport is waiting.
- **Process improvement**: when a state-changing command can be a no-op, tests
  must cover both changed and identical values; changed-value tests alone can
  falsely imply that a completion event is guaranteed.

### 5. Knowledge capture

- [x] Update `backend/local-daemon-ipc.md` with the fence, coalescing, no-op,
  and resize-specific admission rules.
- [x] Update `guides/cross-layer-thinking-guide.md` with the reusable
  queue-write versus authoritative-state checklist.
- [x] Keep the pure owner test plus production multiprocess PTY regression.
- [x] No template sync is applicable because this repository has no
  `src/templates/markdown/spec/` mirror.

## Validation commands

Focused while implementing:

```sh
cargo test -p zterm-core
cargo test -p zterm-proto
cargo test -p zterm-daemon session
cargo test -p zterm-daemon remote_attachment
cargo test -p zterm-daemon local_ipc
cargo test -p zterm-cli terminal_ui
sh tests/foundation/terminal-blackbox.sh --mode tmux
sh tests/foundation/terminal-blackbox.sh --mode herdr
```

Broad gates before review and at final handoff:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo doc --workspace --no-deps
sh tests/source-policy.sh
sh tests/secret-scan.sh
git diff --check
```

## Risky files and rollback points

- `crates/core/src/terminal.rs`: history epoch/page and mode semantics. Land and test before any protocol consumer.
- `proto/zterm/v1/terminal.proto`, `proto/zterm/v1/wire.proto`, `crates/proto/src/lib.rs`: additive compatibility boundary. Do not reuse an existing kind or change old field meaning.
- `crates/daemon/src/session.rs`, `session_wire.rs`, `remote_attachment.rs`: controller/reconnect ownership. History remains read-only and cannot enter PTY close/replay paths.
- `crates/daemon/src/connection_broker.rs`, `remote_session.rs`, `local_ipc.rs`, `operations.rs`: redacted status and closure correlation. Revert status independently to unknown if observation fails; do not roll back terminal safety.
- `crates/cli/src/terminal_ui.rs`: raw TTY restoration and input exactness. A restoration or paste regression blocks the feature rather than shipping a partial mode.

## Stop conditions

- Stop when every PRD acceptance criterion has one passing owner; do not add search/copy/configuration or further status fields as opportunistic hardening.
- At four active hours, report completed scope, remaining scope, and the largest risk. At eight active hours, stop for explicit approval before continuing.
- A harness-only flake gets at most two materially different fixes or 90 minutes; then simplify or defer the exact harness without adding production seams.
- Stop immediately on any unexpected external-provider invocation, authentication, quota, or billing signal.
