# 补齐基础终端控制与文字效果

## Goal and source requirement

落实父任务 [R1](../09-22-terminal-protocol-audit/prd.md)：常见 TUI 的文字位置、重复字符、删除线与隐藏属性在两端一致。

## Confirmed facts

- ingress 明确拒绝 REP，普通 CSI u 被 Kitty-only 分支拦截；现有 ESC 7/8 工作正常。
- 上游 REP 直接重复 Handler.input，绕过 Zterm 的组合字符配额，不能简单放开。
- [projection.rs](../../../crates/terminal/src/projection.rs) 丢弃 strike/hidden 标志；core/wire/Android 尚无对应语义字段。源码定位见父 [研究](../09-22-terminal-protocol-audit/research/protocol-support.md)。

## Requirements and acceptance

- [x] REP 支持缺省/0 计数为 1、最大 4,096；超过上限整条拒绝。ASCII、宽字符、组合字符、跨 chunk 的重复遵守现有字符/会话预算。
- [x] 无前序字符、RIS、非法/截断 UTF-8 不错误重复历史字符，普通输入保持原行为。
- [x] CSI s/u 保存与恢复正确，Kitty set/push/pop/query 及现有 ESC 7/8 不回归。
- [x] SGR 9/29、8/28、0 的 strike/conceal 在桌面及 Android 显示正确；背景、网格位置和原始文本不被 conceal 改写。
- [x] 快照、delta、历史、resize、备用屏幕、重连保留属性；Android selection/Copy 保留原文语义，光标和选区覆盖不泄漏被隐藏字形。
- [x] 组合配额、wire 非法值、行缓存失效和同步输出回归有测试。

## Boundaries and dependencies

在查询回复任务之后串行实施，避免并发修改 ingress；两者无协议语义依赖。后续光标/链接/标题任务以本任务模型变更为基线。不增加文本闪烁、新键盘模式、Android OSC 52 或新的终端引擎。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
