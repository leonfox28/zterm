# Terminal Input Command Contract

## 1. Scope / Trigger

The CLI owns local terminal commands before child encoding. This contract
applies when adding a zterm control, changing host-input decoding/routing, or
changing the attach CLI. The original byte-only prefix parser missed enhanced
Ctrl+] because the already-decoded key was encoded back into CSI-u first.
Child application identity is never an input-routing condition.

## 2. Signatures

```text
zterm connect <target> [--session <name-or-id>] [--takeover]
zterm session new <target> <name> [--cwd <host-path>]
zterm session attach <target> <session> [--takeover]
```

These commands have a fixed Ctrl+] prefix. `--escape` and the internal
`EscapePrefix`/`TerminalRequest.escape` configuration no longer exist.

```rust
CommandMode::route(event: HostInputEvent, now: Instant, report_events: bool)
    -> Result<Vec<PrefixAction>, CliError>
PrefixAction::Input(HostInputEvent)
PrefixAction::Command(LocalCommand)
LocalCommand::Detach
```

`prefix.rs` owns the static `BINDINGS` table. Currently only `.` maps to
`Detach`. Add future zterm controls to this table and the command executor;
do not add suffix checks to the decoder or another prefix parser.

## 3. Contracts

- Flow: `HostInputCodec` -> typed physical reply dispatch -> existing
  selected-copy priority -> `CommandMode`
  -> command executor or existing encoding/viewport/Active input gates.
  Inactive attach waits use the same command owner, with their existing
  cancellation and input-admission rules. Color/appearance/status replies
  never cancel a prefix or selection, including across input epochs. Literal
  bracketed paste retains ownership of apparent replies inside it; see
  [Terminal Colors](./terminal-colors.md).
- Legacy 0x1d and an enhanced Ctrl+] press enter a one-shot command mode for
  one second. Match exact modifiers after excluding lock bits, using reported
  primary/base-layout identity; associated text alone never selects a binding.
- A second deliberate Ctrl+] press quotes one literal key through the existing
  forwarding policy. Repeats are consumed for owned keys, do not quote, and do
  not refresh the deadline. Owned releases are consumed even if Ctrl went up
  first; unowned releases and pure modifier events keep their existing owner
  and do not cancel the pending command.
- Unknown command keys and timeout cancel locally. No implicit replay of a
  pending prefix or attempted command reaches the Session. Unknown opaque
  CSI/Alt/SS3 units are consumed whole. An unknown UTF-8 scalar consumes its
  continuation bytes across read boundaries, preserving the following key.
- `report_events` comes from the last successfully presented keyboard flags.
  Store at most 64 held command-key identities while release reports are
  requested. Retire a lease on release or a new press of that identity; retain
  no press payloads or repeat history. Epoch reset clears all command state.
- Selected copy retains priority and its existing lease. Paste and mouse
  cancel a pending command and retain their existing routing. Page/history
  keys keep their normal behavior outside command mode.
- Route unconsumed enhanced keys before selection invalidation changes the
  encoding context. `ForwardedBytes` are already interpreted and must never
  be scanned again for a prefix. Opaque frames also bypass embedded-byte
  prefix matching when no command is pending.
- Command state never changes presenter keyboard flags. Keep the existing
  child/selection-driven policy; add no reporting query, override or IME
  mapping. Pure Chinese U+3002 text is an unknown command, not `.`. Text without
  a reported key identity cannot establish ownership of a physical-key release.
- Detach closes only the view; the Session and child PTY survive. Transport,
  wire, storage and environment configuration gain no new fields.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| `--escape` on connect/new/attach | Clap `UnknownArgument` before execution |
| Prefix plus plain/enhanced period | One local `Detach`; no chord input forwarded |
| Unknown suffix or deadline | Local cancellation; no replay or error |
| U+3002 suffix | Consume text scalar; no detach or physical-key inference |
| Ctrl+Alt/Super/Shift+] | Existing non-prefix input route |
| Another held identity beyond the 64-key cap | `ResourceExhausted`; existing terminal cleanup runs |
| EOF, reset, attachment loss | Clear pending input under existing fences |

Existing codec frame/paste limits and their errors remain authoritative.

## 5. Good / Base / Bad Cases

- Base: `1d 2e` detaches; `1d 1d` forwards one literal Ctrl+].
- Good: `CSI 93;5u`, matching release, `CSI 46u` also detaches, including
  fragmented reads and an intervening pure Control release.
- Good: prefix + `x` + `ok` consumes the attempted command and forwards `ok`.
- Bad: searching re-encoded CSI-u bytes for 0x1d misses a valid prefix;
  matching associated U+002E text from a different key triggers the wrong command.

## 6. Tests Required

- `terminal_ui/prefix.rs`: real codec-to-command regression, legacy/enhanced/
  mixed/fragmented input, exact modifiers and base-layout matching, lifecycle,
  quote, unknown UTF-8/opaque units, timeout/reset and no orphan owned releases.
  Assert commands and exact forwarded input, including unrelated trailing text.
- `missing_release_reports_do_not_accumulate_held_keys`: many distinct command
  attempts without requested releases must not exhaust the held-key bound.
- `removed_escape_option_is_rejected_on_all_terminal_entry_points`: normal
  parsing, removed-option rejection and fixed-prefix help for all three commands.
- `daemon_autospawn` enhanced-prefix scenario: generic child enables flags 15;
  unknown commands/timeout leave authoritative revision unchanged; enhanced
  detach restores the outer terminal; reattach preserves Session ID and mode.
- Retain existing copy/elevation, page/history, pointer, paste and input-fence
  regressions. PTY-injected sequences do not prove physical IME compatibility
  or remote-network behavior.

## 7. Wrong vs Correct

Wrong: `decode -> encode for child -> scan bytes for Ctrl+] and '.'`.

Correct: `decode -> CommandMode -> BINDINGS -> LocalCommand`, with unconsumed
events passed to the existing forwarding owner. This keeps future command
bindings independent of keyboard wire syntax.

See [Local daemon and IPC](./local-daemon-ipc.md) for presenter, clipboard and
Session contracts that this input owner must preserve.
