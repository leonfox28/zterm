# 基础控制与文字效果设计

遵循父 [R1 设计](../09-22-terminal-protocol-audit/design.md#r1-basic-controls-and-text-effects) 和共享 wire 合同。

## Ownership and data flow

- ingress 区分普通 CSI u 与 Kitty 形式；REP 复用当前 UTF-8 分帧及字符准入路径，记录实际准入的最后字符，不镜像 grid。
- 重复字符仍送入同一个 Alacritty engine，当前字符集/光标/换行语义由上游负责。非法 UTF-8 清除重复记录，普通字节处理不做无关重构。
- projection 增加 strike/conceal；core 的 TerminalStyle、proto style、客户端组合帧和 Android native style 都携带这两个字段。
- ANSI presenter 使用规范 SGR 8/28、9/29，reset/baseline 不泄漏属性。Android attributes 使用独立 flag，隐藏时不画字形或其装饰，背景仍绘制。
- 不改变选择文本提取算法；修正 cursor/selection overlay 绘字路径，防止 conceal 时重新显示原文。
- 新字段进入 row equality/hash 与历史 DTO，更新内部 checkpoint format。生成绑定走既有脚本，不手工维护第二份类型。

## Tradeoffs and rollback

单条 REP 限额限制放大工作量，保留连续多个合法 REP 的正常终端行为；这是有界兼容子集。重复入口重构只为恢复原字符预算，必须用 chunking/非法输入/字符集回归证明普通文本路径没变。

core/proto/terminal/两端同组更新及回退；不支持中间 wire 版本混用。需同步 terminal-model、core-wire-domain 与 Android frame preparation 中变化的合同。

Root-cause classification: ingress admission gaps for REP/plain CSI u, plus a cross-layer semantic boundary gap for strike/conceal. The existing engine already owns these attributes, but every public DTO omits them; the fix must extend the shared contract and both renderers together.
