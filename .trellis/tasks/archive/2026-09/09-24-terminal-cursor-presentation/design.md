# 光标呈现设计

遵循父 [R2 设计](../09-22-terminal-protocol-audit/design.md#r2-cursor-presentation)。

- core 增加独立 TerminalCursorPresentation（shape enum、blinking bool），cursor 的现有 style 继续表示文字 pen。读取上游 cursor_style，不能依赖 transient event 作为恢复源。
- projection、checkpoint、snapshot/delta、proto 转换、shared client 和 composition/native frame 完整传递；非法 shape 拒绝，不用任意未知值回退掩盖 wire 错误。
- 无软件 cursor 的 CLI 输出 DECSCUSR；现有自定义颜色软件 cursor 扩展三种形状并加入 actor 管理的局部定时重绘。光标不可见/退出取消 timer，保持单一 presenter 写端。
- Android 使用动态 overlay 和已有 UI 生命周期管理 timer，shape/颜色变更触发光标范围失效，不污染 immutable row RenderNode cache。
- blink phase 是客户端短期显示状态；shape/blinking/visible 是权威语义状态，DEC 2026 hold 与 snapshot 均遵循同一发布边界。
- CLI guard 负责自身接管的物理状态清理，不引入终端查询等待或新远程状态通道。

同版本部署；schema/producer/consumer 整组回退。Spec 修改限 terminal-model、core-wire-domain、Android app/frame preparation 的光标合同。

Root-cause classification: cross-layer semantic boundary gap. The host engine owns cursor shape/blinking, but public cursor DTOs discard them. Preserve declaration state through all existing revisioned paths; local blink phase stays in presenters.
