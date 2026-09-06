# Implementation plan

## Delivery and execution model

One cohesive implementation target, C05–C18/R01–R10. Stages below are dependency
order and review boundaries, not cuts to the user-requested scope. No theme picker,
presets or persistent client preference work (P1–P7/C19).

User approved implementation on 2026-09-06 and explicitly requested no subagents.
Implement and review inline in the primary agent. Preserve existing edits; do not
launch workers or external provider CLIs. Task started; before-dev specs loaded.

## Ordered checklist

### 0. Ready/start

- [x] User scope, protocol matrix and compatibility defect recorded.
- [x] Research pinned engine and current Session/driver/CLI paths.
- [x] Converge PRD; design and ordered implementation plan exist.
- [x] Decide one task with internal stages, not independently shippable fragments.
- [x] Validate populated implement/check manifests and read converged PRD once.
- [x] Present latest final planning summary and obtain subsequent approval per
  the invoked brainstorm workflow; only then task.py start.
- [x] Load trellis-before-dev and exact Phase2 instructions. Recheck git status,
  task state, applicable spec indexes and any concurrent edits before product code.

### 1. Shared semantic contract and color owner (R01/R03/R05/R09)

- [x] Add bounded base/effective values, appearance, source-aware Unknown fallback,
  explicit cursor rendering provenance, underline enum/color to zterm-core.
- [x] Extend surface/delta/history validation and transactional candidates, using
  existing Revision for changed_at. Preserve semantic cells and valid selection.
- [x] Add one engine-owned ZtermColorState and typed mutations; Alacritty palette
  callbacks/setters remain intercepted. Keep pinned Alacritty/vte dependency.
- [x] Add model base update, revision preflight and palette-only projection/delta.
  Extend checkpoints and increment internal format from 2 to 3; no persistence migration.
- [x] Define reset/screen lifetime and mapped screen5 semantics exactly as protocol
  research. Current pen underline comes from upstream, not a second SGR parser.
- [x] Test initial/unknown/RGB/dynamic/inherit distinctions, mode5 mapping,
  palette-only revisions and base-under-override/reset before proceeding.

### 2. Child protocol completeness and mixed-SGR fix (R01/R02/R04/R06–R09)

- [x] Admit exact OSC4/10/11/12/17/19 setters/queries and104/110/111/112/117/119.
- [x] Implement OSC21 supported keys/dynamic/reset/negative replies, bounded
  RGB/rgbi/X11-name parsing with pinned licensed constants, opaque alpha policy.
- [x] Implement shared10-slot stack, xterm store/restore/report and kitty aliases;
  documented explicit-slot semantics and overflow/underflow no-ops are fixtures.
- [x] Add996/997,2031 subscription/DECRQM and external-change-only notifications.
- [x] Add DECRQSS actual SGR, XTGETTCAP allowlist and truthful negative replies.
- [x] Remove whole-SGR58/59 rejection; project all underline styles/colors. Ensure
  fixed extras do not trigger combining-count caps.
- [x] Use immediate bounded replies at dispatch; no deferred color callbacks.
  Corpus covers set/query/set, multiple pairs, malformed entry, chunk invariance,
  SGR58/59 siblings/RGB channels, reset/stack/mode queries and unrelated policy.

### 3. Wire, driver and Session lifetime (R02/R03/R10)

- [x] Extend protobuf/conversions with required bounded colors and underlines.
  Preserve reserved tags and exact attachment IDs; color deltas/history remain
  semantic, not raw ANSI.
- [x] Canonical create209/attach323/base-update324; retire202/300 with no alias or
  fallback. Extend registry/source-policy fixtures and local/remote dispatch.
- [x] Add one ordered commit gate for ingest/resize/base changes through replies
  and revision publication. Preserve model-lock release before I/O and independent
  child interruption/reaping. Test ordering with deterministic gates, not sleeps.
- [x] Thread initial profile through default_main/create_reserved and interactive
  named create before model/spawn/driver; update create operation fingerprint.
- [x] Add authorized current-controller updates and staged pending-takeover base.
  Commit at lease handoff; replace/ACK any prepared snapshot made stale by colors.
- [x] Carry latest profile through reconnect under fresh attachment identity;
  retain base/overrides on disconnect and reject former-controller updates.
- [x] Test local/remote initial queries, active/pending takeover, startup ordering,
  base-under-override reset, recovery/history and both old/new rejection directions
  before Session/PTY effects. Empty operation lease allocation is permitted.

### 4. Physical observations and input framing (R01/R02/R10)

- [x] Extend existing host codec with typed color/appearance/mode/status replies;
  preserve literal bracketed-paste ownership and unrelated key/mouse semantics.
- [x] One bounded250ms collection,256-slot batches,64KiB observation budget,
  trailing DSR boundary, no overlapping rounds, one pending refresh bit.
- [x] Missing boundary closes acceptance but retains consume-only draining state;
  don't relabel a late reply as a newer generation. Retain last observations on
  partial refresh; no RGB/appearance guesses or timer/brand-based polling.
- [x] Share reply framing across prepare/ACK waits and keyboard epoch transitions.
  Replace blind tcflush with joined-reader decoder-state handoff and bounded drain
  discarding old keyboard units. Retain stale-input and exact-once paste contracts.
- [x] Capture2031 original mode, enable only when known reset, restore only owned
  changes through existing guard cleanup. Route notifications through host state.
- [x] Test fragmented BEL/ST, mixed keys/replies, prefix/selection noninterference,
  stale epochs, missing/no/partial replies, UTF-8/C1, paste literal payload, burst
  coalescing and normal/error/signal restoration at existing test seams.

### 5. Presenter, history, selection and cursor (R03–R05/R09/R10)

- [x] Separate semantic fallback baseline from resolved physical frame. Use mapped
  state exactly once; RGB resolution does not write into grid/history cells.
- [x] Repaint content on palette-only changes, including pinned history/erased
  blanks. Latest changed_at wins; delayed history cannot roll colors back.
- [x] Preserve valid selection on color-only update, resolve roles after inverse.
  Underline explicit color stays explicit; default follows displayed text.
- [x] Paint explicit fixed/Dynamic cursor overrides as steady software block over
  actual glyph. Keep native hidden and CUP accurate; repair wide/combining old/new
  spans, hide in history, reset back to native cursor.
- [x] Keep outer/base chrome independent from child overrides/screen5. Emit no
  physical palette/default/selection/cursor setters or stack mutations.
- [x] Keep one buffered2026 transaction/flush and post-success state commit; failure
  preserves semantic baseline, invalidates physical baseline and repairs next frame.
- [x] Test focused frame bytes and semantic candidate failure, not screenshot-only
  assertions or a second terminal renderer in production CLI.

### 6. Integration, specs and review

- [x] Complete A01–A12 mapping with one authoritative test per distinct boundary.
  Add an application-neutral bounded conformance/swatch fixture and smoke recipe.
- [x] Update terminal-model, terminal-driver, core-wire-domain, session-service,
  local-daemon-ipc and terminal-input-commands specs where contracts changed:
  palette authority; reset/query ordering; protocol retirement; input framing across
  epochs; semantic history colors; software cursor; one presenter; no raw setters.
- [x] Run focused tests during iteration, then native just check once after all
  layers integrate. Fix actual failures; repeat broad checks only after relevant
  changes or new concerns.
- [ ] Run physical-terminal light/dark smoke where available: query/swatches,
  same-class palette update, set/reset/stack, detach, local/remote. Herdr/Pi is a
  secondary reproduction, not an app-specific implementation branch.
  Not run in this environment: physical GUI light/dark switching and Herdr/Pi.
  Neutral fixture and manual recipe delivered; isolated PTY fixture passed.
- [x] Inline trellis-check review, self-fix real failures and document exact
  hosted-only evidence. No benchmark, repeated stress loops, unsupported-platform
  revival or production daemon restart for verification.
- [x] Report completed behavior/tests/limitations; follow project spec-update and
  commit/wrap workflow. User subsequently authorized the release workflow.
  Implementation, review, specs and native gate complete; commit approved.
  See execution.md and commit-plan.md for v0.1.22 handoff.

## Validation commands and evidence ownership

Use cargo +1.98.0. Select existing named tests, adding only meaningful cases:

```sh
cargo +1.98.0 test -p zterm-core
cargo +1.98.0 test -p zterm-terminal
cargo +1.98.0 test -p zterm-proto
cargo +1.98.0 test -p zterm-daemon --test controller_lease
cargo +1.98.0 test -p zterm-daemon --test attachment_resync
cargo +1.98.0 test -p zterm-daemon --test local_session_ipc
cargo +1.98.0 test -p zterm-daemon --test terminal_drain
cargo +1.98.0 test -p zterm-daemon --test terminal_recovery
cargo +1.98.0 test -p zterm-daemon --test two_daemon_transport
cargo +1.98.0 test -p zterm-cli --lib
just check
```

Focused commands are per owning change, not a mandatory repeated full list.
`just check` owns source policy, format/lint/tests/docs/dependency and available
relay static checks. Linux/glibc/other supported-host runtime and release evidence
remain their existing CI owners. Tests use temporary effective-user state, never
real ~/.zterm or a user's active remote Session. Execution evidence and the final gate result are tracked in execution.md.

## Risky boundaries and rollback

Highest-risk files: ingress/engine/model/projection; core/proto DTO conversions;
terminal_driver/session actor/wire/client; terminal_ui codec and reader fences;
composition/history/presenter. Edit in staged dependency order and keep mixed-version
requests explicitly rejected. Scope is in-memory; no persisted color data migration.
Revert the cohesive implementation if needed, not partial wire/presenter modules.
The physical terminal is never color-mutated, so there is no speculative RGB
restoration or automatic rollback machinery. Custom cursor shape/blink, unknown
outer values and missing response-boundary limitations remain documented limits.
