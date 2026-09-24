# 光标呈现执行计划

基础控制/文字效果任务完成后开始；父评审通过且 trellis-before-dev 已加载。阅读 backend/frontend index、terminal-model、core-wire-domain、shared-client、terminal-colors 和 Android app/frame preparation。

- [x] 为六种形状/闪烁及 0、DEC 12/25/reset 建立投影与 wire 测试。
- [x] 加入 presentation 领域/wire/native DTO，修复 metadata-only 发布，更新 checkpoint。
- [x] 扩展 CLI canonical cursor 输出、自定义颜色 cursor 与退出清理。
- [x] 扩展 Android overlay 与 lifecycle timer；conceal 路径沿用前置任务规则。
- [x] 覆盖主/备用屏幕、snapshot/delta、重连、同步输出和 timer 停止行为；更新 specs。

验证 terminal/core/proto/client/cli/android 相关测试、source-policy、Android build/check；在本地 fixture/编辑器中观察三种形状及闪烁，切后台/隐藏光标确认停止绘制。运行证据记录外层终端支持条件。后续链接子任务以本任务的 renderer/native frame 为代码基线。

Intermediate progress 2026-09-24 (superseded by final acceptance record below): semantic cursor/wire/desktop/native rendering implemented. Rust lib tests passed (Android 21, CLI 105 plus 3 pre-existing ignored, proto 21, terminal unit 13); new cursor_protocol 2 passed. Android assembleDebug/lintDebug/testDebugUnitTest/compileDebugAndroidTestKotlin passed; subsequent pixel additions will be included in final Android gate. Desktop custom-color beam/underline use partial-block glyphs because ANSI cannot overlay a partial glyph; semantic text remains intact and is repaired when the caret moves/blinks. Runtime pixel/lifecycle evidence remains pending.

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
