# Alacritty Migration Difference Ledger

Date: 2026-09-02

## Implementation Baseline

- Source baseline: `1374bb756a99a7822451491b6275e8965f5fd35f`.
- Toolchain: `rustc 1.98.0`, `cargo 1.98.0`.
- The pre-existing modification to
  `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/task.json` was
  user-owned and remains outside this migration.
- Before cutover, the existing core terminal corpus and snapshot/delta suites
  each passed 5/5. Daemon attachment resync, session limits, terminal drain,
  and terminal recovery passed; the downloadable black-box test retained its
  explicit-only default skip.
- No terminal performance, throughput, latency, CPU, or RSS benchmark was run.

## Must Preserve

| Contract | Evidence after cutover | Status |
| --- | --- | --- |
| One Session = one PTY/root child/model | Existing platform/driver/session ownership remains; daemon constructs one `TerminalModel` per one `PtySession` | Preserved |
| Ordered ingest and one revision per non-empty chunk | terminal corpus plus model revision tests | Preserved |
| Main/alternate, cursor, styles, wide/combining Unicode, modes | whole/one-byte/fixed/random corpus | Preserved |
| Exact DA/DSR/CPR replies | terminal corpus and driver DSR real-PTY test | Preserved |
| Snapshot, latest merged delta/resync, bounded history | snapshot/delta suite and attachment resync | Preserved |
| Revision-only updates remain ordered | same-size resize emits a contiguous empty-ANSI delta; takeover/barrier fixtures accept intervening contiguous deltas | Preserved |
| No-drop zero-attachment PTY drain | `terminal_drain` | Preserved |
| 8 Sessions, 240x80 viewport, 2,000 history | session limits and default domain limits | Preserved |
| Fixed hosted child identity | real PTY login shells inherit the fixed profile under Ghostty-, kitty-, tmux-like, and unset parent environments | Preserved |
| Core/proto engine isolation | manifest/source policy plus executable dependency-tree fixture | Preserved |

There is no unresolved must-preserve difference.

## Approved Visual Normalization

Alacritty represents an untouched default blank and an explicitly written
default-styled space identically. Zterm therefore normalizes both to an empty
default projected cell. This changes no displayed result or input mode. Styled
blank cells, wide heads/spacers, and bounded combining text remain distinct and
are covered by the semantic corpus.

## Security Hardening

- A bounded streaming ingress policy now consumes OSC/DCS/APC/PM/SOS and
  unsupported queries before Alacritty. Secret hyperlink, clipboard, and
  control payloads do not reach grid/state/replies/ANSI/Debug.
- OSC 52 and Kitty keyboard are disabled; OSC 8, synchronized-update 2026,
  REP, and underline-color controls are rejected or contained.
- Control sequences retain at most 256 bytes, control strings 1,024 bytes,
  titles/icons 256 source bytes, replies 64 KiB/update, and side events 32/update.
- A control string exceeding 1,024 bytes is discarded through its terminator
  and emits only one payload-free classification; it cannot become a truncated
  title/icon event.
- Cell text is fixed at 22 UTF-8 bytes. Main/alternate combining usage is
  tracked independently and capped across the Session at 4,096 retained cells
  and 64 KiB. A scalar crossing any cap is discarded before `CellExtra` growth
  and emits a bounded character classification.
- `BoundedEventSink` applies its own 32-event bound before the model collector,
  preventing an upstream callback flood from creating an intermediate backlog.

## Removed Policy (Intentional, Not a Compatibility Difference)

Per the user's approved decision, the migration removes
`TerminalResourceProjection`, aggregate fixed-cell accounting, the 128 MiB
terminal-memory admission rule, the 256 MiB RSS gate, the terminal-state
benchmark, and its resource-gate script. No Alacritty capacity estimate or
replacement memory reservation was added. Session count, dimensions, history,
wire, ingress, reply, event, title, and combining limits remain.

## Blockers

None identified in local implementation and targeted verification. Hosted
macOS/Linux/Windows and formal release-readiness jobs remain the authoritative
external owners for their platforms; local success is not recorded as hosted
evidence.

## Independent Check Security Finding

The independent check found and fixed an ingress framing bypass: a nested ESC
or C1 introducer, or an embedded C0 control, could previously be retained in a
partial ESC/CSI buffer and then interpreted by the upstream parser using its
ECMA-48 anywhere/execute transitions. That could allow filtered OSC or
synchronized-update 2026 input to reach the engine. The policy now restarts on
ESC/C1, dispatches embedded controls without adding them to buffered syntax,
and counts every consumed byte toward the sequence cap. Regression coverage
compares whole-input and one-byte chunks across a nested ESC restart and checks
that embedded controls cannot obscure the filtered sequence identity.

The same review corrected SGR underline-color classification. Raw substring
matching had rejected safe foreground/background RGB components equal to
58/59 while allowing the numeric alias `058` to reach Alacritty. Detection now
observes top-level SGR parameter consumption: ordinary indexed/RGB components
remain compatible, and leading-zero underline-color forms are contained before
they can create `CellExtra` state.

## Verification Record

| Check | Result |
| --- | --- |
| `cargo +1.98.0 test -p zterm-terminal --all-features` | PASS: 13 unit, 10 security, 5 corpus, 5 snapshot/delta, 0 doc tests |
| `cargo +1.98.0 test -p zterm-core --all-features` | PASS: 31 unit and 10 pairing-vector tests |
| `cargo +1.98.0 test -p zterm-daemon --lib --all-features` | PASS: 189 tests, including repeated full-suite confirmation after the revision-only fixes |
| daemon terminal black-box/drain/recovery/attachment/session-limit targets | PASS; black-box retained its intentional explicit-only skip |
| `cargo +1.98.0 test -p zterm-platform --all-features` | PASS: 21 unit plus real-PTY lifecycle and four fixed-profile parent variants |
| `cargo +1.98.0 test -p zterm-cli --all-features` | PASS; three intentionally explicit-only tests remained ignored |
| `cargo +1.98.0 test --workspace --all-features` | PASS |
| source/workspace-version/format/Clippy/docs/cargo-deny/`just ci-policy`/`just check` | PASS |
| `sh tests/terminal-dependency-policy.sh` and recorded `cargo tree` views | PASS; core/proto exclude the engine and `vte 0.15.0` has exactly the pinned Alacritty host path |
| independent targeted recheck: workspace type-check, terminal Clippy/tests, format, source/dependency policies | PASS after the two local ingress fixes above |

Hosted macOS/Linux/Windows matrix and four native release-readiness jobs were
not executed locally; their existing definitions include the new host crate
and remain hosted-only evidence for the next CI run.

No performance/RSS benchmark was run, and no post-migration performance
guarantee is made.
