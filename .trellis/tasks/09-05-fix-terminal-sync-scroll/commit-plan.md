# Proposed work commit

`fix: preserve terminal sync barriers and live presentation`

One coherent bug-fix batch includes the shared event contract, UI presentation,
regressions and the corresponding executable specs and causal task record.

## Files

- `crates/daemon/src/client/session.rs`
- `crates/daemon/src/client/view.rs`
- `crates/daemon/tests/local_session_ipc.rs`
- `crates/cli/src/terminal_ui.rs`
- `crates/cli/src/terminal_ui/session.rs`
- `crates/cli/src/terminal_ui/session_tests.rs`
- `crates/cli/tests/daemon_autospawn.rs`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/session-service.md`
- `.trellis/spec/guides/root-cause-and-architecture-thinking-guide.md`
- `.trellis/tasks/09-05-fix-terminal-sync-scroll/` — all task documents, context
  manifests and planning/acceptance probe sources, including this plan and
  validation.md. Ignored target logs, copied source and binaries are excluded.

Unrecognized dirty files: none.

Validation: [validation.md](validation.md).

Status: approved by the user on 2026-09-05: “提交，走发布流程吧。”
The approved batch is unchanged. Archive and journal bookkeeping follows this
work commit, then the normal PR/CI/merge and next patch release flow.
