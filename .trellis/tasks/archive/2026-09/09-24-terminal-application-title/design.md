# 应用标题设计

遵循父 [R4 设计](../09-22-terminal-protocol-audit/design.md#r4-application-title)。

- title 是 terminal 现有 Zterm-owned metadata，单一状态所有者；不是 session registry 的 name，也不是 host effect。core 保有有界有效文本字段。
- ingress OSC 0/2 更新状态；projection/snapshot/delta 包含 title，即使 rows/cursor 没变也可发布。DEC 2026 frozen checkpoint 同时冻结 title。
- title 清空用空值表达，不引入 optional-update 与 empty-title 混淆；history rows 不各自保存标题。reset 清空、screen transition 保留。
- proto 与 shared client 按现有全量元数据方式转换/应用，校验上限及控制字符，Debug 脱敏。
- CLI presenter sole writer 输出规范 OSC 2；TerminalGuard 首次接管 push、退出 pop title stack，失败清理与正常退出一致。无 stack 支持时不能承诺恢复外层旧标题，不查询 raw title。
- Android NativeFrame/TerminalFrames status 单向携带 title；TerminalScreen 组合已有会话名与应用标题，固定单行高度。生成 Kotlin 由既有 native build 产生。
- 不修改 session rename/storage/列表 API，无数据库迁移；同版本模型/协议/客户端整组更新回退。

Spec 更新 terminal-model、core-wire-domain、shared-client 与 Android app 的实际标题合同。

## Implementation classification

Missing metadata transport/presentation: retain title in the existing authoritative terminal state and propagate it through the established surface pipeline. No session-name storage changes.
