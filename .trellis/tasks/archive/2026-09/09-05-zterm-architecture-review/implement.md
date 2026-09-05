# Implementation Plan

执行方式：仅主代理，明确禁止所有子代理，包括 Trellis 角色。任何上下文清单仅供主代理加载。
审查和隔离复现已完成，用户已明确确认实施。先处理独立的模型捕获与候选构造，再迁移客户端并修复其控制预算，最后收拢 UI 状态；各步骤边界和验收不变。

## 0. 规划与基线

- [x] 创建任务，保存范围、原始基线和执行约束。
- [x] 审查主链路及生命周期/连接/授权/重放边界；F1–F6 已有代码证据。
- [x] 执行工作区测试、`just check-fast` 和 F1 故障观察器。
- [x] 形成 PRD、design、执行计划和真实上下文清单。
- [x] 用户审阅最终方案后明确回复“好，那就按你说的改”，满足规划门，进入 `in_progress`。

## 1. 修复客户端响应性（D1 / F1）

修改边界：`crates/daemon/src/local_ipc.rs`、`operations.rs` 及其现有测试。

- [x] 将探针中的静默租约/无关帧/阻塞写转为现有 socket fixture 的回归。故障观察器目前期望发现问题，不能直接当修复后 pass 测试。
- [x] 命令排队、传输写入、相关控制响应共享 absolute deadline；出队过期命令不写出。
- [x] 对 deferred 内容施加 8 帧/8 MiB 编码 payload 总量限制，涵盖路径侧带；超限结束当前 epoch。
- [x] 部分写或写超时作废 transport，验证后续命令不会复用残缺 stream。
- [x] 证明普通 idle event wait 不超时、退出能释放 owner、Remote resume 不重放 input/takeover。

验证：`cargo +1.98.0 test -p zterm-daemon --lib --all-features`；
`cargo +1.98.0 test -p zterm-daemon --test local_session_ipc --test controller_lease --all-features`。
只补能区分新失败机制的测试，不重复整个协议组合矩阵。

## 2. 去掉已确认的重复工作（D2–D3 / F2–F4）

修改边界：`crates/terminal/src/model.rs`、`projection.rs`（仅在捕获重用需要时）、
`crates/daemon/src/terminal_driver.rs`、`local_ipc.rs`、`operations.rs`、
`crates/core/src/terminal.rs`、`crates/cli/src/terminal_ui/surface.rs`。

- [x] 模型一次捕获返回 update 与新 checkpoint；driver 不再为同一 revision 二次投影。
- [x] core 提供唯一候选构造，UI 不预先 clone 再调用另一个事务应用。
- [x] snapshot/delta/history protobuf 转换直接转移所有权。
- [x] 准备阶段移交初始 snapshot，运行态不保留无消费者的完整初始屏幕。
- [x] 逐项复核 F2–F4 操作次数的前后路径；保留必要的物理画面候选和全部校验。

验证：`cargo +1.98.0 test -p zterm-terminal -p zterm-core -p zterm-proto -p zterm-cli --all-features`；
`cargo +1.98.0 test -p zterm-daemon --test attachment_resync --test terminal_drain --test terminal_recovery --all-features`。
补一条捕获输出与 returned checkpoint 严格一致的回归；沿用 delta/Unicode/resize/flush-failure 现有契约测试。

## 3. 整理源代码所有权（D4 / F5–F6）

修改边界：`crates/daemon/src/client/`、`lib.rs`、`local_ipc.rs`、`operations.rs`，
`crates/cli/src/terminal_ui.rs` 及已有 `terminal_ui/` 模块。此步不再变更协议行为。

- [x] 迁移前列出真实生产/测试 import，避免公开 API 或非 Unix 条件编译漂移。
- [x] 明确同一 client 内 transport/session/view/ipc 子模块，server 侧保持同 UID ingress 与 opaque tunnel。
- [x] 把 UI event loop 的状态与转移方法收敛到一个具体 owner，保留事件入口同步判断和唯一 presenter。
- [x] 移走 obsolete 字段/重复流程，保留只为真实消费者服务的 façade；不增加新 crate、actor framework 或后台任务。
- [x] 验证 local/remote target-visible trace、Main/Alternate ACK、resize fence、history/selection 和输入模式提交失败行为不变。

验证：前述 daemon/CLI 测试；`cargo +1.98.0 test -p zterm-daemon --test two_daemon_transport --all-features`。
macOS 的此目标仅能提供共享 fixture/编译证据，真实 Iroh 由 Linux 拥有。
若改动实际 UI 转移，执行现有 `sh tests/foundation/terminal-blackbox.sh --mode herdr`；缺少既有前提则记录限制，不重建另一个 harness。

## 4. 最终验收

- [x] 主代理按 `trellis-check` 逐项对照 PRD，确认不是只有文件变小，而是重复计算/状态流程确实消失。
- [x] 更新与实际新 API/边界对应的 terminal-model、terminal-driver、core-wire-domain、local-daemon-ipc specs；不提前把设计写成已实现事实。
- [x] 本机执行一次 `just check`；发现失败后只针对失败点迭代，再补必要受影响验证。
- [x] `git diff --check`；确认 source policy、format、Clippy、测试、docs 和依赖门的实际结果。
- [x] 记录平台证据限制，提交/归档进入下一节的一次性确认。

## 回退与停止条件

每步保持源码可独立回退，不涉及数据库迁移、tag、发布、部署或现有用户会话。
出现 snapshot/ACK/lease/flush 原子性回归，先恢复原有契约，再重新审视设计，不能通过忽略错误或添加 route 特例使测试变绿。
F1–F6 验收通过后停止；不把延后项、引擎比较、性能 benchmark 或新平台纳入本任务。

## 5. 已确认的提交与收尾

- [x] 用户已回复“确认 走发布流程吧”，批准以下工作提交和收尾，并授权后续发布流程。
- [x] 工作代码与 specs 已提交为 `0c21738`；随后按 finish-work 归档任务及记录 journal。
- 后续推送、PR 和发布按 `docs/releasing.md` 执行；最新用户指令已授权发布，不再受此前仅本地收尾范围限制。

工作提交：`refactor: simplify terminal architecture and bound client control`

文件范围：
- `crates/terminal/src/model.rs`、`crates/core/src/terminal.rs`
- `crates/daemon/src/client/{mod,ipc,session,transport,view}.rs`
- `crates/daemon/src/{lib,lifecycle,local_ipc,operations,terminal_driver}.rs`
- `crates/daemon/tests/local_session_ipc.rs`
- `crates/cli/src/terminal_ui.rs`、`crates/cli/src/terminal_ui/{session,surface,composition}.rs`
- `.trellis/spec/backend/{terminal-model,terminal-driver,core-wire-domain,local-daemon-ipc}.md`

上述 21 个工作文件全部由本会话修改，无未识别改动。任务的 10 个文件随 Trellis 归档提交，journal 随记录提交，遵循工作提交在前、记账提交在后的顺序。原始运行日志及提取脚本都位于忽略的 `target/architecture-review/`，不进入提交。
