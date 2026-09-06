# Proposed commit batch

1. `fix(cli): unify Ctrl+] command handling`

   - `README.md`
   - `docs/remote-cli.md`
   - `crates/cli/src/lib.rs`
   - `crates/cli/src/terminal_ui.rs`
   - `crates/cli/src/terminal_ui/keyboard.rs`
   - `crates/cli/src/terminal_ui/prefix.rs`
   - `crates/cli/src/terminal_ui/session.rs`
   - `crates/cli/src/terminal_ui/session_tests.rs`
   - `crates/cli/tests/daemon_autospawn.rs`
   - `crates/daemon/tests/support/daemon_harness.rs`
   - `.trellis/spec/backend/index.md`
   - `.trellis/spec/backend/local-daemon-ipc.md`
   - `.trellis/spec/backend/terminal-input-commands.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/check.jsonl`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/design.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/implement.jsonl`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/implement.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/prd.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/capture_host_keys.py`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/doubao-physical-keys.json`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/findings.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/ime-input.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/outcomes.json`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/research/run_escape_probe.py`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/task.json`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/verification.md`
   - `.trellis/tasks/09-05-investigate-zterm-herdr-escape/commit-plan.md`

All listed paths belong to this task and were authored in this task session.
Unrecognized dirty files: none. Local ignored build logs are not included.

After this work commit, run the standard Trellis task archive and journal
bookkeeping, each in its own generated commit. Do not amend. The user subsequently authorized the release flow, including
push, PR merge and publication through the repository release operator.

Validation: final `just check` passed; see [verification](./verification.md).
The user accepted continuation with “走发布流程吧”, authorizing this commit
batch and the v0.1.20 patch release.
