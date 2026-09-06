# 支持在远程会话内更新 zterm

## Goal

用户通过 zterm 连接远程服务器后，可以在远程终端内执行 `zterm update` 并完成远程服务器的 zterm 升级；关闭会话导致连接断开，不应使升级本身中止。

## Background and Confirmed Facts

- 用户报告：在远程 zterm 会话内执行 `zterm update`，命令提示存在运行中的 session；输入 `y` 后连接断开，但远程 zterm 未升级。尚未在用户服务器复现或检查现场日志。
- CLI 在当前进程直接调用更新操作（`crates/cli/src/lib.rs:863`）；确认后先停止 daemon（`crates/daemon/src/operations.rs:495`），再激活候选程序（`:520`），没有脱离原 PTY 的更新执行者。
- daemon 关闭所有 session（`crates/daemon/src/session.rs:1016`）并终止 PTY root child（`crates/platform/src/pty.rs:473`），原会话中的 updater 没有独立存活保证。
- 当前更新已支持签名校验、原子替换、激活失败回滚，以及成功后为已配置设备启动新 daemon。现行契约明确已结束的 PTY 不恢复（`.trellis/spec/backend/distribution-lifecycle.md`、`docs/install.md:127`）。
- 因果分析、根因分类与验证方向见 `research/current-behavior.md`。

## Requirements

- **R1 — 入口与目标：** 保留 `zterm update [--version <vSEMVER>] [-y|--yes]`，更新命令执行所在服务器、当前有效用户的 managed installation。
- **R2 — 独立生命周期：** 经确认并成功交接后的升级，必须能在原 PTY、前台命令或远程连接结束后继续完成。同一原则覆盖本机 zterm 自连接内执行 update 的等价场景。
- **R3 — 确认与交接：** 保留先下载校验、再确认会话影响、最后停止 daemon 的顺序。执行者接受前的取消、EOF、交接失败，以及未授权的非交互调用，均不终止会话或替换程序；接受后的前台断开遵循 R2。空闲预检后新出现的会话仍需授权，交接不等于强制中断授权。
- **R4 — 完成与诊断：** 成功后保留身份和配对，启动目标版本 daemon，并可重新连接。断开前明确连接影响与后续操作；“成功交接”不能当作“更新成功”。失败结果与恢复指引不能只存在于已消失的终端。
- **R5 — 既有升级契约：** 沿用签名验证、版本与目标检查、原子激活、回滚以及启动失败的部分完成语义。

## Acceptance Criteria

- **A1 → R1/R2/R4：** 从真实 zterm 远程 PTY 发起更新并确认交接后，即使连接断开，服务器仍完成更新；重连后可验证目标版本和运行中的 daemon，身份与配对保留。
- **A2 → R2：** 本机 zterm 自连接内的更新也不因自身 PTY 结束而中止；普通外部终端更新保持准确的最终结果及退出状态。
- **A3 → R3：** 执行者接受前的取消、EOF 或交接失败，以及未授权非交互调用，均不终止现有会话或替换已安装程序；预检后新出现的会话在未获中断授权时也保持存活。
- **A4 → R3/R5：** 下载、签名及候选验证失败保留现有程序和会话；版本选择与 `-y` 保持现有语义。
- **A5 → R4/R5：** 激活失败恢复旧程序；新程序已提交但启动失败时，明确记录部分完成与恢复操作，不虚报成功或声称恢复已结束的会话。断开后仍有获取结果和排障的途径。

## Confirmed Scope

用户在 2026-09-06 明确允许结束现有 session。沿用确认后结束全部 session 的语义：后台完成更新，成功后自动启动远程 daemon，用户手动重新连接；不恢复已结束的 shell 或其中运行的进程。

根因归类为更新流程自身的执行生命周期架构缺陷：缺少独立于被停止 daemon / PTY 的更新执行者；不需要重构远程传输或 session 持久化架构。具体故障信号仍以真实 PTY 回归验证为准。

## Out of Scope

- 保留现有 session、跨 daemon 重启恢复进程、升级后自动重连。
- 新增控制端 `zterm update <device>` 命令或新的远程管理协议。
- 定时更新检查、常驻 updater 服务、系统启动服务、sudo / 系统级安装。
- 恢复已暂停的平台发行目标或新增混合版本兼容适配。
- 对 `daemon restart`、`reset`、`uninstall` 等相邻命令做完整生命周期重构。

## Compatibility and Risks

- 已安装旧版的 updater 不能因下载了新版候选程序就获得新版执行逻辑。第一次迁入修复版本仍需从不会被该 daemon 关闭的外部终端（如 SSH）执行升级；后续才可在 zterm 内使用修复后的流程。
- 已完成隔离的真实 PTY 更新回归：原流程失败、独立更新流程通过；正式发行包在真实远程服务器上的验收仍待发布后执行，不能用本机测试替代。
