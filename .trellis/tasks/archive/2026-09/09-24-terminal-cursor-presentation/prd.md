# 传递并呈现终端光标形状与闪烁

## Goal and source requirement

落实父任务 [R2](../09-22-terminal-protocol-audit/prd.md)：编辑器模式切换时，两端能够显示应用声明的光标形状与闪烁。

## Confirmed facts

上游已解析 DECSCUSR，但语义 cursor 仅包含位置、可见性和文字 pen；CursorBlinkingChange 被丢弃。Android 与桌面自定义颜色路径均绘制固定块状光标。参见父 [研究](../09-22-terminal-protocol-audit/research/protocol-support.md)。

## Requirements and acceptance

- [x] DECSCUSR 1–6 对应 block/underline/beam 的 blinking/steady 组合，0 恢复默认常亮块状；DEC 12 修改及查询匹配实际状态。
- [x] DEC 25 隐藏、reset、主/备用屏幕切换、重连、metadata-only delta 后状态一致。
- [x] 自定义光标颜色不使形状/闪烁退化；conceal 文本在光标覆盖时仍隐藏。
- [x] 本地 blink 不制造 host revision/网络更新；Android 后台/不可见/detach 停止 tick，闪烁不重建全部行。
- [x] 桌面正常与错误退出执行既有终端状态清理，后续 shell 光标可正常使用。

## Boundaries and dependencies

依赖基础控制/文字效果作为模型、cursor style 和 conceal 渲染基线，串行执行。不实现文字 blink，不要求跨设备闪烁相位同步，不增加用户配置页。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
