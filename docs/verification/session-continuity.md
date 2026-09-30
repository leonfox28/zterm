# 会话连续性验收记录

日期：2026-09-30。开发主机：macOS arm64。按用户要求使用 Android 模拟器验证：
API 36、arm64、独立只读 AVD 实例 `emulator-5580`，仅安装 debug 包和 instrumentation 包。
主机使用 `presentation_fixture`，所有身份、配对、Session 和运行目录位于
`/tmp/zterm-continuity-host-0930`。未修改真实 `~/.zterm`、用户登录项或真实会话。
复现方法见 [端到端测试说明](../../tests/e2e/README.md)。

## 连接可靠性复核

- `remote_unary` 对写入后模糊失败最多进行一次相同请求字节的重试，保留原 deadline。
- `unary` 收到 `operation_outcome_unknown` 后只为下一次显式操作更新 lease，
  不把当前操作换 lease 后重发。操作去重规则保持不变。
- Android 只在成功取得 live list 后处理过期记忆；授权、网络失败不授权创建默认 Session。
  排队输入仍绑定 attachment、native input epoch 和 upload epoch。
- 新增的进程恢复路径只 attach 保存的原 Session ID，不调用默认创建；占用不自动接管，
  已结束时 Retry 仍只尝试原 ID。主动进入主机、选择或创建会话才进入显式操作流程。

本轮确实观察到一次冷连接 `operation_outcome_unknown`：默认连接的变更请求在
5 秒窗口内未确认结果，主机已创建一个 `main`，客户端报错后该 Session 仍存在。
随后显式重新进入主机复用了这个唯一 Session，未重复创建。本轮没有把这次现象
解释为已消失，也没有通过放宽去重或自动重放掩盖它。首次连接延迟和结果确认路径仍需
进一步定位；网络夹具准备中也出现过 `transport_unavailable` / `deadline_exceeded`。
下面的断网测试在连接已经建立后开始，不作为冷连接成功率的证据。

模拟器还复现了可确定修复的接管竞争：显式接管到达时旧控制端已 detach，新的
attachment 已取得空闲 lease；snapshot ACK 后再提交 takeover 被错误拒绝为
`not_synchronized`。现在先验证 principal 和同步状态，再提交无副作用的成功结果，
保留 operation key。新增
`explicit_takeover_of_a_vacant_session_commits_once_after_snapshot`
覆盖 ACK 前拒绝、ACK 后成功、同一 operation 重试和后续输入。

## 原生与构建检查

- `just check` 通过：格式、策略、Workspace Clippy（warnings denied）、完整 Rust 测试、
  文档构建、依赖审计、Relay 静态检查和上游制品校验。
- 其中 `local_ipc`、`authorization`、`controller_lease`、`principal_detach`、
  `session_concurrency`、`terminal_recovery` 覆盖授权、去重和会话连续性。
- 自启动测试覆盖 macOS/Linux 配置格式、默认关闭、重复 enable/disable、异常配置、
  外来文件保护、setup 前置条件、只读 status、升级路径稳定性和 reset 清理。
- `single_instance` 覆盖隐藏前台入口保留进程组、与手动启动复用同一 daemon、
  登录入口零 Session，以及 stop 后退出。
- Android `assembleDebug`、`assembleDebugAndroidTest`、`lintDebug`、
  `testDebugUnitTest` 通过。新增 JVM 用例覆盖后台服务策略与默认值。
- 单次 daemon lib 并行测试中，既有 pairing 测试的日志计数断言曾得到 0 而非 1；
  该用例单独重跑及随后完整 `just check` 均通过。未将首次失败从记录中删除。

## 模拟器执行结果

| 范围 | 实际执行 | 结果 |
| --- | --- | --- |
| 本地兼容与 UI | IdentityStore、TerminalInputConnection、AttachGeometry、TerminalRendering、TerminalFrames、ConnectionStatusUi、SettingsUi | 初轮 43 项中 39 项通过，4 项因专用夹具开关未开启而跳过 |
| 新设置卡片 | 实际点击后台开关，检查保存文件、服务列表和设置截图 | 通过；默认关闭、切换保存；无终端时不启服务，拒绝通知时显示不可见 |
| 通知允许 | `connectionSurvivesActivityAndServiceLifecycle`，启动测试进程前 grant 通知权限 | 通过；连接通知实际存在，后台新启服务被阻止，返回前台后启动 |
| 通知拒绝 | 同一流程在独立测试进程启动前 revoke 权限 | 通过；通知不可见、服务正常、无虚假的启动失败 |
| 后台生命周期 | 切到后台、锁屏/解锁、Activity 重建、开关关闭再开启 | 通过；原 Session 延续，关开关不切断前台终端 |
| 中文输入 | 真实 TerminalView InputConnection 的组合文本 `nihao` → 提交 `你好` → finishComposition | 通过；远端输出只出现一次预期中文结果 |
| 通知断开 | 发送通知 Disconnect action，检查本地恢复记录和远端列表 | 通过；回首页、停止服务、清除恢复记录，唯一远端 Session 继续存在 |
| 控制权与结束 | 第二个 attachment 接管，再 detach；原客户端显式接管；关闭远端 Session | 通过；失控/结束停止服务；空闲 lease 的显式接管成功 |
| 进程恢复 | prepare 后外部 force-stop，再分进程运行 resume / occupied / ended；ended 前由主机关闭原 Session | 四阶段通过；只尝试原 ID，占用不接管，结束不创建，Retry 仍不创建 |
| 真实终端往返 | `NativeTerminalTest`，使用隔离主机 | 通过；Unicode、历史补页、跨屏复制、A-B-A resize、输入顺序、重命名和 detach |
| 连接期间测量 | 独立运行 `measurementsDuringAttachReachTheNewTerminal`，开启隔离主机夹具 | 通过；连接期间连续测量后，远端实际 PTY 尺寸为最新的 31×67 |
| 断网与原 shell | 隔离模拟器按测试应用 UID 丢弃 IPv4/IPv6 出站流量，再恢复 | 通过；进入 reconnecting、拒绝断线输入、保留已完成画面；恢复原 Session 和 shell 变量，没有输入重放 |
| 键盘完整动画 | `productionLayoutReportsTheTargetBeforeTheSystemImeFinishes`，主屏和备用屏各开合一次并录屏 | 通过；提前提交目标尺寸，终态尺寸一致，查看录屏中的连续网格 |

初轮跳过的项目是 attach 期间测量的主机夹具，以及设置中的通知权限、导出 picker、
存储失败注入三项专用夹具。attach 测量随后单独运行并通过，余下三项仍记为跳过；
新后台通知允许/拒绝用例是独立执行的。
Android 撤销通知权限会结束正在运行的应用进程，因此权限矩阵在 instrumentation
启动前配置；测试进程因撤权退出不被算作前台服务错误。

旧断网用例未解析按需 frame source，修正后才能检查真实终端文本。单向丢包允许
在途数据继续进入，原协调器的 52 秒等待曾先于连接失效检测超时；只延长了测试
等待上限，未改变产品超时。最终包含 shell 变量连续性的用例用时 77 秒，通过后
确认测试添加的全部 IPv4/IPv6 规则已移除。另一次较早的断网流程用时 65 秒并通过。

本机原始日志保存在 `/tmp/zterm-*.log`，通过的关键日志另复制到
`target/verification/2026-09-30/`。验收录屏及抽帧图位于
`target/verification/2026-09-30/android-ime-open-close.mp4` 和
`android-ime-contact-sheet.png`，新设置截图为 `android-background-settings.png`。
这些本机构建产物未纳入 Git。

## 尚未关闭的发布验收

| 范围 | 剩余判据 |
| --- | --- |
| 首次连接 | 定位本轮冷连接超时；新配对、首次创建和重试均保留可核验的唯一 Session 证据 |
| 真实网络 | 跨物理网络 Direct、强制 Relay 回退、Wi-Fi/蜂窝切换；模拟器流量切断不替代此项 |
| 授权撤销 | Rust 回归已通过；撤销真实 Android 客户端授权的端到端验收尚未执行 |
| 设备和显示 | 手机节电策略、真实中文键盘和桌面交互 resize；本轮只使用模拟器 |
| 登录自启动 | 隔离账户上实际 macOS/Linux 登录/注销、并发手动启动、disable 不结束会话、stop 不拉起 |
| 更新清理 | 正式签名安装包的远程会话内升级、版本/配对/自启动选择保留；实际 manager 的 reset/uninstall 无残留 |

没有发布或安装正式签名包。以上项目通过前，路线图保留「已实现待验收」。
