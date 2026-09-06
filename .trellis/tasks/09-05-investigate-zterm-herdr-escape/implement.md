# Implementation plan: shared prefix command mode

## Entry gate

**Main agent only; no sub-agents for implementation, checks or review.**

- [x] Generic and Herdr baseline evidence retained.
- [x] Latest scope recorded: Ctrl+] enters a shared command mode/dispatcher;
  period is the first binding. Remove --escape; preserve normal other shortcuts.
- [x] PRD/design/plan updated; no new command bindings or history-key migration.
- [x] Pin Ghostty 1.3.1 source, identify Doubao 0.9.7 and prepare input probe.
- [x] Obtain actual Ghostty/Doubao physical-key output: U+3002 in flags 0/7,
  key 46 in flags 15, prefix received in all cases, no timeout in the six chords.
- [x] User selected local cancellation for unknown commands and timeout; consume
  the attempted prefix command, with no implicit replay to the child.
- [x] User declined input-method adaptation after reviewing drawbacks. Remove
  temporary reporting, negotiation and further IME probes from the plan.
- [x] Present the reduced proposal for final review: shared prefix dispatcher,
  existing delivered-key decoding, local cancellation and --escape removal;
  existing keyboard-reporting/clipboard policy remains intact.
- [x] After explicit review approval, run task.py start and load applicable
  phase detail plus trellis-before-dev directly.

## 1. Establish the regression

- [x] Check current working changes and relevant specs.
- [x] Add a failing production codec/prefix-boundary case for enhanced Ctrl+]
  plus plain/encoded period, observing detach and emitted child input.
- [x] Confirm missing detach is the failure, not startup/timing.

## 2. Establish one prefix command owner

- [x] Reuse decoded identity for fixed Ctrl+] semantic matching; legacy 0x1d
  uses the same owner. Remove configurable/disabled prefix branches.
- [x] Introduce Normal/AwaitCommand transitions, a semantic command-key lookup
  and LocalCommand output. Only period -> Detach is registered. The protocol
  decoder/prefix recognition contains no detach-specific suffix branch.
- [x] Retain original forwarding representation/context on unconsumed events;
  store only a deadline, bounded owned identities and UTF-8 continuation count.
  Local cancellation needs no buffered prefix payload.
- [x] Implement release, repeat, explicit quoting, deadline and local cancellation
  contracts. The observed pure Control release must not cancel AwaitCommand;
  repeats neither quote the prefix nor extend the deadline.
- [x] Replace the byte-only prefix decision; avoid parallel protocol parsers or
  post-encoding prefix scanning.
- [x] Introduce no new command bindings or configurable registration framework;
  future commands extend the static binding/execution boundary only.
- [x] Remove --escape from all three CLI entry points, EscapePrefix/parser,
  TerminalRequest field and dispatch/debug/default plumbing. Add no hidden
  compatibility path or alternative setting.

## 3. Integrate existing callers

- [x] Wire the active loop and inactive attach waits to the same prefix owner.
- [x] Preserve current selected-copy priority/lease, paste bypass, direct
  page/history keys, pointer routing and selection-only mode handling.
- [x] Capture encoding context before selection mutation; route fallthrough
  through existing history/resume/Active gates with correct ordering.
- [x] Cover timeout, EOF, snapshot/epoch reset, disconnect and cleanup.
- [x] Preserve other CLI parsing, daemon/transport and the existing child/
  selection-driven keyboard-reporting policy. Add no command-mode reporting
  state, request, query or presenter override.
- [x] Interpret key identity already received; do not infer physical period from
  U+3002 or trigger commands from a release alone.
- [x] Define complete text/frame ownership so unknown-input cancellation cannot
  leak a UTF-8 continuation or the tail of a CSI-u frame to the child. Local
  cancellation does not justify dropping unrelated ordinary input afterward.
- [x] Remove --escape from README/remote CLI usage and explanations; document
  fixed Ctrl+] command mode, detach/quote/deadline and local cancellation.

If evidence requires a larger behavior change, reconverge the plan rather than
silently widening the agreed task.

## 4. Verification

- [x] Focused cases: legacy/enhanced/mixed/fragmented input, encoded period,
  fixed-prefix identity/modifiers/layout aliases, press/release, repeat,
  double prefix, no replay on timeout/unknown commands and stale-state cleanup.
- [x] Keep copy/elevation, paste, existing page/history and pointer regressions.
- [x] Cover reported primary/base-layout keys independently of committed IME
  text. Use the retained real trace for general modifier/press/release cases.
  Verify that U+3002 cancels locally and command state adds no keyboard-mode
  request. No new physical IME probe or terminal/IME identity branch.
- [x] Verify --escape is rejected by connect/session new/session attach before
  execution and absent from help; retire obsolete custom/disabled-parser tests.
- [x] Add one generic enhanced-child outer-PTY case for successful detach,
  terminal restoration and Session survival/reattachment.
- [x] Run relevant checks:

    cargo +1.98.0 test -p zterm-cli --lib --all-features --locked
    cargo +1.98.0 test -p zterm-cli --test daemon_autospawn --all-features --locked
    cargo +1.98.0 fmt --all -- --check
    cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings

- [x] Complete any remaining required phase/repository gate before commit or
  pre-push (just check is the repository pre-push owner); do not repeat passing
  checks without relevant changes/failures.
- Optional Herdr post-fix smoke skipped: the required generic outer-PTY
  regression is the acceptance owner. Preserve the original failing baseline;
  its runner asserts broken behavior and is not post-fix acceptance.

## 5. Direct review and completion

- [x] Apply trellis-check directly, with no sub-agent dispatch.
- [x] Review one command-mode owner and binding lookup, unchanged normal input,
  input fidelity, bounded lifecycle state and absence of application heuristics
  or new bindings. Confirm future commands do not require decoder edits.
- [x] Use trellis-update-spec for the executable prefix contract/regression owner.
- [x] Record actual commands/results and platform/evidence limitations; mark
  acceptance only on observed evidence.
- [x] User authorized the reviewed commit batch and subsequent release flow
  with “走发布流程吧”. Follow work commit -> task archive -> journal, then the
  repository release operator for v0.1.20. Do not mutate existing user sessions.

Rollback the prefix change and tests as a unit; retain research. No schema/data
rollback is involved.

## Execution status

User approval: “好，开始执行吧”; task started and all implementation/check work
was performed directly without sub-agents. The full local `just check` passed.
Final focused and full checks passed with the last lifecycle assertions; see [verification](./verification.md)
for actual results and remaining commit/finish bookkeeping.
