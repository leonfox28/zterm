# 修正 OSC 21 与字符尺寸查询回复

## Goal and source requirement

落实父任务 [R5](../09-22-terminal-protocol-audit/prd.md)：应用收到符合协议的颜色查询回复和实际字符尺寸。

## Confirmed facts

- [colors.rs](../../../crates/terminal/src/colors.rs) 与现有测试把未知 OSC 21 字段回复为原字段加 `=?`；Kitty 要求 `unknown=` 加无填充 Base64 字段名。
- [ingress.rs](../../../crates/terminal/src/ingress.rs) 的窗口操作分支未实现 `CSI 18t`；engine 已持有权威 size。
- 依据与旧行为测试详见 [研究](../09-22-terminal-protocol-audit/research/protocol-support.md)。

## Requirements and acceptance

- [x] 单个、多个以及混合已知/未知颜色查询按规范和请求顺序回复；未知名使用正确的 UTF-8 字节和无填充 Base64。
- [x] `CSI 18t` 返回 `CSI 8;rows;columns t`，resize 后使用当前尺寸，保持 BEL/ST 分帧和 reply budget。
- [x] 错误参数、超长输入、截断/分块输入不引入宽泛窗口命令准入。
- [x] 现有已知颜色值/不可报告值、颜色栈、同步输出期间查询、Primary DA/DSR/CPR 回归通过。

## Boundaries and dependencies

首个实现子任务，无功能依赖；后续基础控制任务以此为代码基线。只实现 R5，不添加像素报告、窗口 resize 执行、DA2/DA3 或其他 DECRQSS。无需 wire 变更。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
