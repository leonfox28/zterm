# Design: one semantic prefix command mode

## Invariant and change boundary

Ctrl+] enters a local AwaitCommand state regardless of the supported keyboard
encoding requested by the child. Subsequent keys enter a common command binding
lookup; currently period resolves to Detach. Ctrl+] is the fixed prefix; remove
customization and disabling as explicitly requested by the user. Raw byte
spelling is transport representation, not shortcut identity.

The current decoder already knows the enhanced key. The prefix owner receives
raw child-directed bytes too late, losing the information it needs. This is a
bounded input-boundary architecture defect; no Herdr detection or terminal/
transport redesign is indicated. See [research](./research/findings.md).

The latest clarification makes command mode the abstraction to implement.
Preserve copy, paste, other shortcuts, their owners and their normal routing.
Add no command bindings and migrate no existing PageUp/PageDown behavior.
Remove --escape as already requested. Device switching is an example of a
future dispatcher extension, not an action to implement in this task.

## Data flow and responsibility

    Input -> existing framing/key decoding -> semantic input with wire identity
          -> Normal: existing input owners; Ctrl+] enters AwaitCommand
          -> AwaitCommand: classify command key -> binding lookup -> LocalCommand
                -> execute command (currently Detach only), return to Normal
                -> unknown key / timeout: consume attempted command, cancel locally
          -> unconsumed input through existing child encoding/gates

- Reuse EnhancedKey identity, modifiers, kind and raw representation. Adapt
  legacy control input into the same prefix classification.
- Replace the byte-only prefix owner with one Normal/AwaitCommand state machine
  holding a deadline and bounded owned-key identities. Keep one state
  machine, not separate legacy and Kitty parsers. A small prefix.rs module may
  keep this owner cohesive.
- Separate key normalization, static binding lookup, LocalCommand and command
  execution. The binding lookup contains only period -> Detach. The decoder and
  prefix recognition must not know which suffix triggers detach. Adding a later
  command means extending the bindings and executor, not this input boundary.
- Repeated-prefix literal quoting, timeout and unknown-command resolution are
  command-mode transitions. A pending suffix is classified by this owner before
  any child forwarding. The user selected local cancellation for unknown keys
  and timeout: consume the attempted command and its owned lifecycle, with no
  implicit replay. Explicit double-prefix quoting remains separate. Command mode
  is one-shot with the existing deadline, not a new persistent modal UI.
- Use a small typed dispatcher/static match; no dynamic registry, user config,
  plugin interface or unimplemented device-switching command is needed.
- Preserve current selected-copy eligibility/priority, CopyKeyLease, direct
  page/history behavior and pointer routing. They remain separate owners;
  decoded identity alone does not authorize a new local shortcut.
- Normal unconsumed enhanced input keeps the existing encoding policy: raw bytes for
  matching outer/child modes or unrecognized mismatch; legacy downgrade only
  for deliberate selection-driven elevation over a zero-flag child.
- Capture any forwarding context before selection invalidation can change the
  presented mode. Forwarded events retain their original representation; local
  cancellation requires no retained prefix payload.
  Do not scan encoded fallthrough bytes for prefix a second time.
- Active session and inactive attach waits must use the same prefix semantics.
  Keep their current cancellation, epoch and host-input admission rules.

Paste and opaque sequences bypass prefix matching. Preserve other event routing;
cancel an earlier pending command before unrelated text/paste is admitted where
needed for order, without redesigning copy, paste or pointer behavior. Do not
divide an unknown UTF-8 input unit at an arbitrary stdin-chunk boundary and leak
its remaining bytes after cancellation. The codec frames opaque keys; the
command owner retains at most three UTF-8 continuation-byte positions.

## Matching

Recognize fixed Ctrl+] from decoded key identity/modifiers, and legacy 0x1d from
raw control input. Reuse existing relevant identity helpers instead of enumerating
CSI-u byte spellings. A legacy 0x1d byte cannot identify which physical alias
produced it; enhanced matching uses the reported Ctrl+] identity. There is no
remaining configurable C0/DEL domain or disabled-prefix branch to support.

Use primary/alternate/base-layout information consistently. Ignore lock bits.
Do not turn extra Alt/Super/Hyper/Meta into Ctrl+] by lossy conversion.
Alternate/layout fields may identify the prefix, but Shift is not an arbitrary
wildcard. Match a logical unmodified period, plain or enhanced; a different
key's associated text containing a period is not sufficient.

No raw CSI-u string list, second parser, new protocol support or application
heuristic is part of this change. See the input-method investigation below for
the distinction between reported keys and committed Unicode text.

## Input-method adaptation excluded

The user's completed Ghostty 1.3.1 + Doubao 0.9.7 capture shows two chords in
each mode: flags 0 delivers legacy Ctrl+] plus U+3002; flags 7 delivers enhanced
Ctrl+] plus U+3002; flags 15 delivers enhanced Ctrl+] plus period key 46. All
intervals are 350-448 ms. [IME research](./research/ime-input.md) retains the
physical trace, timings and pinned-source anchors.

After discussing drawbacks, the user declined the proposed temporary reporting
adaptation. Command mode is local routing state only; entering or leaving it
must not request different outer keyboard flags. Keep the presenter's current
child/selection-driven policy. Add no reporting override, protocol query,
acknowledgement state, per-command keyboard-stack operation, or new general
input transcoder. Further physical IME probes are outside this task.

The command owner consumes key identity already received. A reported period key
can match regardless of associated text; plain U+3002 cannot be inferred to be
that key and cancels the pending command locally. Do not match on release,
remap Chinese punctuation, switch the user's input method or add a terminal/
input-method-specific branch. The captured Chinese-punctuation limitation
therefore remains for modes that deliver only text.

The withdrawn proposal is not needed to fix the proven enhanced-prefix
recognition defect: existing decoding already supplies key identity. Its risks
were a suffix encoded before mode activation, IME composition hiding future
letter keys, varying terminal support and added restore/event complexity. Those
risks explain the scope decision, not additional work to perform before starting
this reduced implementation.

Keep legacy/enhanced input, modifier lifecycles and local cancellation on one
owner. Explicit quoting and normal forwarding use the existing mode policy;
no automatic replay of failed commands is introduced.

## Pending state and lifecycle

Retain a deadline and at most 64 owned key identities when the last presented
flags request event types. No raw press/repeat payload is retained. Consume owned
releases, retire stale ownership on a new press, and clear state on epoch reset.
Without release reporting, do not accumulate held-key records. Preserve the
existing one-second timeout; explicit quoting forwards the second press.

| Input/state | Result |
| --- | --- |
| Idle, Ctrl+] press | Own the key as applicable and await command |
| Pending, matching release | Keep waiting; consume release and retire ownership |
| Pending, plain modifier event | Maintain lifecycle bookkeeping; keep waiting; never treat as command suffix |
| Pending, command-key press | Resolve shared binding; currently period -> Detach; consume matched prefix and command |
| Pending, second deliberate prefix press | Quote one literal key; consume first lifecycle and forward second once |
| Pending, prefix repeat | Consume as part of the current prefix lifecycle; do not extend deadline or fabricate a second deliberate press |
| Pending, unknown command / timeout | Consume attempted command and cancel locally; never replay |
| Release not owned by prefix handling | Follow existing handling |
| Epoch reset / attachment loss | Clear obsolete pending state under current fences |

Do not accumulate unbounded repeats. Keep only necessary bounded event/lease
state. The captured standalone Control release is legitimate lifecycle input,
not malformed input or an unknown suffix. Preserve the separate copy lease.

Current-epoch local detach remains available during synchronization without
permitting host input. Any chosen forwarding continues through existing Active/
history gates. Snapshot/epoch resets, EOF and cleanup use this same pending owner.

## Files

Expected production boundary:

- crates/cli/src/lib.rs: remove --escape from connect/session new/session attach,
  parse_escape_prefix, EscapePrefix, and per-invocation propagation into
  TerminalRequest and debug/dispatch construction. No replacement config option.
- crates/cli/src/terminal_ui/keyboard.rs: reusable key/prefix and pure-modifier
  identity predicates, retaining existing copy and legacy-conversion helpers.
- crates/cli/src/terminal_ui.rs: evolve/wire prefix ownership and the inactive
  wait path, with only necessary codec distinctions.
- crates/cli/src/terminal_ui/session.rs: semantic prefix integration, fallthrough,
  timeout and lifetime handling.
- Optional crates/cli/src/terminal_ui/prefix.rs: cohesive command state, static
  bindings and LocalCommand output, not an additional protocol parser.

Verification belongs alongside the owner, in terminal_ui/session_tests.rs where
needed, and one existing daemon_autospawn outer-PTY fixture. Its shared
crates/daemon/tests/support/daemon_harness.rs test shell gains a flags-15 trigger;
this is test-only, with no daemon production change. Normal presenter behavior,
other CLI configuration, core/protobuf/daemon/transport and other shortcut
implementations are not intended change targets. Presenter keyboard-mode policy
changes are explicitly outside scope.

The semantic-prefix executable contract lives in
.trellis/spec/backend/terminal-input-commands.md, linked from the backend index
and local-daemon-ipc.md. Remove the option from
README.md command summaries and docs/remote-cli.md usage/explanations, and say
the prefix is fixed. Archived task evidence is historical, not current CLI docs.

The removed option is a deliberate CLI compatibility change: previous invocations
containing --escape (including --escape ctrl-] or --escape none) fail normal
argument parsing. Do not silently ignore the flag, retain a hidden compatibility
option, or replace it with an environment variable. There is no persisted state
migration because the old selection was per invocation.

## Validation and risks

Focused behavior cases cover legacy/enhanced control, encoded period, mixed
input, fragmentation, relevant modifiers/layout aliases, quoting, timeout and
exact fallthrough. Replace obsolete custom/disabled-prefix tests with argument
rejection checks for the three affected CLI entry points and normal fixed-prefix
construction. Verify help/current documentation no longer advertises the option.
Use flags 0/3/7/9/15/31 only where they distinguish behavior; avoid a Cartesian
product. Assert actual local actions and emitted child input, not helper internals.

Keep existing copy/elevation, paste, direct page/history, pointer and fencing
checks as compatibility evidence. Session checks own context capture/reset
interleaving. One generic enhanced-input child in an outer PTY owns detach,
terminal restoration and surviving/re-attachable Session acceptance.

Main risks: double scanning, quoting in the wrong encoding/order, orphan release
and stale prefix state. Each is covered at its owning boundary. Herdr smoke is
supplementary; PTY injection does not prove physical-key or remote-network
coverage. No new command binding or configurable shortcut option is proposed;
failed-command local cancellation is selected above.

## Execution and rollback

All implementation/check/review runs directly in the main agent; no sub-agents.
The user approved the reduced proposal with “好，开始执行吧”; the task is
in_progress and implementation is complete. Local cancellation is implemented;
input-method adaptation and temporary mode switching remain excluded. See
verification.md for check results and the remaining commit/finish step.

Integrate the one prefix owner and its callers together. Rollback reverts this
bounded CLI change and related tests as a unit. No data/schema migration,
deployment, release or existing-session changes.
