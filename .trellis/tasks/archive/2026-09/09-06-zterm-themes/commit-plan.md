# Approved commit

One cohesive work commit:

`feat(terminal): support color and appearance protocols`

This includes model/protocol compatibility, required semantic metadata, bounded
frontend observations/rendering, local/remote Session propagation, regressions,
smoke fixture, executable specs and this task's planning/verification artifacts.
These layers must land together because create/attach kinds and required color
metadata intentionally reject mixed old/new peers.

All listed paths were created or edited for this task. Unrecognized dirty paths:
none. User authorized the commit and release workflow on 2026-09-06.
Trellis archive and journal follow as separate bookkeeping commits, then
v0.1.22 preparation, PR CI, merge, main candidate CI and protected publication.

## Exact file list

- .trellis/spec/backend/core-wire-domain.md
- .trellis/spec/backend/index.md
- .trellis/spec/backend/local-daemon-ipc.md
- .trellis/spec/backend/session-service.md
- .trellis/spec/backend/terminal-colors.md
- .trellis/spec/backend/terminal-driver.md
- .trellis/spec/backend/terminal-input-commands.md
- .trellis/spec/backend/terminal-model.md
- .trellis/tasks/09-06-zterm-themes/check.jsonl
- .trellis/tasks/09-06-zterm-themes/commit-plan.md
- .trellis/tasks/09-06-zterm-themes/design.md
- .trellis/tasks/09-06-zterm-themes/execution.md
- .trellis/tasks/09-06-zterm-themes/implement.jsonl
- .trellis/tasks/09-06-zterm-themes/implement.md
- .trellis/tasks/09-06-zterm-themes/prd.md
- .trellis/tasks/09-06-zterm-themes/research/color-frontend-architecture.md
- .trellis/tasks/09-06-zterm-themes/research/color-protocol-contracts.md
- .trellis/tasks/09-06-zterm-themes/research/color-reporting-protocol-scope.md
- .trellis/tasks/09-06-zterm-themes/research/color-state-architecture.md
- .trellis/tasks/09-06-zterm-themes/research/color-theme-capability-audit.md
- .trellis/tasks/09-06-zterm-themes/research/implementation-context.md
- .trellis/tasks/09-06-zterm-themes/research/nested-application-theme-diagnosis.md
- .trellis/tasks/09-06-zterm-themes/task.json
- crates/cli/src/terminal_ui.rs
- crates/cli/src/terminal_ui/ansi_presenter.rs
- crates/cli/src/terminal_ui/composition.rs
- crates/cli/src/terminal_ui/host_colors.rs
- crates/cli/src/terminal_ui/prefix.rs
- crates/cli/src/terminal_ui/selection.rs
- crates/cli/src/terminal_ui/session.rs
- crates/cli/src/terminal_ui/session_tests.rs
- crates/cli/tests/daemon_autospawn.rs
- crates/core/src/terminal.rs
- crates/core/src/terminal/colors.rs
- crates/daemon/src/client/ipc.rs
- crates/daemon/src/client/session.rs
- crates/daemon/src/client/view.rs
- crates/daemon/src/connection_broker.rs
- crates/daemon/src/local_ipc.rs
- crates/daemon/src/operations.rs
- crates/daemon/src/remote_session.rs
- crates/daemon/src/session.rs
- crates/daemon/src/session_color_tests.rs
- crates/daemon/src/session_wire.rs
- crates/daemon/src/terminal_driver.rs
- crates/daemon/tests/local_session_ipc.rs
- crates/daemon/tests/terminal_recovery.rs
- crates/proto/src/lib.rs
- crates/proto/tests/compatibility.rs
- crates/terminal/src/color_names.rs
- crates/terminal/src/colors.rs
- crates/terminal/src/engine.rs
- crates/terminal/src/ingress.rs
- crates/terminal/src/lib.rs
- crates/terminal/src/model.rs
- crates/terminal/src/projection.rs
- crates/terminal/tests/color_protocol.rs
- crates/terminal/tests/security_policy.rs
- proto/zterm/v2/session.proto
- proto/zterm/v2/terminal.proto
- proto/zterm/v2/wire.proto
- tests/foundation/terminal-colors.md
- tests/foundation/terminal-colors.py
- tests/source-policy.sh
