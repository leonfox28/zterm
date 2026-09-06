# Reattach input freeze: authoritative geometry is replaced by an initial guess

## Conclusion and classification

**Local implementation defect in CLI attachment initialization.** The retained
Session's initial snapshot is authoritative for its size, but `run_view` seeds
resize deduplication from a provisional Main-screen creation size. It never
reconciles that baseline with the existing Session snapshot. This generates an
unnecessary same-size resize on Alternate reattach, makes the CLI wait for a
synchronization barrier that is not generated, and drops ordinary input forever.

The existing owners are sufficient: the host owns actual geometry, the CLI
projects desired geometry and deduplicates resize, and explicit snapshot/resume
barriers control input admission. Correct the initialization contract at this
boundary; do not replace the prefix system, invent Herdr detection, acknowledge
ordinary deltas, or redesign transport.

## User report and evidence boundary

The user runs `zterm connect dev`, starts Herdr, detaches using Ctrl+] then `.`,
and reconnects. Herdr's screen returns; only zterm's local detach still works.
Local installed versions are zterm 0.1.20 and Herdr 0.8.2. Research uses main
997a731 (v0.1.20), macOS arm64, Rust 1.98.0 and unchanged production libraries.

The failure was reproduced in private **local** Sessions, independently of
network transport. The same `run_view` initializer is used for local and remote
views. No existing dev Session, production daemon, pairing or remote state was
read or mutated. This establishes the generic CLI failure class; the exact
remote daemon version/route and the user's physical terminal were not captured.

## Reproduction and controls

The research runner reuses the archived private runtime harness. It links the
real CLI/daemon code, local IPC, Session service, PTY and terminal model. Each
case has a unique `/tmp/zt-rea-*` root; Herdr has private config/socket/XDG paths.
It reuses the same outer 140x40 PTY across detach and reattach, and observes exact
bytes received by a raw child or a shell-created marker inside Herdr.

| Case | Initial input | Input after reattach | Local detach after reattach |
| --- | --- | --- | --- |
| Generic Main, flags 7 | Received | Received exactly | Exit 0 |
| Generic Main, flags 15 | Received | Received exactly | Exit 0 |
| Generic Alternate, flags 7 | Received | No bytes | Exit 0 |
| Generic Alternate, flags 0 | Received | No bytes | Exit 0 |
| Herdr 0.8.2, flags 7 / Alternate | `before.txt` created | `after.txt` absent; Herdr quit also ineffective | Exit 0 |
| Generic Main, outer width changes 140 -> 150 while detached | Received | Received, but retained viewport incorrectly stays 139 columns rather than 149 | Exit 0 |

The flags-0 case uses the legacy detach chord, proving that enhanced-prefix
matching is not necessary for this defect. All private daemon cleanup commands
returned 0; private Herdr server cleanup returned 0. Terminal termios differs
only by macOS's kernel-maintained `PENDIN` bit, also excluded by the existing
`daemon_autospawn` restoration assertion (lines 1311-1322); normalized termios
restoration passes. The raw equality diagnostic is not another input failure.

Reproduce from the repository root:

```sh
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_reattach_probe.py
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_reattach_probe.py --alternate-only
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_reattach_probe.py --alternate-only --observed
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_reattach_probe.py --matrix
```

The `--observed` build adds metadata-only traces to isolated source copies under
ignored `target/`; it changes no behavior or tracked product source. Compact
results are in [baseline-outcomes.json](./baseline-outcomes.json), and the causal
sequence is retained in [causal-trace.log](./causal-trace.log). Raw ANSI and build
logs stay under ignored `target/reattach-input-probe/`.

## Causal chain with source anchors

1. `crates/cli/src/terminal_ui.rs:249-265` assumes Main before receiving the
   snapshot. A 140-column outer terminal yields a 139-column creation hint and
   `ResizeCoalescer.last_submitted = 139`. Main reserves a chrome gutter;
   Alternate uses the full 140 columns (existing local-daemon-ipc spec:362-370).
2. The hint is not an accepted resize of an existing Session.
   `crates/daemon/src/session.rs:627-653` uses `initial_viewport` to create a
   missing main, but `PrepareAttach` for a retained Session carries no size.
   The equivalent remote path is `:660-695`. The returned snapshot remains 140.
3. `terminal_ui.rs:298-305` correctly derives desired Alternate width 140 but
   compares it with the provisional 139, rather than snapshot width 140. It
   queues a redundant 140-column resize.
4. Initial snapshot acknowledgement emits Active (`client/view.rs:335-354`).
   `ResizeCoalescer::enter_transport_state` (`terminal_ui.rs:1342-1355`) changes
   that to Synchronizing because the stale baseline yields a pending resize.
   `terminal_ui/session.rs:696-737` sends the resize and commits that state.
5. The trace proves `SERVER RESIZE current=140 requested=140`. The host advances
   revision for resize (`crates/daemon/src/terminal_driver.rs:471-479`,
   `crates/terminal/src/model.rs:171-184`),
   but unchanged geometry yields an ordinary delta, not a replacement snapshot.
   Observed: `UI DELTA from=56 to=57 ... state=Synchronizing barrier=false`.
6. Ordinary deltas correctly do not produce snapshot ACKs. Only a correlated
   resume barrier is acknowledged at `terminal_ui/session.rs:573-574`.
   No subsequent Active is generated for this ordinary delta; input remains
   gated. Restoring the old practice of ACKing arbitrary deltas would violate
   the queued-delta fixes and can recreate `not_synchronized` failures.
7. The trace then records `UI INPUT state=Synchronizing len=16` and **no**
   `CLIENT SEND TerminalInput`; `session.rs:227-234` drops live ordinary input
   outside Active. The local command path at `:197-211` still handles Detach,
   followed by `CLIENT SEND TerminalDetach`, explaining the user's asymmetry.

The sibling Main-screen case confirms the same bad baseline in the opposite
direction: desired size equals the provisional size, so the real resize needed
by the retained smaller Session is omitted. This is why the remedy must use
snapshot geometry, not merely suppress one Alternate-screen resize.

## Rejected explanations and scope

- Enhanced keyboard parser/IME: generic Alternate flags 0 also fails; generic
  Main flags 7/15 forwards the exact same received input after reattach.
- Herdr-specific state: application-neutral Alternate fixture fails identically.
- Lost stdin reader: post-reattach stdin and local Detach are in the trace.
- Remote network alone: a private local attachment reproduces the complete chain.
- Dead Session or child: initial input succeeds; the Session remains Attached;
  ordinary input is blocked before IPC. The generic child remains live.
- Raw terminal restoration: only kernel PENDIN differs, matching existing tests.

`git blame` shows the provisional baseline and comparison predate v0.1.20's
prefix change. This is a latent initialization bug exposed by the newly working
exit flow, not evidence that the semantic prefix architecture should be removed.

## Why previous acceptance missed it

The v0.1.20 `daemon_autospawn` fixture does send a marker after reattachment and
verifies model progress; however, its enhanced child stays on **Main**. The
alternate-screen scenario returns to Main before detaching. Those checks do not
exercise a retained Alternate snapshot during the next `run_view` initialization.
The new acceptance must combine retained Alternate, initial snapshot geometry,
reattachment, input delivery and completion of the Active fence.

## Recommended repair and limits

Reconcile the sole resize coalescer with the authoritative initial snapshot
before deriving the first post-attach command. Compare desired layout from the
latest physical dimensions and snapshot screen with that actual size. Equal
sizes must not request another resize or reopen synchronization; different sizes
must submit one real resize and wait for its proper barrier. Preserve input
fences, copy/paste, prefix handling and ordinary-delta ACK rules.

Investigation is complete. Product code has not been modified. The proposed
implementation and acceptance plan are in ../design.md and ../implement.md;
implementation and publication are not authorized by this investigation request.
