# 执行与验收计划

## Current state and activation gate

- [x] 保留源码证据与 54 项基线 terminal 测试结果。
- [x] 切换到 `feat/terminal-protocol-compatibility`，保留全部原有未提交改动。
- [x] 创建五个独立验收的子任务，完成 PRD、共享设计与执行计划。
- [x] 具体设计已呈现；用户于 2026-09-24 回复“开始”，最终评审通过。
- [x] 实现前加载 `trellis-before-dev`，阅读当前子任务、相关 spec/checklist 及共享思考指南，然后激活该子任务。

采用 inline 串行执行；不派发子代理。空的 JSONL 是脚手架，本路径按 workflow 的 inline 规则读取上下文，不为通过检查填入虚假条目。父任务负责整合，不把任务树误当成依赖调度器。

## Ordered child tasks

| 顺序 | 子任务 | 依赖与完成边界 |
| --- | --- | --- |
| 1 | [查询回复](../09-24-terminal-query-replies/implement.md) | 无功能依赖，不改 wire；精确回复及资源测试通过。 |
| 2 | [基础控制/样式](../09-24-terminal-grid-compatibility/implement.md) | 不依赖查询语义；安排在其后避免同时改 ingress。完成 model/wire/两端样式及 REP 资源边界。 |
| 3 | [光标呈现](../09-24-terminal-cursor-presentation/implement.md) | 以基础样式变更为代码基线，共用 cursor/style DTO 与隐藏字形规则；完成软件光标与局部 timer。 |
| 4 | [网页链接](../09-24-terminal-web-hyperlinks/implement.md) | 以基础样式/光标为代码基线，共用 row、wire 和 renderer；完成历史、资源回收及系统菜单操作。 |
| 5 | [应用标题](../09-24-terminal-application-title/implement.md) | 复用前面最终 surface/delta/native frame 结构；完成 title-only 发布与两端显示。 |

这里后四项是明确的串行集成顺序，不表示标题协议在语义上依赖超链接。每项改动保持可单独理解、可单独测试；不在未经请求时 commit/push。

## Validation

每项先运行针对改变行为的测试，不为文档或可逆小改动制造镜像测试。新增 meaningful regressions 覆盖原缺陷、边界与跨层丢失风险；保留现有 corpus 的普通终端语义。

```sh
cargo test -p zterm-terminal --all-features
cargo test -p zterm-core -p zterm-proto --all-features
cargo test -p zterm-client -p zterm-cli -p zterm-daemon -p zterm-android --all-features
sh tests/source-policy.sh
sh tests/terminal-dependency-policy.sh
just android-build
just android-check
just check
```

按实际改动选择 focused package/case；`just check` 是最终宿主权威门禁，无改动、失败或未解决疑点时不重复全套。Android 实机/模拟器使用明确选定的目标与本地 fixture，运行相关 instrumentation；不得仅因存在设备就操作用户的生产配对会话。记录设备、平台、命令和结果，环境缺失单独报告。

## Integration acceptance

- [x] 主/备用屏幕、保存/恢复光标、宽字符、组合字符、滚动/历史淘汰及 resize 正常。
- [x] 同时输出 strike/conceal、cursor shape、OSC 8 与 title；snapshot 与连续 delta 到达同一状态。
- [x] DEC 2026 内不得提前显示标题/光标/链接，释放后整体一致；输入回复仍走现有及时回复路径。
- [x] 离开并重连可恢复当前标题/光标/文字样式与历史链接；无原始 OSC 透传，host effects 没有扩展到 Android 剪贴板。
- [x] Android 长按选择/Copy、链接打开、TUI 鼠标点击、后台停止光标闪烁，桌面链接关闭/退出清理有运行证据。
- [x] 更新 owning specs：terminal-model、terminal-colors、core-wire-domain、shared-client 和 Android app/frame preparation 中实际变化的合同。
- [x] 父 PRD 验收逐项关联证据；记录平台或外层终端限制。用户已于 2026-09-24 授权提交并合并，质量通过后执行收尾归档。

## Risk and rollback points

REP 不得绕过字符预算；hyperlink 不得按 cell 复制长 URI 或漏算 inactive/saved cursor；光标 blink 不得制造网络 revision；metadata-only 更新不得被 row equality 丢弃。重点审查上述四处及 wire 校验。按子任务整组回退模型、schema、生产者与消费者，不破坏已有未提交文档；没有数据库或用户数据迁移。

Acceptance evidence and platform limits: [research/implementation-evidence.md](research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
