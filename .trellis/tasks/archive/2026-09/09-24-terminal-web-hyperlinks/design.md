# 网页链接设计

以父 [R3 设计](../09-22-terminal-protocol-audit/design.md#r3-osc-8-web-hyperlinks) 为配额、wire 和 UX 的权威定义。

## Contracts and ownership

- ingress 在既有 OSC 长度上限内解析、标准 URL 校验后送入上游 hyperlink handler。只接收 HTTP/HTTPS；输出规范 URI，id 单独保存，不经 shell。
- engine 借助上游 cell hyperlink 保留网格语义；按两个屏幕、scrollback、当前与保存 cursor template 计数和回收有界链接身份/字节。inactive screen 使用切换时保存的保守统计，不访问私有字段或 fork 上游。
- 领域 cell 持共享不可变 link value；投影不按 cell 深拷贝 URI。消息级 dictionary/引用避免长链接按每格放大帧；snapshot/delta/history 每条自包含，decode 验证非法引用、重复/过量项及总字节。
- core/link redacted Debug，row Eq/hash 包括 link，Android 页面预算计入 link allocation；不因新字段绕开现有 16 MiB cache/selection 预留。
- Android native lookup 使用实际绘制 frame source 与坐标，沿用 stale source/geometry 防护。系统菜单动作只在明确有效目标上出现；系统 ACTION_VIEW 再验证 HTTP/HTTPS。
- CLI presenter 给文本 span 输出 OSC 8 opener/closer，在 row/chrome/error/exit 边界关闭；不能输出未经过准入的原始 OSC。

## Failure, compatibility and rollback

不合法/超配额 opener 关闭当前链接但不丢后续可见文字。字典校验失败沿用现有坏帧/重同步路径，不产生部分可交互页面。配额压力下回收已不被 grid/template 引用的目标，不能把历史累计计数当作活跃使用量。

同版本 model/schema/clients 成套更新及回退；无数据库迁移。遵循 terminal-model、core-wire-domain、shared-client 与 Android frame preparation 的修订合同。

Root-cause classification: cross-layer semantic boundary gap. Existing upstream cells can retain links, but ingress rejects OSC 8 and Zterm DTOs/cache/actions discard its target. Shared bounded values and self-contained wire dictionaries provide one complete contract across live/history/reconnect and both consumers.

Implementation refinement: Android resolves the already source-pinned selection instead of introducing an independent link-at-point source lifetime. This reuses existing selection admission, retained page accounting and wide-cell normalization; click delivery is additionally fenced by attachment epoch and selection version.
