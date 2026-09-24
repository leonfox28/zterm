# 查询回复设计

Root-cause classification: local protocol implementation gaps. The existing color owner and ingress reply collector already own ordered, bounded replies; OSC 21 serializes an incorrect unknown-field form and CSI 18t has no admitted branch. Both can be corrected at these owners without adding cross-layer state.

遵循父 [共享设计](../09-22-terminal-protocol-audit/design.md#r5-query-replies)。

- OSC 21 继续归现有 `ZtermColorState` owner；仅修改 unknown-field 序列化，区分未知字段与已知但无可报告颜色值，不能把两者合并。
- 复用现有 Base64 依赖的无 padding 编码；回复仍经 UpdateCollector 和既有字节预算，不增加旁路 PTY 写入。
- ingress 只识别精确 `18t`，从 engine.size() 取 row/column 生成规范回复。未执行的 `8;rows;cols t` resize event 不改变报告值。
- 不改 session service、wire、Android API；无迁移。修改 `terminal-colors.md` 旧 unknown 合同及 `terminal-model.md` 支持列表。
- 回退 colors/ingress/tests/spec 为同一组；不改变普通颜色设置行为。

参考：[Kitty color query](https://sw.kovidgoyal.net/kitty/color-stack/#querying-current-color-values)、[xterm controls](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)。
