# Desktop connection progress and startup escape leakage

## Goal

Explain and fix the desktop CLI connection screen displaying unreadable terminal
control-sequence fragments. A temporary empty screen is acceptable, but it should
show readable connection stages or connection logs while the session is loading.

## Background

- User explicitly requested a new Trellis investigation task on 2026-09-08.
- The supplied macOS terminal screenshot shows a mostly empty terminal containing
  fragments such as `185;?;186;?;187;?;188;?;189;?;190;?;191;?` at the top.
- The request concerns the desktop connection experience, not Android UI.
- Related terminal presentation and reconnect work must be checked before deciding
  whether this is a local defect or a boundary/ownership problem.
- The user subsequently confirmed Ghostty (typed “Ghostly”), direct local CLI
  execution, and apparently equivalent symptoms for local and remote targets.
- Baseline `bd4e320`: raw/alternate-screen entry precedes color observation and
  asynchronous prepare; the first semantic presentation occurs only after prepare
  returns (`terminal_ui.rs:209`, `:274`, `:313`, `:359`, under `crates/cli/src/`).
- The screenshot's entire visible fragment exactly matches an outgoing OSC 4
  palette-query tail at zero-based offset 1025 in the query buffer
  (`crates/cli/src/terminal_ui/ansi_presenter.rs:280`). This identifies its byte
  source, but does not establish why the physical terminal rendered it as text.

## Requirements

- R1: Identify the causal path using code evidence and a minimal reproduction
  where practical; distinguish facts from hypotheses.
- R2: Show readable connection progress during desktop startup/connection waits.
  Include the target and current real stage; do not display elapsed time. Keep a small
  stage history if space allows; use existing CLI language/style. Stage changes
  must follow actual operations, not simulated percentages or guessed network
  milestones. Both local and remote target entry use this behavior.
- R3: Keep terminal queries, responses, and connection diagnostics out of visible
  remote session content.
- R4: Transition to the ready terminal without hiding errors or regressing colors,
  input, resize, or reconnect behavior.
- R5: Show a bounded chronological startup journal of actual service, target,
  connection reuse/address lookup/secure-connect/handshake/channel and Session
  stages. Remove the detach-shortcut hint. Reuse must not fabricate new dialing.
- R6: Persist the same typed stages and final startup outcome in existing
  `daemon.log`, with a per-invocation correlation and safe failure category.
  The user explicitly selected simultaneous file logging. No terminal content,
  credentials, addresses or unvalidated target text belongs in these records.

## Acceptance Criteria

- AC1 (R1, R3): Record an evidence-backed diagnosis and regression scenario for the
  observed control-sequence fragments.
- AC2 (R2): While connection/session preparation is pending, display human-readable
  information describing the current stage, including a deliberately slow local
  or remote prepare. Resize preserves a readable current status.
- AC3 (R3, R4): The first session frame takes over cleanly; protocol fragments and
  startup diagnostics are not rendered as session cells.
- AC4 (R4): Connection failure remains understandable and terminal input/modes
  are restored on exit. Existing cancellation and stateful-create outcome rules
  continue to hold, and startup keystrokes are not replayed into the session.

- AC5 (R5): Broker reuse emits no old dialing stages; real address/transport/
  handshake/stream observations survive local frame fragmentation/coalescing.
  The first screen displays a chronological bounded journal with no time or
  shortcut hint and yields to the first valid Session frame.
- AC6 (R6): Actual local attachment records `starting` through `terminal_ready`
  for one CLI PID/invocation. Early failures/cancellation remain typed. Log
  rotation, unsafe/missing paths and observer retirement are covered without
  logging target/payload text, creating setup or changing operation outcomes.

## Out of Scope

- Android connection UI changes.
- Changing the host terminal application's theme or eliminating all connection
  latency.
- Unrelated terminal rendering or transport redesign.
- A persistent log viewer, raw daemon-log streaming, remote Session protocol changes, artificial
  connection delay, and changes to established-session reconnect policy.

## Investigation Status

The empty wait is a desktop presentation lifecycle gap: the existing sole
presenter has no pre-session view. After user-run isolated reproduction and local
log inspection, query leakage is confirmed as a physical-probe compatibility
defect: Ghostty 1.3.1 exhausts its per-OSC response allocator for a 32-index query
and aborts processing the current slice. Single-index OSC commands avoid the
observed failure. See `research/findings.md` for evidence and limitations.

Product scope is resolved. `design.md` and `implement.md` record the proposed
implementation and validation. The user approved the final plan (“按这个方案来”).
The task is in progress on `fix/desktop-connection-progress`.
After reviewing the visible labels, the user requested that concrete waiting
times be removed. This applies to initialization, preparation, synchronization
and cancellation feedback; stage/target information remains.
The latest request expands the coarse progress screen to real cross-layer
stages and explicitly requests persistence in the existing log file. A bounded
same-UID progress sideband and optional in-process observer are in scope.
