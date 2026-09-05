# Validation record — 2026-09-05

Host: macOS arm64, Rust 1.98.0. Work branch: `fix/terminal-sync-scroll`.
Baseline: v0.1.16 / `13600e67952e2c448dcf0f1241609e2366c03b58`.
All execution and review were performed by the main agent; no subagents.

## Owning regressions

- `queued_deltas_across_resize_do_not_create_snapshot_ack_cascades`: actual UI,
  command driver, SessionWireServer and SessionService. Covers the frontend fence
  while the target is still Active, physical resize with a queued delta, the real
  mode snapshot -> Active -> deferred resize -> queued delta sequence, and a valid
  explicit resume delta whose presentation/ACK completes ResumePending.
- The actual screen switch produces a full snapshot, not a streamed mode-changing
  delta. Tests retain this observed sequence instead of inventing a target mode
  delta. Ordinary delta ACK authority is tested independently of UI sync state.
- Direct/Tunnel socket tests submit resize between output emission and event
  consumption and compare typed ordinary events plus exact outgoing command kinds.
  View-driver queue tests retain ResumeDelta meaning across state changes. Existing
  correlated remote reconnect tests require the new variant. These are component
  boundary tests, separate from the real local UI/target test and paired-dev smoke.
- `resume_snapshot_presents_all_live_rows_before_activation_or_more_input` checks
  every child row for Local/Remote display metadata and with/without new output,
  offset-zero chrome, input/paste retention, cursor restoration and no-I/O equality.
- Snapshot flush failure preserves surface/layout/metrics/readiness/retained input;
  background snapshots preserve pinned history cells. Existing delta write/flush,
  selection, input epoch, paste, capture, resize and takeover regressions pass.
- The real `daemon_autospawn` outer-PTY scroll scenario now compares all 23 x 79
  child cells before scrolling, while in history, and after wheel-down, before any
  further click/input/output. Captured ANSI is replayed through an isolated
  SessionService using the existing sole model, not a new test parser.

## Discriminating mutation checks

Both temporary changes were restored byte-for-byte immediately after the check:

| Removed correction | Expected observed failure |
| --- | --- |
| Restore state-derived delta ACK | Maintained UI/target regression exits 101 with NotSynchronized |
| Restore retained-history composition during ResumePending | Maintained presentation regression exits 101 on the first child-row comparison |

Logs: `target/terminal-sync-scroll/mutation-{ack,presentation}.log`.

## Passed checks

- `cargo +1.98.0 test -p zterm-daemon --lib --all-features`: final 204 tests pass.
- `cargo +1.98.0 test -p zterm-daemon --test controller_lease --test local_session_ipc --test attachment_resync --all-features`: pass.
- `cargo +1.98.0 test -p zterm-cli --lib --all-features`: final 60 pass; 3 isolated
  helper tests remain explicitly ignored in the parent suite and are exercised by
  their existing subprocess owners where applicable.
- `cargo +1.98.0 test -p zterm-cli --test daemon_autospawn --all-features`: exit 0.
- Final `just check`: exit 0; workspace tests, Clippy, formatting, policy, docs,
  dependencies, release fixtures and relay checks pass. One new queue test initially
  used a socket fixture without a Tokio runtime; corrected its test annotation and
  reran the full gate successfully. No product failure was ignored.
- `sh tests/foundation/terminal-blackbox.sh --mode herdr`: PASS, cleanup PASS;
  Herdr 0.8.2 alternate screen, resize 47 x 123, detached progress and resync.
- Explicit rustfmt checks for the included session and session_tests modules pass.
- `git diff --check`, task manifest validation, Python syntax, JSON and task-local
  Markdown link checks pass. The pre-existing large IPC spec exceeds the injection
  cap; the main agent read its affected sections directly.

## Actual Herdr acceptance and cleanup

- Fixed, uninstrumented libraries: local default persistent Herdr at 50 x 180
  succeeds 3/3. All cases show Herdr UI and no NotSynchronized on start or detach.
  Previously the same baseline setup failed 3/3.
- Separate --no-session control at 40 x 140 succeeds. It is not substituted for
  the default persistent application workflow.
- Built CLI -> existing paired `dev` daemon -> shell -> already primed isolated
  Herdr server at 40 x 140 succeeds. The UI is visible and no NotSynchronized occurs
  during startup or detach (`zterm-causal-930d7d0fac/outcome.json`).
- The remote close reply was `operation_outcome_unknown`, so the runner exited 1
  during cleanup. A subsequent read-only `session list dev --json` showed only
  the user's `main` Session; the owned test Session was already removed. No close
  retry or action against `main` was performed. Isolated local daemons and Herdr
  state were stopped/removed by their scoped cleanup.

Raw records are retained under ignored `target/terminal-sync-scroll/`:
`just-check.log`, `herdr-blackbox.log`, `herdr-50x180-persistent-fixed/`,
`herdr-40x140-monolithic-fixed/`, `zterm-causal-930d7d0fac/`.

## Platform boundary

The Linux-owned real Iroh two-daemon loopback test remains explicitly ignored on
macOS (`two_daemon_owners_reuse_endpoint_for_pair_and_normal_confirmation`). The
workspace's native shared two-daemon test passes. Actual paired Linux-dev smoke
is separate evidence, not a claim that the Linux CI gate or other architectures
have executed here. No release or version change is part of this work.
