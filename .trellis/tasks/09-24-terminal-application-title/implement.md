# 应用标题执行计划

以之前四个子任务的最终共享结构为基线；父设计评审通过后按 trellis-before-dev 阅读 backend/frontend index、terminal-model、core-wire-domain、shared-client、Android app/frame preparation 及 logging/quality 指南。

- [x] 定义 title 领域上限与净化规则，新增 OSC 0/2、empty/RIS、Unicode 边界测试。
- [x] 将 title 接入权威状态、projection、checkpoint、snapshot/delta，先验证 title-only 与同步输出 hold/reconnect。
- [x] 补齐 proto validation、共享客户端和 NativeFrame/status 传递，维持敏感字段 Debug 脱敏。
- [x] 接入 CLI 标题输出与 guard stack 清理；Android 标题组合与省略号显示。
- [x] 测试持久会话名不变、手工 rename 与 app title 独立、退出清理；更新 specs。

运行 terminal/core/proto/client/cli/android 相关测试与受影响 daemon 快照/重连测试；source-policy、Android build/check。实际外层终端验证退出恢复条件，Android fixture 验证中文长标题、清空、重连与手工名称。

完成后回到父任务做五项组合验收与 `just check`。设备/模拟器运行缺失必须明确记录，不以编译通过代替 UI 证据；本任务完成不自动意味着父任务验收完成。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
