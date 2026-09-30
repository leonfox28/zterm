# 会话连续性验收记录

日期：2026-09-30。当前开发主机：macOS arm64。所有自动化状态使用测试临时目录，
不得修改真实 `~/.zterm` 或用户会话。初始 ADB 无连接设备；随后按用户要求启动独立的只读 Android 验收模拟器。

## 连接可靠性复核

- `crates/client/src/remote_unary.rs` 只对写入后模糊失败做一次完全相同字节重试。
- `crates/client/src/unary.rs` 收到 `operation_outcome_unknown` 后只为下一次显式
  操作更新 lease，不把当前操作重发到新 lease。daemon 重启后的旧 lease 拒绝属于
  预期安全行为，不能将所有该错误直接当作首次连接缺陷。
- `crates/daemon/tests/local_ipc.rs` 覆盖请求字节/操作 ID 相同及未知结果的 lease 更新。
  `authorization`、`controller_lease`、`principal_detach` 覆盖授权和控制权。
- Android `AppRepository` 只在成功取得 live list 后处理过期记忆；授权、网络失败
  不授权创建默认 Session。排队输入同时绑定 attachment、native input epoch 和 upload epoch。
- `TerminalInputConnectionTest`、`AttachGeometryTest`、`TerminalRenderingTest` 和
  `TerminalFramesTest` 提供 IME、几何及历史展示回归，仍需设备执行。

未取得历史首次连接 `operation_outcome_unknown` 的复现证据，不据此宣称已在真机消失。
模拟器后台测试复现了另一项连接边界：显式接管请求到达时旧控制端已 detach，
新 attachment 已取得空闲 lease；snapshot ACK 后再次提交 takeover 被错误拒绝为
`not_synchronized`。修复为在验证 principal 和同步状态后提交无副作用的成功结果，
保留 operation key 供原去重机制使用。`explicit_takeover_of_a_vacant_session_commits_once_after_snapshot`
覆盖 ACK 前拒绝、ACK 后成功及同一 operation 的精确重试。

## 自动化执行

- `cargo test -p zterm-client -p zterm-android --lib`：通过（Android 22；client 86，另 1 项显式忽略）。
- `cargo test -p zterm-daemon --test local_ipc --test authorization --test controller_lease --test principal_detach --test session_concurrency --test terminal_recovery`：通过。
- 自启动：平台两种配置格式、幂等、默认关闭、异常配置/外来文件保护，以及 CLI setup 前置条件、只读状态、reset 清理测试通过。
- `single_instance`：服务管理器前台入口保留进程组、与手动启动复用同一 daemon、零 Session、stop 后退出通过。
- Workspace Clippy（all targets/features，warnings denied）通过。
- 后续 Android 新功能检查在对应实现后补入。临时目录测试不代表实际登录验收。

## 必须另行执行的现场验收

| 范围 | 操作与判据 | 结果 |
| --- | --- | --- |
| 首次连接 | 新配对、首个显式 create；无重复 Session，无授权异常 | 未执行：需隔离主机和设备 |
| 网络 | Direct、强制 Relay、断网、Wi-Fi/蜂窝切换；原 Session/PTY 延续，断线输入不重放 | 未执行：需真实网络 |
| 授权 | 接管、撤销；旧客户端输入失效，后台服务停止 | 未执行：需设备 |
| 显示输入 | 全程记录键盘打开/关闭、中文组合与提交、历史补页、桌面 resize | 未执行：需模拟器及真机 |
| 登录自启动 | 实际 macOS/Linux 登录；并发手动启动只有一个 daemon；disable 不终止会话；stop 不拉起 | 未执行：需隔离账户/测试主机 |
| Android 后台 | 切换应用、锁屏、Activity 重建、进程终止；只恢复原 Session；结束/占用显示明确操作 | 未执行：需模拟器及真机 |
| 通知 | 拒绝权限仍可连接；通知可见状态真实；通知断开只 detach | 未执行：需设备 |
| 更新清理 | 正式签名包远程升级后手动重连；版本/配对/自启动选择保留；reset/uninstall 无注册残留 | 未执行：需正式包和隔离主机 |

以上现场项目通过前不将路线图标为「已完成」，不以本地构建替代发布验收。
