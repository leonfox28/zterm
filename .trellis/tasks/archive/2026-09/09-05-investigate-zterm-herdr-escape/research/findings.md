# Herdr prevents the local detach chord from being recognized

This document retains the original investigation and initial repair advice.
The current [design](../design.md) establishes a shared prefix command mode
and, as requested by the user, removes --escape to fix the prefix to Ctrl+].
Copy/paste, other existing shortcuts and selection-driven keyboard handling are
preserved; new copy/history bindings remain out of scope. Custom-prefix advice
below is historical. Recorded baseline observations remain unchanged.
The subsequent [Ghostty/Doubao investigation](./ime-input.md) retains completed
user-operated input capture: period becomes U+3002 in flags 0/7, while flags 15
report the period key. That is a separate failure from the encoded prefix below.

## Conclusion

The default zterm detach command is recognized as **legacy bytes**, not as a
keyboard gesture independent of the child's keyboard protocol. Herdr activates
Kitty keyboard enhancements. Zterm correctly mirrors those flags to its outer
terminal, but forwards the resulting enhanced Ctrl+] event without recognizing
it as its own detach prefix. The subsequent period cannot complete the command.

This is an **input-boundary defect in the CLI**: local-command recognition occurs
after enhanced key semantics have been reduced back to child-directed wire
bytes. It does not require a Herdr-specific fix, a transport change, or disabling
the child's keyboard protocol. The intended invariant is that the configured
local detach gesture works across supported keyboard encodings, with unrelated
input still preserved for the child. The CLI host-input/local-command router
should own that invariant.

Scope: investigation only. No product code was edited or deployed.

## Environment and reproduction

- Date: 2026-09-05; macOS arm64; repository baseline `3f77461` (v0.1.19).
- Installed Herdr: `/opt/homebrew/bin/herdr`, version 0.8.2.
- Libraries: built from the current checkout with Rust 1.98.0, all features,
  locked offline dependencies, debug profile.
- The runner reuses the archived `cli_runtime_probe.rs` and its build helper.
  It links unchanged CLI/daemon code and runs real `run_terminal`, local IPC,
  SessionService, PTY, keyboard-mode projection, and presentation. The fixture
  supplies only private user-state paths, a local-only daemon, and `/bin/sh`.
- Every case uses a unique `/tmp/zt-esc-*` directory. Herdr uses a separate config
  and socket. Existing user sessions, daemon, and pairing state are untouched.

From the repository root:

```sh
python3 .trellis/tasks/09-05-investigate-zterm-herdr-escape/research/run_escape_probe.py
```

The script asserts the observed **broken baseline**, so it is an investigation
fixture rather than an acceptance test for a future fix. Full captures and build
logs are under ignored `target/herdr-escape-probe/`; the compact machine-readable
result is retained as [outcomes.json](./outcomes.json).

| Case | Observed final outer flags | Enhanced chord | Legacy `0x1d 0x2e` |
| --- | --- | --- | --- |
| Ordinary shell | 0 | Not sent | Detached; exit 0 |
| Generic raw-input child | 3 | Stayed attached; child captured identical bytes | Detached; exit 0 |
| Generic raw-input child, all keys encoded | 9 | Stayed attached; child captured identical bytes | Detached; exit 0 |
| Real Herdr, default persistent mode | 7 | Stayed attached | Detached; exit 0 |

For flags 3 and observed Herdr flags 7, the injected chord was:

```text
Ctrl+] press:    ESC [ 93 ; 5 u
Ctrl+] release:  ESC [ 93 ; 5 : 3 u
period:          .
```

For flags 9, the period was also encoded (`ESC [ 46 u`). Generic-child capture
proved that the failed chord reached the host PTY unchanged. Herdr's `spaces`
UI appeared, and the outer presenter emitted `ESC [ = 7 u` without any local
selection. All four final daemon cleanups and the private Herdr server cleanup
returned exit 0. No `not_synchronized` output occurred.

An initial runner attempt used the archived cleanup argument `--force`, which
the current CLI rejects. The task runner was corrected to current `daemon stop
--yes`; that attempt's private orphan daemon was terminated by its exact PID.
The final recorded run passed all assertions and cleanup checks.

## Causal path and source anchors

1. The default escape parser maps `ctrl-]` to `0x1d`:
   `crates/cli/src/lib.rs:987`; its current parser test is at `:1896`.
2. The terminal engine's keyboard state is projected into `TerminalModes` at
   `crates/terminal/src/projection.rs:270`. The frontend mirrors nonzero child
   flags; `crates/cli/src/terminal_ui/keyboard.rs:656` and
   `crates/cli/src/terminal_ui/ansi_presenter.rs:416` own that decision/output.
3. `HostInputCodec` recognizes CSI-u as `HostInputEvent::EnhancedKey` at
   `crates/cli/src/terminal_ui.rs:2427`. The enhanced parser already retains
   key identity, modifiers, kind, and raw bytes at
   `crates/cli/src/terminal_ui/keyboard.rs:67` and `:93`.
4. The enhanced router only handles the local copy shortcut. When outer and
   child flags match, it returns `key.raw` unchanged at
   `crates/cli/src/terminal_ui.rs:2812`. Legacy downgrade is limited to temporary
   selection-driven elevation over a zero-flag child, not ordinary Herdr input.
5. `crates/cli/src/terminal_ui/session.rs:233` routes the enhanced key and queues
   its returned bytes back as `HostInputEvent::Bytes` at `:279`. The bytes branch
   invokes `PrefixParser::feed` at `:199` and otherwise writes input to the host.
6. `PrefixParser` at `crates/cli/src/terminal_ui.rs:1305` stores only an optional
   control byte and deadline. Its `feed` at `:1320` arms only when an actual input
   byte equals `0x1d`; the pending branch recognizes an actual `.` byte. CSI-u
   contains printable digits `93;5`, not the control byte, so the prefix is never
   armed. For flags 9 the period is independently invisible to this byte matcher.
7. Actual detach completion is checked after event routing at
   `crates/cli/src/terminal_ui/session.rs:398`. The legacy fallback reaches it
   successfully in the same running Herdr session.

The existing [local daemon/IPC contract](../../../spec/backend/local-daemon-ipc.md)
describes raw preservation when outer/child modes agree (`:413`). That remains
necessary for child input, but it does not establish a semantic detach owner
before forwarding. The distinction between local commands and child input is
the missing boundary; blindly downgrading all enhanced keys would violate the
existing forwarding contract.

The [Kitty protocol specification](https://sw.kovidgoyal.net/kitty/keyboard-protocol/#disambiguate-escape-codes)
specifies CSI-u encoding for Ctrl combinations when disambiguation is enabled.
Flags 7 combine disambiguation, event types, and alternate keys. Modifier value
5 represents Ctrl, and Unicode value 93 represents `]`; release kind is 3.
These definitions explain the injected bytes and the change from the legacy
control byte. Optional alternate-key fields do not change the underlying issue.

## Why existing checks missed this

- `crates/cli/src/terminal_ui.rs:3356` tests the prefix parser with raw C0 bytes.
- `crates/cli/src/terminal_ui.rs:5601` tests enhanced copy and byte-preserving
  forwarding, but does not combine enhanced Ctrl+] with the detach parser.
- The archived Herdr startup probe quits Herdr using its own prefix-q **before**
  sending zterm's raw detach bytes. A PTY by itself does not encode physical
  keys according to output keyboard flags, so injecting `b'\x1d.'` cannot catch
  this protocol-dependent failure even while a child requests enhancements.

The new generic-child experiment separates the cause from Herdr, alternate
screen state, network behavior, and the existing one-second prefix timeout
(`crates/cli/src/terminal_ui.rs:109`). Each chord is injected in one write; the
legacy counterpart succeeds under the same mode.

## Bounded repair recommendation

1. Recognize configured local prefixes and the following command from decoded
   key semantics before enhanced input is forwarded. Keep legacy input and
   enhanced input on one local-command state machine.
2. Retain original child-directed encoding for keys that are not consumed as a
   local command, including timeout, unknown command, and repeated-prefix
   behavior. Do not globally convert enhanced input to legacy bytes.
3. Define prefix press/repeat/release handling together, so the prefix's own
   release cannot cancel the pending command or leak an orphan event. Support
   an encoded period as well as a plain period, configured control prefixes,
   and `--escape none`. Preserve bracketed-paste bypass and copy ownership.
4. Keep the repair within CLI keyboard/local-prefix routing and its session
   integration; no process-name matching, Herdr modification, wire-schema
   change, daemon lifecycle change, or keyboard-mode suppression is indicated.
5. Add a regression at the codec/router/prefix boundary, then use one outer-PTY
   case with an application-neutral child that enables enhanced flags and sends
   an encoded detach chord. Cover fragmentation and event kinds in focused
   unit checks. Verify unrelated enhanced input remains byte-preserving and
   local detach still preserves the Session.

## Evidence limits

The physical user's terminal and remote device were not instrumented. The PTY
runner explicitly injects protocol-correct enhanced events in response to the
observed output mode. This establishes the current CLI bug and a real Herdr
trigger locally, but does not claim a recorded hardware-key trace or a remote
end-to-end reproduction. No claim is made that terminals lacking Kitty protocol
support must fail; those may continue sending legacy bytes and detach normally.
