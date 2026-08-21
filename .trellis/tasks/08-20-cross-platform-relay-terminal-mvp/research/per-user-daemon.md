# 每用户 daemon 与 direct install 分发边界

## 已确认的权限模型

每个操作系统用户运行自己的非特权 zterm daemon。远程 PTY 直接继承该用户的权限和环境，zterm 不提供系统账户认证、身份切换或 root/Administrator 权限代理。

用户进一步确认，权限边界不仅约束 daemon 的运行身份，也约束安装：官方 installer、`zterm setup`、`zterm update` 与卸载都不能请求管理员授权、`sudo`、root 密码、PolicyKit 管理员确认或 UAC 提升，也不能写系统级服务目录。

同一物理机器上的两个用户对应两个独立 zterm 设备：

- 各自生成长期设备密钥和 endpoint 身份。
- 各自维护授权设备、会话、配置、日志和缓存。
- 各自启动和控制自己的 daemon 进程。
- 互相不能查看、接管或撤销对方的会话与授权。

这一模型避免系统级远程 Shell daemon 必须处理本地用户认证、权限下降、跨用户文件访问和共享状态迁移，显著缩小首版安全边界。

## Direct install 分发约束

第一阶段通过官方 HTTPS installer 交付 CLI 与 daemon，产品核心是单一 Rust 原生可执行文件：

- 用户安装过程必须使用预编译的目标平台/架构产物，不要求 Cargo 或 Rust 编译环境。
- installer 只负责识别平台、下载、验证与原子放置原生二进制，不承载网络、认证或 PTY 业务逻辑。
- 程序、数据与任何后续服务注册必须落在当前 OS 用户的作用域，不能因为管理员另行预装二进制而共享运行时身份。
- 用户已确认不把 npm 作为第一阶段官方渠道；installer manifest、校验、签名、代理/离线安装和回滚进入技术设计与发布验收。

## 已确认的安装生命周期

安装与初始化严格分离：

1. 官方 installer 只提供 CLI、daemon 和目标平台预编译产物；不生成身份、不注册服务，也不启动进程。
2. 用户显式运行 `zterm setup`，确认设备名称、生成设备密钥、写入基础设施配置并 detached-spawn 当前用户的唯一 daemon；1.0 不注册开机或登录启动项。
3. `zterm setup` 重复运行时保留原设备身份和授权，除非用户另行执行带明确警告的身份重置操作。
4. 停止 daemon 与更新程序保留本机身份；官方 `zterm uninstall` 则是用户明确选择的身份销毁边界，在警告并确认活动 PTY 会结束、旧配对需重建后，删除 `~/.zterm/` 中的私钥、授权、known devices、配置与日志，再移除程序。普通重新安装因此不能复用旧 EndpointId。

这会比安装后自动可用多一步，但避免 bootstrap 脚本静默创建长期密钥或修改启动项，也使失败恢复和多用户隔离更容易解释。删除本机私钥不是中央全局撤销：若旧私钥曾被复制，仍需在每台曾授权该 EndpointId 的宿主上分别撤销。

## 1.0 平台映射

- Linux：`zterm setup` 或后续本地命令 detached-spawn daemon，不注册 systemd user service，不修改 lingering，也不承诺系统重启后自动上线。
- macOS：`zterm setup` 或后续本地命令 detached-spawn daemon，不注册 LaunchAgent、LaunchDaemon、Login Item 或 crontab，也不承诺系统重启后自动上线。
- Windows：第三阶段采用同样的每用户身份边界，具体用户后台启动机制在 Windows 设计阶段确定。

程序文件共享与 daemon 实例共享是两个不同问题。即使未来管理员另行预装一份二进制，每位用户仍必须拥有独立的初始化、密钥、配置、endpoint 和 daemon 进程；1.0 的官方入口则是每位用户分别运行非特权 direct installer。

## 手动启动后的持续运行

第一阶段的典型宿主是通过 SSH 管理的 Linux 服务器，因而“当前 SSH 会话还在”不能成为 daemon 或 PTY 的生命周期前提。

1.0 采用 Herdr 式路径：本地 CLI 发现 daemon 不存在时直接 detached-spawn 唯一 `zterm daemon`，不增加自研 `zterm-supervisor`、root daemon 或常驻代理。daemon 必须关闭对发起终端的 stdio 依赖并脱离控制终端，使关闭普通 Terminal 窗口或断开 SSH 不会产生 SIGHUP 连带退出。

不同 Linux 发行版的 logind/systemd 策略可能在最后一次登录退出时清理整个用户 cgroup；`setsid()` 不能对抗这种系统级清理。1.0 不为此修改 linger 或注册开机服务，doctor 应检测并披露已知限制。用户若在这类主机上需要无人值守能力，只能自行配置外部服务，直到后续版本正式交付受支持的启动机制。

以下 systemd/cron/launchd 调查保留为 1.0 之后的设计输入，不属于首版实现。

systemd 官方文档给出的机制是 user lingering：

- [`loginctl enable-linger`](https://www.freedesktop.org/software/systemd/man/252/loginctl.html) 会让指定用户的 user manager 在开机时启动，并在该用户全部注销后仍保留，官方明确将其用途描述为让未登录用户运行长生命周期服务。
- [`pam_systemd`](https://www.freedesktop.org/software/systemd/man/251/pam_systemd.html) 说明了默认登录生命周期：首次登录时创建 `user@.service`，最后一个会话退出时会终止该用户的 systemd 实例并移除 `/run/user/$UID`。发行版配置可能影响具体清理行为，因此不能仅凭一次实测侥幸存活就承诺可靠后台运行。
- 设置 linger 状态由 `org.freedesktop.login1.set-user-linger` PolicyKit 权限保护；当前用户能否直接启用取决于系统策略。zterm 不能假定命令一定无需提权成功，更不能在 setup 中静默调用 `sudo`。

若后续恢复开机启动需求，建议的 systemd 产品边界是：

1. `zterm setup` 在 systemd Linux 上安装并启用当前用户的 `zterm.service`，随后读取 `loginctl show-user <user> --property=Linger` 验证生命周期条件。
2. 若 `Linger=yes`，正常完成并明确报告 daemon 可在注销后及重启后运行。
3. 若 `Linger=no`，setup 不发起 PolicyKit 管理员授权，也不提示用户用 `sudo` 改变系统状态；先检测受支持的纯用户级冷启动后备。
4. zterm 绝不调用 `sudo`、写 `/var/lib/systemd/linger` 或修改系统级 PolicyKit/logind 配置。后备不可用时保留已生成的设备身份和用户服务，但明确标记该宿主未达到“无人值守可用”状态。
5. 非 systemd Linux 和 user-cron 后备的支持矩阵在技术设计阶段确定，不能伪装成已满足这一保证。

这些机制不能以扩大安装权限为代价；后续支持矩阵必须诚实反映宿主已有用户级机制是否足以达到目标。

## Herdr、Zedra 与 Paseo 的现状

两者都能让用户不必显式运行一个单独的 `server start` 流程，但必须区分“用户运行主命令时按需拉起 server”和“操作系统启动后、尚无人运行命令时 server 已经在线”。

### Herdr

- 普通 `herdr` 启动会先探测本地 socket；没有 server 时自动启动 `herdr server`，然后当前 TUI 作为 thin client attach。因此系统重启后，用户只需再次运行 `herdr`，无需另行执行 `herdr start` 或 `herdr server`。
- [`src/server/autodetect.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/server/autodetect.rs#L179-L218)把 server 的 stdin/stdout/stderr 重定向到 null，并注明用独立 session 使其在客户端退出后继续运行；[`src/platform/mod.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/platform/mod.rs#L72-L88)在 Linux/macOS 上通过 `setsid()` 实现。
- SSH remote bridge 发现远端 server 不存在时也调用同一 daemon spawn 路径；因此即使 Herdr server 已停止，用户仍可以把 SSH 当作外部 bootstrap 通道重新启动它。
- Herdr 自带的 curl 安装脚本只下载二进制并提示运行 `herdr`，不会注册或启动系统服务（[`website/install.sh`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/website/install.sh#L107-L131)）。不过 Homebrew 官方公式额外定义了 `herdr server` 的 `service do` 与 `keep_alive true`；用户显式执行 `brew services start herdr` 后，Homebrew 会用 launchd/systemd 管理它（[`herdr.rb`](https://github.com/Homebrew/homebrew-core/blob/315cbb1f46d2a130eca92bdef7b71ece27800050/Formula/h/herdr.rb#L34-L39)）。这是可选的系统服务路径，不是 Herdr 自己新增 supervisor。
- Herdr 文档也把裸 `herdr server` 定义为供 supervised/service-style setup 使用的入口（[`cli-reference.mdx`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/docs/next/website/src/content/docs/cli-reference.mdx#L84-L94)）。
- “机器重启后 session 回来”不表示原进程跨重启存活。Herdr 明确区分：普通 detach 时原进程继续；server 重启后原进程已经消失，只恢复布局/cwd，screen history 默认关闭，受支持的 Agent 可以依靠自身 session ID 重新 resume（[`session-state.mdx`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/docs/next/website/src/content/docs/session-state.mdx#L8-L50)）。

### Zedra

- README 在安装和 Agent hook setup 之后仍要求用户显式运行 `zedra start --detach`；Windows 示例同样如此。`zedra setup` 的职责是安装 AI Agent hooks，不是安装 daemon 服务（[`README.md`](/Users/huyuanzhe/projects/zedra/README.md:20)、[`main.rs`](/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/main.rs:193)）。
- Unix detach 实现把 stdin 设为 null、stdout/stderr 写入 workspace daemon log，并调用 `setsid()` 脱离 SSH 进程组和控制终端，以避免普通 logout SIGHUP（[`daemon_launch.rs`](/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/daemon_launch.rs:90)）。
- Unix 和 PowerShell 安装脚本都只下载/安装二进制并提示运行命令，没有注册或启动操作系统服务（[`install.sh`](/Users/huyuanzhe/projects/zedra/scripts/install.sh:169)、[`install.ps1`](/Users/huyuanzhe/projects/zedra/scripts/install.ps1:189)）。仓库中也没有 systemd unit、launchd plist、linger 检查或开机启动机制。因此 daemon 崩溃、系统重启，或 Linux logind 按策略清理整个登录 session 时，不存在 supervisor 自动恢复保证。
- Zedra 的 `HostWorkspaceOpen` 可以让一个已经在线并完成认证的 host 为另一个 workspace 执行 `zedra start --detach`（[`PROTOCOL_SPECS.md`](/Users/huyuanzhe/projects/zedra/docs/PROTOCOL_SPECS.md:460)）。这只是在线 host 提供的远程启动便利；机器重启后若没有任何 host 进程，它没有可接收 RPC 的入口，因而不能完成冷启动自举。

### Paseo

本次调查以 `getpaseo/paseo` 的 `5d7afd59ae35d78a145b5bc8d80391f385ae393b` 提交为准，并检查了完整 Git tree，而非只依赖本地副本或文档中的“自动启动”措辞。

- CLI 的 `paseo daemon start` 默认调用 `startLocalDaemonDetached`（[`start.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/cli/src/commands/daemon/start.ts#L43-L60)）；裸 `paseo` onboarding 在发现 daemon 未运行时也调用同一路径（[`onboard.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/cli/src/commands/onboard.ts#L357-L377)）。
- detached 路径只用 Node `spawn(..., { detached: true, stdio: ignore })` 加 `unref()` 启动 runner（[`local-daemon.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/cli/src/commands/daemon/local-daemon.ts#L579-L604)）。这能脱离当前 CLI，但不会注册操作系统冷启动项。
- runner 自身是 `Paseo Supervisor`，再启动 daemon worker，并设置 `restartOnCrash: true`（[`supervisor-entrypoint.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/server/scripts/supervisor-entrypoint.ts#L153-L181)）。因此 Paseo 实际是 supervisor + worker 两进程；它能在 supervisor 活着时恢复 worker 崩溃，却不能在 supervisor 也因整机重启而消失后自举。
- 桌面 App 的 React 启动流程只在 App 已经运行并 mount 后决定是否启动内置 daemon（[`_layout.tsx`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/app/src/app/_layout.tsx#L358-L380)），随后仍是 detached spawn（[`daemon-manager.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/desktop/src/daemon/daemon-manager.ts#L347-L414)）。默认设置甚至是退出桌面 App 时停止 daemon（[`desktop-settings.ts`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/desktop/src/settings/desktop-settings.ts#L43-L51)）。
- 完整仓库没有 `systemctl`、`enable-linger`、`launchctl`、LaunchAgent、LaunchDaemon、Electron `setLoginItemSettings` 或等价注册代码，打包配置也只有普通 DMG/ZIP/AppImage/DEB/RPM 等目标（[`electron-builder.yml`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/packages/desktop/electron-builder.yml#L38-L67)）。所以 Paseo 文档所说“桌面 App 自动启动 daemon”是“打开 App 后自动”，不是“无人登录的系统重启后自动”。
- Paseo 唯一明确提供冷启动恢复语义的部署示例是 Docker Compose 的 `restart: unless-stopped`（[`docker.md`](https://github.com/getpaseo/paseo/blob/5d7afd59ae35d78a145b5bc8d80391f385ae393b/public-docs/docker.md#L44-L66)）。容器内 daemon 以非 root `paseo` 用户运行，但开机拉起依赖宿主 Docker 服务，不是 direct installer 可直接复用的纯用户级机制。

因此 Paseo 可以借鉴的是“本地命令/App 自动拉起”和清晰的进程状态检查，不能作为“原生用户安装、无人登录冷启动”的实现证据；其内建 supervisor + worker 也与 zterm 首版坚持的单 daemon 进程边界不同。

### macOS launchd 边界

Apple 明确区分两类服务生命周期：

- [Launch Agent](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)在用户登录时启动，只在该登录会话期间运行，注销时会被终止。
- [Launch Daemon](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/DesigningDaemons.html)处于系统上下文，可以在没有用户登录时运行；全局安装的 daemon 配置必须由 root 拥有。LaunchDaemon 可配置为降权到指定用户运行实际 zterm 进程，但注册这份系统启动项本身仍是一次特权安装操作，不再是纯用户级 setup。
- 启用 FileVault 时，用户的登录凭据还承担[解锁加密启动盘](https://support.apple.com/guide/mac-help/protect-data-on-your-mac-with-filevault-mh11785/mac)的职责。在磁盘尚未解锁、操作系统和 launchd 尚未真正启动的阶段，任何 zterm 服务都无法提供网络入口；产品不能把“通电后、无人完成 FileVault 解锁”误称为可达。

用户已经排除 root-owned LaunchDaemon，因此 launchd 本身没有同时满足“纯用户注册”和“登录前运行”的路径。1.0 已整体延期自动启动；LaunchAgent 只作为未来登录后自动启动候选，不能用于宣称无人登录可达。

### 用户 crontab 的后续冷启动候选

macOS 26.6.2 的系统手册和 Apple OSS 源码显示，cron 虽然属于兼容机制，但仍是 Darwin 正式支持的组件：

- 每个用户拥有自己的 crontab，命令以 crontab 所有者身份执行；`HOME`/`LOGNAME` 来自该用户账户（[`crontab.5`](https://github.com/apple-oss-distributions/cron/blob/main/crontab/crontab.5#L26-L99)）。
- `@reboot` 的定义是“系统启动时运行一次”（[`crontab.5`](https://github.com/apple-oss-distributions/cron/blob/main/crontab/crontab.5#L230-L244)）。
- macOS 自带的 root-owned `com.vix.cron` 是操作系统组件，当 `/usr/lib/cron/tabs` 非空时由 launchd 拉起（[`cron.8`](https://github.com/apple-oss-distributions/cron/blob/main/cron/cron.8#L25-L48)、[`com.vix.cron.plist`](https://github.com/apple-oss-distributions/cron/blob/main/com.vix.cron.plist#L13-L23)）。普通用户通过 `/usr/bin/crontab` 管理自己的条目，不需要取得 root shell 或响应管理员授权；被启动的 zterm 仍是该用户进程。

后续候选注册形式是无损保留现有 crontab，并维护带版本标记的单条 `@reboot <稳定绝对路径>/zterm daemon --foreground`。cron 负责一次开机 spawn，不负责监督；因此手动 stop 后不会立即复活，daemon crash 后也不会自动恢复。若未来正式采用，必须验证：

1. macOS 支持版本和代表性 Linux 发行版上的真实无人登录重启。
2. FileVault 已解锁、系统实际完成启动这一前置条件。
3. cron 精简环境、网络尚未就绪时 Iroh 持续重试、日志重定向和稳定二进制路径。
4. setup 并发修改、重复执行、保留其他 cron 条目、卸载只移除 zterm 标记块。
5. macOS TCC/隐私目录对登录前用户进程的实际限制，并在产品中如实呈现。

Apple 手册同时指出 launchd 是更灵活的新式机制，所以 user cron 不能在未做上述验收前直接视为可靠结论。不过它是当前唯一同时符合“无管理员交互、进程保持用户权限、无人登录冷启动”的 macOS 原生候选。

### 对 zterm 的含义

`setsid()` 足以处理“关闭启动它的终端”和常见 SIGHUP，但进程仍可能留在原登录 session 的 systemd cgroup 中；它不能替代 user service、linger、崩溃重启和开机启动。

zterm 可以完全复刻 Herdr 的命令体验：任何本地 `zterm` 命令发现本用户 daemon 不存在时自动拉起它，不要求用户记住 `zterm start`。但这不能解决无人值守宿主的网络 bootstrap：Herdr remote 可以先通过 SSH 执行远端 `herdr`，zterm 控制端若只能经 zterm transport 建连，就无法命令一个尚未运行的远端 daemon 启动自己。

用户最终把开机自动启动整体延期到 1.0 之后。首版明确采用 Paseo/Herdr 式“运行本地命令时按需 detached-spawn”，并接受宿主重启后在首次本地运行 zterm 之前远端不可达；长期身份与授权仍持久化，因此重新拉起 daemon 后无需重新配对。systemd user + linger、LaunchAgent、LaunchDaemon、Login Item 与用户 `crontab @reboot` 都只作为后续研究，不进入 1.0 setup 或验收。
