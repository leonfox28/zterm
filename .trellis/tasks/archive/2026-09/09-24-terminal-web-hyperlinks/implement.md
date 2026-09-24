# 网页链接执行计划

基础文字与光标任务完成后开始。先加载 trellis-before-dev，读取 backend/frontend index、terminal-model、core-wire-domain、shared-client、Android app/frame preparation、logging/quality/error 指南。

- [x] 定义共享链接值、长度/数量/字节预算、OSC 8 URL/id 解析及错误 opener 行为。
- [x] 验证上游 hyperlink grid/template 保留和屏幕切换，实现配额回收；先测试 saved cursor、inactive screen、resize 与历史淘汰。
- [x] 扩展 cell projection、checkpoint、wire dictionary 与校验，测试长 URI 重复格子不会线性复制 URI 数据。
- [x] 将 link 纳入 row identity、历史/重连以及 Android cache 字节计算。
- [x] CLI 输出规范链接 span 并覆盖边界清理。
- [x] Android 增加 native source-bound 查询和系统菜单“打开链接”，保留 Copy 和普通鼠标路径。
- [x] 覆盖恶意/过量目标、坏 dictionary、stale frame、选区跨目标、浏览器失败；更新 specs。

运行 terminal/core/proto/client/cli/android 相关测试及 source/dependency policy；Android build/check。对实际外层终端和选定 Android fixture 分别验证显式操作、历史、重连、长按 Copy/TUI mouse。URI 不放入日志或测试失败的生产诊断中。完整质量门禁由父任务执行。

重点审查：inactive/saved template 配额、链接生命周期、dictionary 自包含、内存缓存计量。若必须改变 scope/交互才能满足合同，应先更新具体设计，不通过漏记内存或隐藏 fallback 掩盖问题。

Acceptance evidence and platform limits: [parent verification record](../09-22-terminal-protocol-audit/research/implementation-evidence.md). Implementation reviewed; user authorized commit and merge on 2026-09-24.
