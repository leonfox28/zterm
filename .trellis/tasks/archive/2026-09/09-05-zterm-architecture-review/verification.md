# Architecture Review and Implementation Verification

Baseline: `fde63f6`. Host: macOS arm64. Date: 2026-09-05.
Execution owner: the main agent only; no sub-agents or external agent runners.
User approved implementation after reviewing the three architecture changes.
Implementation and spec synchronization are complete. The user approved the
Phase 3.4 commit and subsequent release process on 2026-09-05. Work commit:
`0c21738` (`refactor: simplify terminal architecture and bound client control`).

## Final implementation gates

| Command | Result | Evidence / limit |
| --- | --- | --- |
| `just check` | Exit 0 | Source/version/engine policy, format, actionlint, release/shell/Python fixtures, workspace all-target Clippy, secret scans, full workspace tests, docs, both cargo-deny workspaces, relay checks and Docker Compose metadata |
| Workspace tests inside `just check` | 507 passed, 0 failed, 6 ignored | 47 standard summaries including nested helper harnesses; custom harnesses also succeeded |
| `sh tests/foundation/terminal-blackbox.sh --mode herdr` | Exit 0 | Pinned/checksummed Herdr 0.8.2; alternate screen, 47x123 resize, detached progress, resync; both gate and cleanup PASS |
| `rustfmt +1.98.0 --edition 2024 --check crates/cli/src/terminal_ui/session.rs` | Exit 0 | Explicit check for the included UI module, which Cargo's module walker does not format |
| `task.py validate` and `git diff --check` | Exit 0 | Nine real entries in each context manifest; no whitespace failures |
| Windows MSVC target check from macOS | Prerequisite failure | `ring` C compilation lacks the Windows SDK `assert.h`; this is not Windows product compile/runtime evidence |

Final full-gate log: `target/architecture-review/just-check.log`.
Other raw outputs remain under the same ignored directory. No logs or binaries
are proposed for commit.

`ATTACHMENT_RESYNC_GATE=PASS`, `TERMINAL_DRAIN_GATE=PASS` and
`PTY_LIFECYCLE_GATE=PASS` were observed. `CROSS_UID_GATE=SKIPPED_NON_LINUX` and
Linux-only real-Iroh cases remain platform limits. The default workspace
black-box skip is supplemented by the explicit Herdr run; no other application,
Linux runtime, signing, installer, deployment or publication evidence is claimed.

## Acceptance mapping

| Findings | Implemented boundary and proof |
| --- | --- |
| F1: unbounded controls | `client::view` puts admission/dequeue/I/O under one absolute deadline. `SessionClient` bounds lease waiting, sent-takeover correlation and deferred frame/path storage. `transport` moves epoch ownership into a write future so partial-write timeout/cancellation cannot be followed by another command. Socket regressions assert silent lease vs idle reads, 9-frame and 10 MiB floods, tunnel sidebands, blocked and cancelled partial writes, takeover ambiguity, queue expiry and remaining-budget writes. |
| F2: repeated projection | `TerminalModel::capture` projects once and returns update plus exact checkpoint; both driver sync paths use it, retaining the equal-revision early return. Consecutive capture replay matches a fresh snapshot through Main/Alternate changes and resize. |
| F3: nested candidate copies | Core constructs and validates one candidate; `apply_to` delegates and UI calls `candidate` directly. Invalid baseline row count returns a typed error without changing the baseline. Existing Unicode, revision-gap, history and failed-flush tests pass. |
| F4: unnecessary content ownership | Consumed snapshot/delta/history protobuf messages move into conversion. Prepared construction takes the initial snapshot once; the running client holds no obsolete initial full screen. Integration fixtures retain the taken revision when inspecting initial content before ACK. |
| F5: client/server separation | `client::{transport,session,view,ipc}` owns frontend behavior in the existing crate. `local_ipc` retains server ingress and actual compatibility exports; `operations` retains use cases/lifecycle. Native route parity, reconnect, typed closure, clipboard and controller tests pass. No extra actor, crate or background owner was added. |
| F6: explicit UI owner | `TerminalUiSession` owns mutable session state and event/transport/prefix/viewport transitions. Delta ACK eligibility is captured before resize side effects. `run` funnels errors and normal outcomes through one cleanup. Semantic surface, history and committed physical frame remain distinct. Existing UI tests and Herdr black-box pass. |

Efficiency evidence is structural: two full projections become one; two complete
semantic candidate copies become one; three consuming protobuf conversions no
longer clone; the initial client-to-prepared snapshot clone/retention is removed.
No throughput, latency, CPU, RSS or engine benchmark was run, and no measured
speedup percentage is claimed.

## Baseline fault evidence

Before implementation, workspace tests passed (497 passed, 0 failed, 6 ignored),
`just check-fast` passed, and the isolated socket observer reproduced:

```text
LEASE_WAIT: still pending at 6 s; 128 unrelated frames retained
CONTROL_WRITE: still pending at 6 s; 0 full input messages completed
```

`research/attachment_deadline_probe.rs` and `research/run_probe.py` intentionally
preserve the pre-fix observer for baseline `fde63f6`. Reproduce it in that source
checkout; it targets the old initial-snapshot API and is not a post-fix gate.
Its expectations have been replaced by the production-module regressions above.
It never creates a real daemon, identity, network connection or PTY.

## Review corrections and bookkeeping

- Module moves preserved tests with their owners. One combined redaction test
  became separate unary-state and takeover-token tests; eight client regression
  tests and one model capture test account for the net ten added tests.
- The full gate caught two fixtures taking the same initial snapshot twice.
  They now retain the initial revision and transfer content only once; the final
  workspace run passes. The secret scanner also caught a test variable named
  `token`; it was renamed without changing scanner policy.
- Windows cfg imports were inspected. Cross-target compilation could not reach
  project code because of the missing SDK, and is explicitly not marked passed.
- Existing context size warnings remain for `local-daemon-ipc.md` and
  `transport-auth.md`; the main session read relevant sections directly.
- All implementation acceptance criteria are checked. Phase 3.4 approval is the
  remaining workflow step; archive and journal auto-commits follow work commits.
