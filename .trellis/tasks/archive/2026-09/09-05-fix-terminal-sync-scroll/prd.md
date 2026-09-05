# 修复终端同步退出与滚动回到底部缺行

## Goal

恢复 v0.1.16 中本地和远程启动 Herdr 的稳定性，以及本地/远程连接滚回最新位置时完整显示最新内容的行为，无需额外点击或输入。

## Background and Confirmed Facts

- 用户报告本地连接启动 Herdr 后以 `not_synchronized: attachment is not awaiting a snapshot` 退出；最初报告远程正常，后续原环境复测也失败。用户还报告两条 route 都在滚回最新位置时缺少末尾两行，点击后恢复。
- 用户澄清：远程流程是先进入 zterm shell，再输入 `herdr` 连接已有 Herdr 服务端。发布版客户端和已有隔离 Herdr 服务端的两条连接对照均能复现；用户随后复测原远程环境，也从正常变为失败。它不构成稳定的“本地失败、远程正常”对照，与已记录的共同同步时序缺陷一致；此前成功那次的具体时序仍不能倒推。
- 基线为已发布 v0.1.16 / `13600e6`。工作区初始干净；规划仅新增任务文档和隔离探针，未修改产品代码。
- R1 已在实际 CLI 路径和 Herdr 默认持久会话模式中复现：本地未修改库连续三次失败，实际 `dev` 连接使用未修改库也失败；隔离观察副本记录了两端等价的普通 delta 误 ACK→补发快照→重复 ACK→退出链。两端 CLI/daemon 均为 0.1.16，Herdr 均为 0.8.2。问题并非本地专有；用户此前正常的远程运行没有历史事件记录，不能倒推出其精确时序或将其成功直接归因于网络延迟。
- R2 已在本地/远程显示模式、滚动期间有/无新输出的四种组合中复现：逻辑已恢复 Live，实际 child cells 仍是旧历史内容；再呈现一次即恢复。缺行数取决于保留的历史位置，并非已证明存在固定减二的高度错误。
- 原 Herdr 黑盒通过 TerminalDriver，未经过 CLI ACK/最终画面路径（`tests/foundation/terminal-blackbox.sh:73`、`crates/daemon/tests/terminal_blackbox.rs:13`），且使用 `--no-session`（`terminal-blackbox.sh:165`）；真实默认 Herdr 走持久客户端/服务端路径，其启动输出序列不同。原 CLI scroll 验收只滚上去后退出（`crates/cli/tests/daemon_autospawn.rs:774`），未检查滚回底部后的行内容。
- 根因分类和完整因果锚点见 [research/causal-evidence.md](research/causal-evidence.md)。原始探针日志位于忽略的 `target/terminal-sync-scroll/`。

## Requirements

- R1（P1）：普通 delta 不得因客户端已请求 resize 或处于同步状态而被确认成 snapshot；两条连接启动/运行/退出 Herdr 不再因此断开。边界缺陷在于恢复 delta 与普通 delta 的意义被丢失（`crates/daemon/src/client/session.rs:579`、`:1105`），随后 UI 仅凭状态推断 ACK（`crates/cli/src/terminal_ui.rs:1416`、`terminal_ui/session.rs:531`）。覆盖实际观察到的 snapshot→Active→deferred resize（`terminal_ui/session.rs:707`），以及 physical/mode resize；错误 ACK 在 Active 时立即失败，在 Awaiting 时可能先补发快照（`crates/daemon/src/session.rs:3486`）再因重复确认失败。等价事件序列在本地和远程遵守同一合同。
- R2（P1）：滚轮回到最新位置后，最新完整内容必须呈现在屏幕上，无需点击、输入或等待新输出。既有呈现合同被快照合成路径违反（`crates/cli/src/terminal_ui/composition.rs:197`、`terminal_ui/session.rs:475`），Active 又无条件跳过呈现（`crates/cli/src/terminal_ui.rs:3151`）。
- R3：保留真实 snapshot/remote resume barrier 的精确 ACK、服务端严格 Awaiting/revision 校验（`crates/daemon/src/session.rs:3470`）、单一 SessionClient/viewport/presenter 所有权及输入非重放。不得按 Herdr 或其他应用身份增加例外。
- R4：恢复过程中保持最后完整历史画面，替换成功后内容与 chrome 一致；保持输入/paste fence、Main/Alternate、背景历史同步和 resize 行为。失败的输出/flush 不得推进呈现提交或释放保留输入。
- R5：回归必须区分这两个失败机制，检验目标收到的 ACK 和屏幕实际 child cells，而非仅检查状态变量、滚动条或帧计数。
- R6：遵循用户本轮明确约束，先调查真实故障完整因果链和两条连接表现差异，再收敛根本修复。对照同版本、同应用/输入和 viewport，记录更新产生、入队、UI 消费、resize、ACK 与服务端同步状态；区分已证实缺陷、真实故障归因和待验证假设。修复共享所有者与契约，禁止按本地/远程连接增加症状补丁；若证据推翻当前 D1，重新设计，不能为保留既定方案挑选证据。

## Acceptance Criteria

- [x] AC1 / R1,R5：确定性覆盖普通 delta 跨 resize 边界消费的时序，包括 deferred/physical/mode resize、服务端仍 Active 或已 Awaiting 两个分支；证明不会发送非法 snapshot ACK，也不会引发重复快照确认链。真实 CLI 运行默认持久模式 Herdr 在本地和实际远程均不再因此退出。
- [x] AC2 / R1,R3：合法 remote resume delta 和完整 snapshot 仍只在成功应用后精确确认；普通更新的两条 route 语义一致，错误/陈旧 ACK 仍按既有服务端合同拒绝。
- [x] AC3 / R2,R5：向上滚动再回到最新处，实际屏幕包含全部最新 child rows，检查发生在任何额外点击之前；覆盖两条 route 和有/无新输出。
- [x] AC4 / R3,R4：状态栏、滚动条、cursor/modes、Main/Alternate、背景历史浏览、输入/paste 恢复、resize 和失败提交边界保持正确；真正相同的目标帧不重复写出。
- [x] AC5 / R5：现有真实 CLI 外层 PTY 场景覆盖往返滚动和正确同步，并记录中立探针、真实 Herdr、受影响测试及最终质量门的实际结果和平台限制。
- [x] AC6 / R1,R6：记录真实 CLI 故障与中立回归之间的因果对应，用对照轨迹和受控交错解释连接表现差异；改变传输分批/调度时，同一同步契约仍正确。未复现或无法观测的差异必须明确保留为证据缺口，不能宣称已查清或根治。

## Decisions and Scope

- 用户已在因果调查和最终方案后明确批准实施 D1/D2；保持共享根因修复，不按连接模式打补丁。
- 全程仅主代理，禁止全部子代理，包括 Trellis 角色；清单仅供主代理读取。
- 当前候选方案在现有 client/view/UI 边界内修复，须经 R6 因果调查验证充分性；不引入新 crate、协议版本、第二套同步解释器、应用或 route 症状分支、服务端 ACK 放宽、固定延迟或每次无条件全量重绘。
- 新平台、额外性能评测、版本升级、发布和部署不属于本任务初始范围。
- 没有待用户决定的产品/兼容性歧义；真实两条连接的故障链已定位，用户原环境复测也确认远程失败。共享契约修复和本机质量门已完成，真实本地/远程 Herdr 验收正常；具体证据边界见 [validation.md](validation.md)。方案见 [design.md](design.md)，调查、执行和验收顺序见 [implement.md](implement.md)。

## Post-validation authorization

2026-09-05：用户明确授权“提交，走发布流程吧”。按已审阅批次提交修复，
随后归档、记录会话，通过正常 PR、CI、合并、版本准备和正式签名发布流程
发布下一个补丁版本。此前“无发布”仅描述初始实施范围，已被本次授权扩展。
仍不使用子代理，不更改保护规则，不操作用户现有 Session。
