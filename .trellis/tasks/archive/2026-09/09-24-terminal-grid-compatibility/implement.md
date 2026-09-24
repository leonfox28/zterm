# 基础控制与文字效果执行计划

## Order and dependency

查询回复子任务完成后开始；父设计评审通过并加载 trellis-before-dev。阅读 backend/frontend index、terminal-model、core-wire-domain、shared-client、Android app/frame preparation 和质量/错误处理指南。

- [x] 为 REP 与 CSI u 原缺陷添加有效行为和预算边界测试。
- [x] 实现最小 ingress/engine 改动；比较单 chunk/逐字节输入、charset、宽/组合字符和 reset。
- [x] 增加核心 style 与 schema 字段、校验/转换，更新快照/增量/历史投影及 checkpoint。
- [x] 接入 CLI SGR 与 Android native/render flags；核对行 hash 和 overlay。
- [x] 添加跨 wire、reconnect/history、conceal-copy 的行为测试及相关 specs。

## Validation

`cargo test -p zterm-terminal -p zterm-core -p zterm-proto --all-features`，以及受影响的 `zterm-client`/`zterm-cli`/`zterm-android` 测试；运行 source-policy 和 terminal-dependency-policy。执行 `just android-build`、`just android-check`；在选定的本地测试会话验证 strike/conceal、选择复制和光标遮盖。

本任务不宣布整个首轮完成；光标任务继续扩展 cursor DTO。REP 放大和 conceal overlay 是重点审查项。完整质量门禁及组合场景见父 implement。

Intermediate progress 2026-09-24 (superseded by final acceptance record below): REP/CSI u and strike/conceal implemented across model, wire, CLI and Android. Terminal suite 61 tests passed. Core/proto/client/CLI/Android tests running; Android build/pixel execution and final quality checks remain integration gates.

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
