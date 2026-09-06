# Reattach geometry repair verification

## Result and scope

Implemented and reviewed directly by the main agent on macOS arm64, Rust 1.98.0.
No sub-agents were used. Product changes are confined to the shared CLI
initializer and its inactive waits in `crates/cli/src/terminal_ui.rs`.

The validated snapshot now establishes ResizeCoalescer's actual size. The Main
creation hint is used only for creation. Both inactive waits retain physical
sizes; after initial acknowledgement the initializer resamples physical size,
projects using the snapshot screen, updates viewport/status, and observes the
final desired child size before the first Active event. Existing input epochs,
ordinary-delta ACK policy, prefixes, clipboard, daemon production and wire
contracts are unchanged. The generalized AsFd context allows real PTY testing
without adding a production test hook.

## Regression and checks

| Check | Evidence |
| --- | --- |
| Real initializer regression before repair | Failed as expected: same-size Alternate reattach advanced revision 43 -> 44; `target/reattach-input-probe/regression-red.log` |
| Real initializer regression after repair | Passed in final `just check`; same-size Alternate preserves revision, all reattachments preserve SessionId and accept unique child echo markers |
| Main/Alternate changed-width behavior | Main 79 -> 99 and Alternate 80 -> 90 outer-PTY cases converge and accept input; local raw probes independently verify 139 -> 149 and 140 -> 150 |
| Controlled inactive waits | `inactive_waits_retain_physical_resizes_and_discard_startup_input` passes for prepare and ACK policies, real SIGWINCH/PTY reads, two physical sizes and discarded queued input |
| CLI library | 68 passed, 3 ignored helper entry points; existing queued-delta, input fencing, prefix, history, copy/paste and cleanup tests pass |
| Final `just check` | Exit 0: workspace Clippy/tests/docs/fmt, source/release/secret policies, dependency checks and local relay static/upstream checks; `target/reattach-input-probe/just-check-final.log` |
| Explicit included-module formatting | session.rs, session_tests.rs, prefix.rs and keyboard.rs pass Rustfmt 1.98.0; no included module was edited |
| Task artifacts | Python AST and JSON parse checks pass; `git diff --check` passes |

Commands:

```sh
cargo +1.98.0 test -p zterm-cli --lib --all-features --locked --offline
cargo +1.98.0 test -p zterm-cli --test daemon_autospawn --all-features --locked --offline
just check
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_reattach_probe.py --verify-fix
python3 .trellis/tasks/09-06-investigate-zterm-reattach-input/research/run_paired_reattach_probe.py
```

Review strengthened the harness in two places: unique markers avoid accepting
retained prior text, and the deterministic visible-cursor child waits for a
complete Active presentation before input. One intermediate run exposed that
observing the daemon controller alone can precede CLI input readiness during a
real initial resize. The final harness observes the CLI's actual cursor fence,
without adding sleeps to production or replaying startup input.

A supplemental Rustfmt wildcard check found pre-existing formatting differences
in untouched ansi_presenter.rs, composition.rs and selection.rs. Each file was
byte-compared with HEAD and is unchanged. These are outside the fix; workspace
format policy and explicit checks of the relevant included modules pass.

## Application-neutral and Herdr acceptance

[research/post-fix-outcomes.json](./research/post-fix-outcomes.json) retains eight
asserted cases from the actual product build, without observation instrumentation:
Main flags 7/15, Alternate flags 0/7/15, changed Main and Alternate width, and
Herdr 0.8.2. Generic children receive the exact post-reattach input bytes. Herdr
creates the after-reattach marker, accepts its own quit command, and returns to
an interactive shell. All preserve the same SessionId, detach twice with exit 0,
restore termios excluding kernel-managed PENDIN, and clean up private daemons.
The private Herdr server stop also returns 0.

[research/post-fix-paired-outcome.json](./research/post-fix-paired-outcome.json)
records the paired dev smoke with a new disposable Session
`zterm-causal-5789d0be88`: initial and reattached Herdr screens are visible and an
octal-escaped printf produces the actual post-reattach marker. No
`not_synchronized` error appears. The paired close returned
`operation_outcome_unknown`; a subsequent successful read-only Session list
confirmed this exact test Session absent. No close was blindly replayed. The
isolated server has a shell cleanup trap and bounded watchdog. The reusable
research wrapper now handles that ambiguous close by observing absence.

The paired wrapper initially needed its ignored output parent directory created;
that filesystem error happened before any Session was created. The subsequent
smoke succeeded functionally, and cleanup was reconciled as described above.

## Direct review and limits

Reviewed the full physical -> ChromeLayout -> snapshot baseline -> queued resize
-> snapshot/Active -> ordinary input flow. Same-size geometry cannot generate
the spurious initial fence; actual changes retain synchronization. The shared
run_view applies to local and remote routes, with no application or keyboard
mode detection. The executable initialization contract and assertion owners are
recorded in `.trellis/spec/backend/local-daemon-ipc.md`.

This establishes the generic geometry failure class, not universal certification
of every TUI or terminal. Physical Ghostty/Doubao key events were not replayed;
PTY input is deterministic. The exact paired daemon version/selected route was
not recorded, and other supported hosts/signing/installers remain hosted CI
scope. No installed zterm executable or existing user Session was modified.

Implementation and acceptance are complete. On 2026-09-06 the user authorized
the commit, Trellis wrap-up and release with “走发布流程吧”. The work commit
is followed by archive/journal bookkeeping and version 0.1.21 preparation.
The release operator owns exact-head PR/main CI, candidate signing and immutable
publication; release evidence is reported after the formal workflow completes.
