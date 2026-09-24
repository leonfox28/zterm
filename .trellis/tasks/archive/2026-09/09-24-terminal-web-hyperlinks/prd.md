# 支持保留历史的 OSC 8 网页链接

## Goal and source requirement

落实父任务 [R3](../09-22-terminal-protocol-audit/prd.md)：命令输出的带标签网页链接可以在桌面和 Android 打开，滚入历史、断开重连后仍保留目标。

## Confirmed facts

OSC 8 当前被 ingress 拒绝；上游支持 cell hyperlink，但领域 cell、wire、Android 缓存及 native action 尚无对应字段。CLI 输出来自 semantic presenter，不能靠外层终端从原始 PTY 识别。参见父 [研究](../09-22-terminal-protocol-audit/research/protocol-support.md)。

## Requirements and acceptance

- [x] 支持 OSC 8 opener/closer、可选 id、合法绝对 HTTP/HTTPS 目标；未知参数不改变目标，非法 opener 不继承旧链接。
- [x] 链接覆盖 live/history、滚动淘汰、宽字符/换行、主/备用屏幕、保存光标及重连；覆盖或擦除 cell 后不可打开旧目标。
- [x] 桌面通过规范 OSC 8 及外层终端链接手势打开，输出边界/退出均关闭链接。
- [x] Android 长按链接后在系统操作菜单选择“打开链接”，由浏览器处理；普通 tap、TUI mouse、选择/Copy 保留。失效 source、重连和跨目标选择不误开。
- [x] 不自动访问目标，不打开 file/custom scheme，不写剪贴板；无浏览器时提供失败反馈。
- [x] 单条 OSC 1,024 字节、id 128 字节、终端 1,024 个不同链接身份/1 MiB URI-id 数据，以及现有 frame/history/Android cache 上限均可验证且能回收。

## Boundaries and dependencies

在基础样式、光标任务之后集成；复用其最终 cell/native frame/renderer，不并行改共享字段。远程文件打开、自动 URL 检测、下载预览、shell integration、浏览器内嵌与新的选择模型不在范围内。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
