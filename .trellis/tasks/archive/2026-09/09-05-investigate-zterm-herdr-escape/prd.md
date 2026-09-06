# Establish a protocol-independent zterm prefix command mode

## Goal

Make fixed Ctrl+] enter zterm's local command mode. The following key goes to
one command dispatcher; currently its only command binding is period -> detach.
Future controls such as device switching extend that binding/command boundary,
without rewriting protocol decoding or adding child-application conditions.
Support the legacy and enhanced key input already delivered by the terminal.
Remove --escape. Detach preserves the host Session and PTY. The input-method
failure is diagnosed; input-method adaptation is excluded from implementation.

## Confirmed scope

- The latest user clarification requires a shared prefix command mode, not a
  special enhanced-keyboard fix for the detach chord. Copy, paste and other
  existing shortcuts retain their current behavior outside prefix commands.
- This supersedes proposals to add prefixed copy/history commands, migrate
  PageUp/PageDown, or remove selection-driven keyboard elevation.
- The user explicitly requested removal of --escape after learning its purpose.
  Remove prefix customization and disabling; Ctrl+] is the fixed prefix. This
  is the only additional CLI behavior change in the otherwise narrowed repair.
- All future zterm-specific controls use Ctrl+] plus a command key. This task
  establishes that dispatcher with only the existing detach binding; it does
  not implement device switching or migrate existing clipboard/history keys.
- The user selected local cancellation: an unknown command or timeout consumes
  the attempted prefix command; it is never automatically replayed to the child.
- After discussing its drawbacks, the user declined the input-method adaptation.
  Do not add prefix-triggered keyboard-mode switching, report-all negotiation or
  punctuation remapping. Keep the current child/selection-driven mode policy.
- Reported input-method environment: Ghostty + Doubao IME. Local installed
  versions are Ghostty 1.3.1 and Doubao 0.9.7. The user's completed physical
  capture shows U+3002 for period in flags 0/7, and the correct period key in
  report-all flags 15. Ctrl+] arrives in every mode. All six attempts are below
  half a second, ruling out the current deadline for the captured failure.
- All implementation, checks and review run directly in the main agent.
  Do not use sub-agents, including Trellis implement/check agents.

## Requirements

- **R1 — Prefix command mode.** Equivalent legacy and supported enhanced prefix
  events enter one AwaitCommand state. Subsequent command keys go through a
  semantic binding lookup, then dispatch a zterm command independently of input
  encoding. Period is one binding, not a special case in the protocol decoder.
  Cover encoded period and mixed encodings without application identity,
  terminal-brand or screen checks.
- **R2 — Fixed prefix.** Remove --escape from all current CLI entry points,
  internal configuration plumbing, help and current user documentation. Preserve
  the one-second timeout and double-Ctrl+] literal quoting. Unknown commands and
  timeout cancel locally and consume the attempted command, including its owned
  lifecycle; no implicit replay. Explicit quoting retains the correct child
  representation under the existing forwarding policy.
- **R3 — Event lifecycle.** Prefix release does not break a command attempt or
  leak an orphan event. Repeat is not a second deliberate prefix press.
  Handle fragmentation/modifiers/alternate-key identity with bounded state.
  Plain modifier events, including the observed left-Control release between
  prefix and period, do not select or cancel a command by themselves.
- **R4 — Existing input behavior.** Keep standard selected-copy priority and
  repeat/release ownership, paste bypass, direct page/history behavior, pointer
  routing, existing selection-driven keyboard handling and transport input fences.
  Non-prefix input follows its existing route and encoding.
- **R5 — General evidence.** Required regression coverage uses a generic child
  enabling enhanced keyboard flags. Herdr is supplementary smoke evidence.
  Distinguish injected PTY, physical-key and remote-network observations.
- **R6 — Input-method boundary.** Match reported key identity/modifiers and
  applicable base-layout identity, not an assumed equivalence between a physical
  key and committed text. The actual flags-0/7/15 capture distinguishes changed
  punctuation and encoded keys from timeout or a missing prefix.
  Input-method adaptation is excluded: add no punctuation mapping, input-method
  switching or prefix-triggered reporting request. Reuse key identity when it
  is already delivered; pure U+3002 text is not period. A general command owner
  does not imply universal physical-key delivery through terminal input methods.

## Acceptance criteria

- [x] AC0: Root cause, generic/Herdr reproductions, source anchors and limits
  are retained in [findings](./research/findings.md) and
  [outcomes](./research/outcomes.json).
- [x] AC1 (R1): Legacy/enhanced/mixed/fragmented prefix enters the same command
  state; plain/encoded period resolves to Detach through one binding lookup.
  Adding a future binding requires no decoder/prefix recognition changes.
  No application-specific branch or actual new command binding is added.
- [x] AC2 (R2): connect, session new and session attach expose no --escape option;
  supplying it fails normal argument parsing. No configurable/disabled prefix
  state remains. Unknown commands and timeout cancel with no attempted-command
  bytes sent to the child. Fixed-prefix quoting preserves child input fidelity;
  help/docs describe Ctrl+] and local cancellation.
- [x] AC3 (R3): Press/release succeeds; repeats do not fabricate another press;
  no orphan lifecycle events, spurious modifier matches or unbounded buffers.
- [x] AC4 (R4): Existing copy, paste, page/history, pointer, selection-driven
  mode handling and input-fence checks pass. Normal input retains existing
  routing/encoding; the only CLI syntax change is removal of --escape.
- [x] AC5 (R5): One generic outer-PTY case proves enhanced detach, terminal
  restoration and a live/re-attachable Session. Relevant checks and direct
  main-agent review pass with evidence limitations recorded.
- [x] AC6 (R6): Entering/leaving command mode causes no new keyboard-reporting
  policy change. Already-reported period keys can resolve the binding; U+3002
  text cannot trigger detach and cancels locally. The real IME diagnosis and
  remaining Chinese-punctuation limitation are retained without claiming an
  input-method fix. No further IME capture is required by this task.

## Diagnosis and artifacts

Research on 2026-09-05 used baseline 3f77461 (zterm 0.1.19), Herdr 0.8.2,
generic flags-3/9 children and actual Herdr flags 7. Enhanced detach failed;
legacy 0x1d plus period detached in the same modes.

Classification: **bounded CLI input-boundary architecture defect**.
HostInputCodec already decodes enhanced keys at terminal_ui.rs:2427, but
route_enhanced_input returns child bytes at :2812 before session.rs:199 calls
byte-only PrefixParser (terminal_ui.rs:1305). There is no encoding-independent
prefix owner. Correct that boundary within the existing CLI; do not redesign
unrelated command, terminal or transport ownership.

[design.md](./design.md) specifies the shared command-mode boundary.
[implement.md](./implement.md) specifies work and validation.
The [IME research](./research/ime-input.md) records pinned upstream sources and
the completed [physical capture](./research/doubao-physical-keys.json).
The user approved this reduced scope with “好，开始执行吧”; task.py start
recorded in_progress. The shared command owner, caller integration, CLI removal
and regression tests are implemented. The full local just check gate passed;
final focused/full checks and completion evidence are recorded in verification.md.

## Out of scope

Input-method adaptation and further IME probes, prefix-triggered or global
keyboard-reporting changes, punctuation remapping, clipboard/other shortcut
changes, new command bindings, history-key migration,
Herdr changes, forced keyboard-mode disabling, new protocol
support, runtime-configurable shortcut frameworks, daemon production/core/protobuf changes, deployment,
releases and manipulation of existing user sessions.
