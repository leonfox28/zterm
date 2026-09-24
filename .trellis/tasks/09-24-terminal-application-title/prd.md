# 传递并显示 OSC 0/2 应用标题

## Goal and source requirement

落实父任务 [R4](../09-22-terminal-protocol-audit/prd.md)：会话页能显示当前运行应用设置的标题，重连后恢复，并保留用户手工会话名。

## Confirmed facts

OSC 0/2 当前仅产生 TitleChanged side event；daemon 不交付该队列，snapshot/delta 不含标题。Android 主标题使用 session/host 名称。参见父 [研究](../09-22-terminal-protocol-audit/research/protocol-support.md)。

## Requirements and acceptance

- [x] OSC 0/2 更新应用标题，最大 256 UTF-8 字节、按字符边界截断且去除控制字符；空值/RIS 清空，屏幕切换保留。
- [x] title-only 变更也发布，snapshot/delta/DEC 2026/reconnect 一致，不依赖瞬时事件重放。
- [x] 桌面外层标题变化，退出执行 title stack 恢复；空值使用默认 zterm 标签。
- [x] Android 显示“会话名 · 应用标题”，空/重复标题省略，单行省略号，不挤占已有状态副标题。
- [x] 应用输出不能改写持久 session name 或会话列表；手工重命名仍单独生效。
- [x] 非法/截断 UTF-8、超长标题、多 chunk/终止方式及标题清理有回归覆盖，title 不泄入日志。

## Boundaries and dependencies

在查询、样式、光标、链接任务之后串行集成，使用最终 surface/delta/native frame 结构；没有标题对链接的协议依赖。不添加 icon、后台列表标题、通知、子应用窗口操作或数据库字段。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
