# Desktop connection investigation

Baseline: `bd4e320`, 2026-09-08. Product source was clean before this task.

## Confirmed reproduction after plan approval

The user approved implementation, then ran `probe_outer_terminal.py --run` in
Ghostty and reported visible garbage. `outer-terminal-results.json` identifies
Ghostty 1.3.1: production/batched/split-batched probes return no final cursor
report; the single-index case returns cursor (1, 1) unchanged and color replies.
The single-index case observed 141 palette entries; partial observations remain
possible and must retain the existing Unknown/bounded-collection behavior.

The macOS unified log, read with the specific predicate
`process == "ghostty" AND eventMessage CONTAINS "error processing terminal data"`,
records two `error.OutOfMemory` errors per batched case at 20:43:18.445,
20:43:19.247–248, and 20:43:20.055 on 2026-09-08. No such error was recorded for
the subsequent single-index case. This is a read-only diagnostic, not UI control.

The pinned [Ghostty 1.3.1 color reply handler](https://github.com/ghostty-org/ghostty/blob/v1.3.1/src/termio/stream_handler.zig#L1204)
allocates an ArrayList response using a 1024-byte FixedBufferAllocator for each
OSC color operation. The relevant limit is **response allocation**, not the
197-byte incoming query. Zig 0.15.2's [ArrayList growth](https://github.com/ziglang/zig/blob/0.15.2/lib/std/array_list.zig#L1406)
can require allocation headroom beyond the final reply length. A 32-slot 16-bit
reply requires 854–896 bytes; geometric growth can exceed the fixed allocator
even though the final logical response would fit in 1024. `try writer.print` propagates
the error before the response is sent. [Termio's slice handler](https://github.com/ghostty-org/ghostty/blob/v1.3.1/src/termio/Termio.zig#L728)
logs the error and abandons the remainder of that slice. A later slice can resume
with lost sequence context, explaining the visible query tail.

**Final classification:** local physical-probe compatibility defect in ZTerm's
batch encoding, exposed by an upstream fixed-response-allocation/error path.
Keep the existing owner/round/decoder architecture. Encode one palette index per
complete OSC command, serialized together in one write/flush and one observation
round. This avoids the demonstrated per-command response growth without terminal
brand detection, new waits, palette guesses or treating a cosmetic repaint as a fix.

The earlier undetermined classification below describes the planning evidence and
is superseded by this confirmed reproduction and diagnostic trace.

## Implementation validation

- Palette queries now use separately terminated single-index OSC commands while
  preserving the one-round collection budget, role coverage and final DSR fence.
- The new actual-emitter regression fails with only the original batch emitter
  restored and passes with the correction.
- Startup output is replayed through the existing disposable Session terminal:
  readable stages without elapsed time, safe target, resize, first session replacement
  and normal Active status are verified. Pending local/remote-labelled waits
  retain cancellation and output-failure Session results without input admission.
- Real disposable local-daemon CLI acceptance passes. The final status is checked
  through terminal replay rather than assuming contiguous incremental ANSI bytes.
- `just check` passed with exit 0 on 2026-09-08, including all native workspace
  tests, Clippy, documentation, dependency policy and relay static checks.
- Corrected `--run --candidate` Ghostty output is pending. No actual remote
  network startup or corrected physical UI smoke is claimed. The installed CLI
  and existing user daemon/session were not replaced or restarted.

## Observed failure and evidence

The user sees a mostly empty macOS terminal during connection with
`4;?;185;?;186;?;187;?;188;?;189;?;190;?;191;?` at the top. They confirmed
Ghostty and direct local execution, with apparently the same behavior for local
and remote targets. Installed app metadata reports Ghostty 1.3.1, build 15212;
this does not by itself verify the running process's exact build.

| Boundary | Source anchor | Finding |
| --- | --- | --- |
| CLI request | `crates/cli/src/lib.rs:769` | Connect constructs a deferred terminal request; the target path does not own a separate connection screen. |
| Physical entry | `crates/cli/src/terminal_ui.rs:125`, `:209`, `:939` | The guard enables raw mode, enters alternate screen and hides the cursor before preparing a Session. |
| Appearance probe | `crates/cli/src/terminal_ui.rs:274`, `:277` | Queries run before stateful create/attach; initial local collection has a 250 ms bound. |
| Query output | `crates/cli/src/terminal_ui/ansi_presenter.rs:270`, `:280` | Eight OSC 4 commands query 32 palette slots each; other role/appearance requests and DSR follow in one buffered write/flush. |
| Connection wait | `crates/cli/src/terminal_ui.rs:313`, `:481` | Inactive wait consumes replies, cancellation, input and resize; it does not paint connection progress. |
| Prepare internals | `crates/daemon/src/operations.rs:951`, `:1112`, `:1122` | Daemon readiness, target resolution, local/tunnel attachment and initial snapshot occur beneath the prepare future. |
| First session paint | `crates/cli/src/terminal_ui.rs:336`, `:359` | Only a returned PreparedTerminalView supplies the first authoritative surface and status renderer. |
| Initial ACK | `crates/cli/src/terminal_ui.rs:381` | A second inactive wait exists before the active view. |

The complete palette output is 1466 bytes. Individual frames are
155, 165, 165, 193, 197, 197, 197 and 197 bytes. Reconstructing the exact emitter
loop finds the screenshot's whole fragment at zero-based offset 1025, in the
160–191 frame. `probe-byte-analysis.json` retains the escaped bytes and assertion
result. This is an outgoing query (`?`), not the normal RGB reply format.

The numerical proximity to 1024 is a useful split-boundary fixture, **not proof**
of a buffer limit: this offset excludes preceding guard output, and actual PTY
read boundaries were not observed. Do not hard-code a 1024-byte theory.

## Classification and intended owners

1. **Missing desktop presentation lifecycle coverage:** the sole physical
   presenter exists, but startup before an authoritative surface has no view.
   Give that presenter explicit startup ownership; no daemon/transport redesign
   or synthetic Session snapshot is necessary.
2. **Escape leakage: undetermined mechanism.** Byte provenance is strongly
   established at the outgoing physical query boundary, but an actual output
   capture/emulator reproduction is needed to distinguish emission/interleaving,
   stream-split parsing and terminal compatibility. Do not claim an input-codec
   defect, upstream Ghostty defect, or oversized command without that evidence.

Invariants: physical query controls never become visible cells or child input;
the sole presenter owns startup and session output; pending connection work has
readable progress; preparing/ACKing a session preserves its existing side-effect,
input-epoch and cancellation contracts.

## Counterevidence and alternatives

- [Ghostty OSC 4 documentation](https://ghostty.org/docs/vt/osc/4) explicitly
  supports repeated index/value query pairs. A batch of 32 is not intrinsically
  invalid protocol.
- The upstream [multi-pair issue #7402](https://github.com/ghostty-org/ghostty/issues/7402)
  was resolved for 1.2.0. It does not establish a failure in installed 1.3.1.
- [Ghostty v1.3.1 OSC source](https://github.com/ghostty-org/ghostty/blob/v1.3.1/src/terminal/osc.zig)
  has a 2048-byte normal OSC buffer and drains invalid data. ZTerm's largest
  palette command is 197 bytes. Its ordinary buffer cap cannot explain the
  screenshot on its own.
- [Ghostty v1.3.1 stream source](https://github.com/ghostty-org/ghostty/blob/v1.3.1/src/terminal/stream.zig)
  retains parser state across input slices. Source inspection does not prove a
  runtime split bug.
- Both target routes execute the probe before the network/Session prepare, so
  changing relay routing would not address its byte source.
- Shortening the wait or painting over artifacts would hide the symptom without
  establishing query/reply correctness.

## Planning-time validation and limitations (historical)

`cargo +1.98.0 test -p zterm-cli host_colors --lib`: **7 passed, 0 failed**.
These cover split/oversized replies, paste ownership, epoch fences, stale replies,
subscription ownership and queried-slot coverage. They do not execute an outer
emulator or assert a startup UI, so they do not reproduce or disprove this report.

The physical-query emitter was reconstructed for byte analysis; no production
code was changed and no daemon/session was started, stopped or replaced.

Computer-use access to `com.mitchellh.ghostty` was rejected with
“Computer Use is not allowed to use the app ... for safety reasons.” No alternate
UI-control route was used. Actual Ghostty visual reproduction was pending at
planning time; the user-run reproduction above subsequently supplied that evidence.

## Planned reproduction (subsequently executed as recorded above)

Use a new disposable ordinary terminal shell and a standalone probe with no
ZTerm daemon or remote Session. Establish raw input ownership before queries,
drain replies, restore modes, and capture output/replies separately. Compare:

1. The exact complete production probe, including entry and terminal mode bytes.
2. Palette-only commands, then the 160–191 batch alone.
3. The same semantic query set as one index per complete OSC, sent in one round
   (no per-index wait and no palette setters).
4. The original stream split at positions around the observed fragment and
   inside introducers/terminators, without changing logical controls.

Record visible cells, exact bytes and process/build identity. If isolated queries
do not reproduce, capture one disposable actual CLI startup and compare all
physical writers. Set the leakage classification before selecting its code fix.
Then retain an application-independent regression for the failing boundary and
smoke both local and remote entry. Never use the user's existing main Session as
a test fixture or restart a live daemon for this investigation.
