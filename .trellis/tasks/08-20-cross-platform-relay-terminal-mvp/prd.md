# zterm 跨平台中继终端：第零与第一阶段 PRD

> **2026-08-21 后续决策（覆盖本文旧的默认 Relay 描述）：** 第一阶段产品默认使用 Iroh 1.0.3 官方 n0 生产 Relay、官方 QAD 与生产 DNS/Pkarr；Phase Zero 已完成的 `relay.zenithconsulting.cn` 镜像和部署保留为可选自建能力，不加入默认 Relay map。具体实现与当前网络证据以子任务 `08-21-foundation-gate` 和 `.trellis/spec/backend/relay-deployment.md` 为准。

## 1. 目标

zterm 是一个面向长时间远程终端任务的跨网络连接工具。用户可以在远端 Linux 或 macOS 主机上运行 Codex、OpenCode、tmux、Herdr 或其他交互式程序；控制端断网、退出或切换网络后，远端 PTY 与其中的进程仍继续运行，重新连接后回到同一个会话并恢复当前终端画面。用户从手机回到项目宿主电脑时，也能通过本机 daemon 接续完全相同的 session，而不依赖第三方终端复用器。

网络层使用 Iroh：优先建立端到端加密的直连路径，直连不可用时通过项目默认或用户自建 relay 转发密文。云端不保存终端内容，也不成为设备授权真相源。

本文先以第零阶段建立本地开发环境并把固定版本的上游 Iroh relay 部署到用户提供的公网服务器，再进入第一阶段 macOS 与主流 glibc Linux 的 CLI 技术 MVP，两者均支持 x86_64/arm64。文中“1.0”表示通用远程终端的首个稳定产品边界；Android、Windows、桌面 GUI 和 iOS 按路线图后续交付，但第一阶段的数据模型与协议不得阻塞它们。

## 2. 项目背景与边界

- 当前 `/Users/huyuanzhe/projects/zterm` 是全新的产品仓库，不迁移或兼容已更名为 `zterm_old` 的旧 Electron/SSH 项目。
- `/Users/huyuanzhe/projects/zedra`、Herdr 和 Paseo 只作为实现证据与风险参考，不决定本项目的产品模型。
- zterm 不是一次性远程 Shell：network connection 是可重建的 transport，宿主 daemon 持有的 PTY session 才是持续存在的工作单元。
- 2.0 之前只交付工具无关的远程终端，不识别或解析任何专有 AI Agent。

## 3. 平台角色与路线图

0. 第零阶段：完成当前开发机的 Rust/Docker/质量工具环境、空 workspace 与固定上游 `iroh-relay` 的 Docker/Compose 部署物；本地验证后设置人工检查点，等待用户提供公网服务器连接方式，再完成单区域默认 relay 公网部署与一次真实握手验收。
1. 第一阶段：macOS x86_64/arm64 与主流 glibc Linux x86_64/arm64 的 Rust daemon + CLI，并使用第零阶段已经部署的 relay 完成 direct/relay 网络验收。
2. 第二阶段：Android 控制端 App；不托管 Android 本机通用 Shell。
3. 第三阶段：Windows daemon + CLI；桌面端既可托管也可控制。
4. 第四阶段：同时覆盖 macOS、Linux、Windows 的完整桌面 GUI 控制客户端；无界面 daemon 与 CLI 继续受支持。
5. 第五阶段：iOS 控制端 App；不托管 iOS 本机通用 Shell。

桌面 GUI 是已确定的后续产品能力，包括设备与配对管理、连接路径状态、内置终端和多会话 tab；第一阶段不实现 GUI。

## 4. 第零与第一阶段交付流程

### 4.0 第零阶段：环境与 Relay

1. 只读检查当前开发机的 OS/架构、Xcode、Homebrew、Rust、Docker 与构建工具；已经满足要求的工具不重复安装。当前 Apple Silicon Mac 已有 rustup、Rust/Cargo 1.98.0、rustfmt、Clippy、rust-analyzer、Xcode/clang、Homebrew、Git 和 CMake，缺少 Docker、`protoc` 与 `pkg-config`；iOS/Android Rust targets仍在。
2. 项目在`rust-toolchain.toml`精确固定当前最新版Rust 1.98.0，不再额外安装或验证Rust 1.91。以后只通过显式版本变更和全量质量门升级编译器，不让浮动`stable`静默改变构建。补齐固定质量工具，并用仓库固定的 Protobuf 生成方案避免把系统 `protoc` 变成最终用户依赖。当前 Mac 默认使用 Homebrew 管理 Docker CLI/Compose 与 Colima，除非用户在实际安装前指定 Docker Desktop。
3. 建立空 Rust workspace、CI/质量命令和 `deploy/relay`，从 Iroh 官方 v1.0.3 Release 下载匹配服务器架构的 `iroh-relay` 预编译产物并验证官方 SHA-256，封装成 scratch、非 root 的最小多架构镜像；不 fork、不修改、不重写 relay 数据平面。
4. 在本地完成上游 checksum、双架构镜像运行、最小 Compose 渲染和 HTTP smoke 后，必须停止并明确告知用户已经到达公网部署步骤，同时索取 SSH 入口/登录方式、relay 域名和 DNS 状态。用户提供之前不得连接服务器，连接秘密不得进入仓库或任务文档。
5. 获得连接方式后先做只读服务器 preflight；若 Docker 已存在且端口/DNS满足要求，则部署上游 relay。若需要安装 Docker、修改系统防火墙或处理端口冲突，先报告具体变更再执行。当前默认服务器使用既有 OpenResty/Cloudflare TLS 反代到宿主 `127.0.0.1:38451`；完成一次宿主 health、公开 HTTP 与真实 authenticated Iroh Relay 握手验收即结束，不开放 metrics、UDP 7842或修改防火墙。

### 4.1 安装与初始化

1. 项目把公开可审阅的 installer 脚本与预编译产物托管在 GitHub；用户通过官方 GitHub HTTPS 地址把 zterm 原生二进制安装到自己的 OS 账户范围内，不需要 Node/npm、Rust 工具链或管理员权限。
2. installer 无参数时只选择 GitHub 上最新的非 prerelease 稳定版；`--version <release-tag>` 可以安装任一仍受支持且已正式发布、签名并带 checksum 的精确版本，包括标记为 prerelease 的开发版。任意 commit、branch head 或会过期的 CI artifact 不是受支持安装源。
3. installer 只检测受支持的平台、下载并校验 release artifact、原子安装 `zterm`，不生成密钥、不写运行配置、不注册启动项，也不启动后台进程。
4. 用户显式运行 `zterm setup`，确认设备名，在 `~/.zterm/` 下生成长期设备身份与配置/状态，并在当前用户权限下 detached-spawn 唯一 daemon。
5. 后续任意需要 daemon 的本地 zterm 命令发现 daemon 不在线时，自动 detached-spawn 它；用户不需要先记住单独的 `start` 命令。

### 4.2 配对与连接

1. 宿主本地生成短时有效、只可成功使用一次的文本配对票据。
2. 控制端导入票据并证明自己持有长期设备私钥；宿主把该控制设备的公钥授权持久化到本地。
3. 配对是 SSH-like 单向授权：上述操作只让票据接收方获得控制票据创建方的权限，不产生反向授权。两台可托管桌面设备需要互相控制时，必须反向再生成和导入一张票据。
4. 后续连接只依赖双方长期设备身份和宿主本地授权，不需要账号、云端业务 API 或重复输入票据。
5. `zterm connect <设备>` 默认进入稳定名称 `main`：不存在则创建，存在则 attach。

### 4.3 持久会话

1. 用户可以列出、创建、命名、attach、显式 takeover 和关闭多个相互独立的 session。
2. 一个 CLI 进程一次只呈现一个 session；用户通过 detach 后重新 attach，或在本地终端模拟器的多个窗口/tab 中同时打开不同 session。
3. 网络断开或 CLI 退出只删除 attachment，不关闭远端 PTY。
4. 重新连接后，用户选择原 session，先取得权威当前画面和有界近期历史，再继续输入。

### 4.4 回到宿主本机继续

1. macOS/Linux 第一阶段和 Windows 第三阶段都必须允许当前 OS 用户通过 CLI 连接自己的本地 daemon；第四阶段桌面 GUI 复用同一路径。
2. 本地连接不是新建或复制 terminal：它通过仅限同 UID 的 local IPC attach daemon 已经持有的同一个 SessionId、PTY 与权威 VT。它不自配对、不 self-dial Iroh、不查询 DNS/Pkarr，也不经过 relay。
3. 用户可以先在手机上进入宿主 `main` 或其他 session 工作，手机 detach/进入后台后，再在宿主本机 attach 并从相同进程、cwd、当前 screen 和近期历史继续；反向切换也相同。
4. 本地 attachment 与远端 attachment 使用同一个 controller lease。手机仍持有控制权时，本地普通 attach 不打断它；显式 takeover 才转移控制权。zterm 自身提供这一能力，不要求用户安装 tmux、Herdr 或其他复用器。
5. Canonical 本机入口是 `zterm connect local [--session ...]`；`local` 是保留 selector。setup 完成后，裸 `zterm` 等价于 `zterm connect local --session main`，用于最快回到本机默认会话；尚未 setup 时只给出可操作的 `zterm setup` 提示，帮助始终通过 `zterm --help` 查看。

## 5. 产品需求

### R0. 第零阶段开发与基础设施门

- 第零阶段属于项目研发交付，不是最终用户安装流程；允许为开发机安装经确认的构建/Docker工具，但不得改变 zterm 1.0“最终用户无需 Rust、Docker、Node/npm 或管理员权限”的要求。
- 环境 bootstrap 必须先探测再补齐，能重复运行且不破坏现有 rustup toolchain、Xcode、Homebrew或用户配置；安装后记录实际版本和验证命令，不把个人绝对路径或秘密固化进产品配置。
- 项目当前开发与release工具链精确固定为Rust 1.98.0；Iroh的更低MSRV只表示依赖兼容下限，不是zterm需要维护的第二套编译器。升级Rust必须由显式变更触发并重跑format、Clippy、test、依赖和artifact门禁。
- relay 转发核心完全采用固定版本的官方上游 `iroh-relay`。zterm 只拥有 Dockerfile/下载校验、`relay.toml`、单一反代 Compose、日志轮转选择和最小运维文档；不写自有 relay协议、数据平面、业务认证服务、自定义 health 二进制或监控 sidecar。
- 本地 relay bundle 验证完成后设不可跳过的人工检查点。用户按约定在该点提供公网服务器连接方法；提供之前不发起连接。服务器部署先只读 preflight，发现需要额外系统级变更时必须先报告影响。
- 第零阶段部署的 relay 是第一阶段 Gate 0 和后续默认 profile 的基础设施。其 workspace/Release/GHCR版本映射、上游 checksum、非秘密配置和一次验收结果必须可追溯；服务器不安装 Rust、不编译 zterm，也不持有终端内容、PairTicket或设备私钥。

### R1. 每用户、非特权 daemon

- 受支持的 macOS 与主流 glibc Linux 上，每个 OS 用户拥有独立 daemon、Iroh EndpointId、长期私钥、授权设备、配置、日志、会话和本地运行目录。
- daemon、PTY、Shell 及其子进程始终以该用户权限运行；zterm 不执行用户切换，也不代理 root/Administrator 权限。
- 安装、setup、运行、升级和卸载不得请求 `sudo`、root 密码、PolicyKit、Administrator/UAC 或写系统级服务目录。
- 同一台电脑上的不同 OS 用户在远端表现为不同设备，配对和撤销互不影响，本地 socket、锁和日志不得使用跨用户冲突的全局路径。
- 官方 release 提供 darwin-arm64、darwin-x64、linux-arm64-gnu 和 linux-x64-gnu 四个预编译 artifact；installer 在下载前识别并拒绝 Alpine/musl、NixOS 原生环境和其他不受支持目标，不能猜测运行 GNU 产物。
- 程序安装目录与用户数据目录分离。第一阶段 macOS/Linux 的全部持久配置、身份、授权数据库、安装元数据与日志都位于当前用户 home 下的 `~/.zterm/`；第三阶段 Windows 使用等价的 `%USERPROFILE%\.zterm\`，移动 App 使用各自应用沙箱。
- `~/.zterm/` 只允许当前 OS 用户访问；设备私钥权限必须比普通配置更严格。daemon 的临时 socket/lock 可以位于平台安全 runtime 目录，因为它们不是持久配置且 daemon 停止后可重建。

### R2. 1.0 daemon 生命周期

- 1.0 不注册开机或登录启动项，不实现 systemd user service/linger、launchd、Login Item、crontab 或独立 supervisor，也不承诺 daemon crash 自动恢复。
- `zterm setup` 和后续按需启动必须幂等：同一用户最多一个 daemon，不得重置已有设备身份或启动第二个实例。
- daemon detached-spawn 后不得依赖发起终端的 stdio 或控制终端；关闭普通 Terminal 窗口或 SSH 连接不应产生 SIGHUP 连带退出。
- 若 Linux 的登录会话清理策略会终止整个用户 cgroup，`zterm doctor` 应明确报告限制；1.0 不提权修改系统策略。
- 宿主重启后，在该用户本地或通过 SSH 等独立通道首次运行 zterm 命令之前，设备显示离线。daemon 再次启动后沿用原身份和授权，无需重新配对，但重启前的 PTY 可以已经结束。
- zterm 不做后台版本检查、自动下载、自动安装或静默 daemon 重启；只有用户显式运行 `zterm update` 才访问官方 release metadata。`zterm update` 无参数时选择最新稳定版，`zterm update --version <release-tag>` 选择精确的已发布稳定版或开发 prerelease；开发版不会被默认选择。
- `zterm update` 必须先完整下载和验证候选 artifact，再检查 daemon。存在活动 session 时默认列出影响并要求确认；非交互流程没有显式 `--force` 就失败。确认后可以停止 daemon 并结束全部 PTY，再原子激活新二进制；激活失败恢复旧二进制，但已经结束的 PTY 不伪装为可恢复。
- 通过手工替换、失败回滚或其他非官方路径形成 CLI/daemon 版本不一致时必须明确诊断并拒绝不兼容操作，不能静默迁移或杀死 session。

### R3. 云端职责、直连与中继

- 产品不依赖账号、业务控制平面、云端设备目录、云端配置同步或云端会话存储。
- 云端只承担 Iroh 建连所需的地址查询、NAT 穿透协调和端到端密文转发。Iroh 可以先通过 home relay 完成发现与握手，再升级为 direct path；不能把云端描述为只在打洞失败后才参与。
- 设备之间的 QUIC 连接必须由 EndpointId 相互认证并端到端加密。relay 不持有内容密钥，不终止 zterm 加密，也不能读取终端输入、输出或应用协议明文。
- 云端不得持久化应用载荷。relay 的短暂内存缓冲不视为持久化；日志只保留排障所需的最小连接元数据，并明确披露来源 IP、EndpointId、时间和流量大小等仍可能被观察。
- 数据路径优先 direct；无法建立或维持 direct 时自动使用所配置 relay。direct/relay 切换只影响 transport，不改变 session、attachment、权限或 PTY 生命周期。
- NAT 打洞/direct 数据路径与 relay 密文回退是独立能力。QAD 只提供观察地址，可能提高部分网络的直连成功率，不参与 relay 转发，也不是打洞失败后回退 relay 的前提；第一阶段必须先在 QAD 关闭时做真实 NAT/path-events 测试，再依据数据决定是否提出独立 QAD-only 服务。
- relay 停机可以中断 attachment 或阻止新连接，但不得终止宿主上的 session；已有 direct path 可以继续运行。

### R4. 默认 relay 与地址查询

- 第零阶段先部署项目方默认 relay，第一阶段使用并发布同一部署物，同时允许用户自建。两者均直接封装固定版本的上游 Iroh 1.x `iroh-relay`，不重写 relay 协议或转发实现。
- relay 可在一台具有公网 IP、域名和现有 TLS 反代的 Linux 服务器上通过 Docker Compose 重复部署，包含版本映射、域名、回环端口、手动更新、一次健康/握手验收和有界日志说明；密钥和凭据不得提交到仓库或镜像。
- 第零阶段部署并在第一阶段使用的 relay 采用开放 beta 策略：`access = Everyone`，不要求 token，不配置 allowlist/denylist、连接限额、带宽限速、外部准入回调、自动封禁或无人消费的 metrics。
- 项目当前只支持现有 TLS 反代模式：Compose 只向宿主回环暴露纯 HTTP Relay 38451，QAD/UDP、metrics和 direct TLS/ACME 模板均不在当前部署中；出现真实需求后再独立设计。
- 若默认域名启用Cloudflare代理，WebSockets必须开启，初始101不得被WAF/限速挑战，Argo不得用于该流量；Cloudflare或OpenResty终止长连接后，Iroh重连只影响transport，不得终止宿主PTY。
- 文档必须明确：公开且无限制的 relay 可能被非 zterm Iroh 客户端使用并产生资源成本；能够使用 relay 不等于获得任何 zterm 终端权限。
- 第一阶段默认地址查询使用 Iroh 官方免费 DNS/Pkarr 服务 `dns.iroh.link`，不部署 `iroh-dns-server`。relay 与地址查询必须是独立可配置的 profile 字段。
- 地址发布只包含 EndpointId 所需的 home relay URL，不公开宿主 direct IP。直连候选只在已认证的端到端连接内交换。
- 配对票据与配对后的本地设备记录保存当时可用的 home relay 路由。新连接优先使用官方查询得到的新鲜签名记录，查询不可用时回退到本地缓存 relay；缓存也失效时明确失败，不创建新身份、不丢失配对，宿主 session 继续运行。
- 文档披露 `dns.iroh.link` 可观察的建连元数据、限速、无 SLA 和故障行为。公共 Iroh relay 不得成为 zterm 第一阶段发布或验收的业务流量依赖。

### R5. 设备身份、配对与撤销

- 每个设备在 `zterm setup` 时本地生成并保存独立的长期 Iroh Ed25519 设备私钥；它派生出的公开 EndpointId 是网络身份。长期凭证是私钥而不是 bearer token；每条 QUIC 连接仍使用独立的临时流量密钥。私钥不得进入票据、日志、DNS/Pkarr、relay 或其他云端组件。
- 配对票据具有显式格式版本、足够强度的随机秘密、到期时间和一次性语义；第一阶段使用文本，后续二维码承载完全相同的编码，不设计第二套配对协议。
- 票据只用于首次引导，不是长期凭证；过期、篡改、重放和并发重复消费都必须失败，失败不能留下半完成授权。
- 一次配对表示“完全信任该设备以当前 OS 用户身份使用此宿主”，类似加入远程 Shell 公钥。所有已配对设备都能列出、创建、attach、takeover 和关闭该 daemon 的全部当前及未来 session，并执行该 OS 用户本来就有权执行的操作。
- 这份主机级信任始终是有向的：`host -> controller` 表示 host 授权 controller 控制自己。接收方只把 host 保存为可连接设备，不为 host 写入反向 `device_auth`；Android/iOS controller-only 与桌面端使用完全相同的语义。
- 1.0 及后续不提供 per-session ACL、访客/分享链接、低信任设备角色或额外终端权限确认。未来 controller/observer 只表示同一用户的完全可信设备当前是否持有某个 session 的输入与 resize 权限。
- 本地提供授权设备查看、命名和撤销。无中央账号或全局撤销；丢失设备后，用户需要在每台曾授权它的宿主上分别撤销。
- 撤销必须先持久化，再立即使该 EndpointId 的全部现有 connection、stream、attachment 和 controller lease 失效；拒绝并发重连和尚未提交的输入。撤销不能关闭或向 PTY 发送信号，远端任务继续运行。
- 官方 `zterm uninstall` 是明确的身份销毁边界：确认活动 session 将结束后，停止 daemon，并删除本机 `~/.zterm/` 中的设备私钥、宿主授权和已知设备/配对状态，再删除程序。重新安装后 `setup` 必须生成新的 EndpointId，旧配对不能被新安装直接复用。
- 1.0 跟随 Zedra 的跨平台卸载模型：卸载只销毁本机身份，不依赖卸载前 `RevokeSelf` 或中央证书撤销服务。正常重装必须重新配对；若设备或私钥疑似泄露，用户在每台曾授权它的宿主上分别执行 device revoke。

### R6. Session 与 PTY 生命周期

- session 由宿主 daemon 持有，具有与任何 connection/stream 无关的稳定 ID 和用户可读名称；名称 `main` 是默认入口。
- 创建 session 时启动当前 OS 账户配置的交互式 login shell；Shell 解析以账户数据库为准，不依赖 daemon 启动环境中的 `$SHELL`。
- 默认 cwd 是该用户 home；显式创建时允许 `--cwd <宿主路径>`。目录无效或不可进入必须在创建 PTY 前失败，不留下残缺 session。
- 第一阶段不接受“创建时直接运行任意命令”的参数。用户 attach 后在同一持久 login shell 中启动 Codex、Herdr、tmux 或其他程序，前台程序退出并返回 Shell 不等于 session 结束。
- 控制端断网、退出、休眠、网络切换或 direct/relay 切换只会 detach，不能给 PTY 发送挂断信号。
- 只有 PTY 根进程自然退出、用户显式 close，或经确认停止 daemon 才回收 session。1.0 不按无输入、无输出或无人连接的持续时间自动关闭 session。
- daemon 崩溃、手动停止、手动重启、升级或宿主重启可以终止全部 PTY；1.0 不承诺进程跨 daemon 生命周期恢复。
- 会话数量、尺寸、输出队列、VT 与 scrollback 都必须有明确上限。达到上限时拒绝新建或丢弃可重建的旧历史，不能静默杀死仍在运行的 session，也不能无限占用内存或磁盘。

### R7. 权威终端状态与兼容性

- daemon 无论是否有控制端在线，都持续读取 PTY 并维护宿主权威 VT 状态；客户端消费速度不得反向阻塞 PTY reader。
- 权威状态至少覆盖主/备用屏幕、光标、样式、窗口尺寸、输入 mode 和有界标准 scrollback。旧 scrollback 可被淘汰，但当前 screen 不能因原始字节 backlog 溢出而变得不可恢复。
- attach 先取得带 session revision/watermark 的当前完整 snapshot，再从该点消费有序增量；snapshot 与增量交接不得丢失或重复。慢客户端可以被要求重新 snapshot，不能持有无界队列。
- 终端内容默认只在宿主内存中，1.0 不把 screen、scrollback、输入或输出 transcript 持久化到磁盘。
- 通用终端只保证当前主/备用屏幕与有界标准 scrollback，不承诺保存任意 alternate-screen TUI 的完整内部对话历史。
- zterm 不按程序名识别 tmux、Herdr、GNU Screen、Zellij 或其他 TUI，也不实现复用器专用协议；所有程序走同一 PTY/VT 路径。
- daemon 必须实现其声明的终端能力与必要查询响应；`TERM`/`COLORTERM` 不得宣称未通过兼容测试的能力。
- daemon 只向客户端发送由权威 TerminalModel 生成的受控 snapshot/delta 与显式 side event；未识别的 OSC/DCS/APC、OSC 52、图形协议或其他原始控制载荷不得绕过解析器直接转发到本地终端。
- 第一阶段用 tmux 与 Herdr 作为黑盒验收样本，覆盖键盘、Unicode、颜色、光标、alternate screen、bracketed paste 和连续 resize。
- CLI 本地控制键使用可配置、可禁用的 `Ctrl+]` 前缀：`Ctrl+] .` detach，`Ctrl+] Ctrl+]` 向远端原样发送单个 `Ctrl+]`。本地 detach 不能关闭 PTY。

### R8. Connection、stream、attachment 与控制权

- 同一控制设备到同一宿主设备的多个 session 共享一条已认证 Iroh connection；不能为每个 session 重复寻址、NAT 穿透和认证。
- 桌面端由当前用户 daemon 持有设备私钥、Iroh endpoint 和按远端 EndpointId 建立的连接池。多个本地 CLI 通过仅限该用户访问的 IPC 向 daemon 请求 attachment，从而共享远端 connection。
- 同一个 local IPC 也必须支持目标为本机 daemon 的 self attachment。它直接调用本机 SessionRegistry/SessionActor，不创建 Iroh connection；local 与 remote adapter 必须共享同一内部 session 服务、TerminalModel、revision 与 controller lease，不能形成两套会话状态。
- 本地 self attachment 以同 UID socket peer 作为信任入口，可以用本机 EndpointId 作为显示身份，但不得要求自配对或把本机写入自己的 `device_auth`。其他 OS 用户不能通过 socket attach。
- 第一阶段的远端 session 管理 RPC 和 terminal attachment，以及后续客户端的通用设备事件，都使用同一 QUIC connection 上相互独立的 stream；本机 self target 则通过 local IPC adapter 调用同一 SessionService。单个慢 stream、重同步或关闭不能阻塞控制 RPC 或其他 session。
- 所有会改变远端状态的 control RPC 必须携带客户端生成的 operation ID，并具有可重试的幂等结果；连接在服务端提交后、客户端收到响应前断开时，重试不得重复创建 session、重复 rename 或把已完成操作误报为新的失败。
- session、attachment、connection 和 controller lease 是不同对象；lease 绑定 `(session_id, attachment_id)`，不能绑定整条 connection。
- 1.0 每个 session 同一时刻只有一个具有输入和 resize 权限的 controller。第二个普通 attach 不得干扰现有 controller；显式 takeover 原子转移控制权，原 controller 收到明确原因并 detach，PTY 与 session ID 不变。
- Iroh connection 断开时其 attachment 结束，但 session 继续。连接恢复后，在仍打开的本地视图上分别 reattach；各 session 通过自己的 snapshot 恢复。
- 协议与内部模型允许一个 session 将来拥有多个 attachment，并让 output revision 属于 session 而非 connection；1.0 不交付多观察端 UI 或同时多写者。

### R9. 后续客户端兼容约束

- Android 与桌面 GUI 默认只让前台 tab 完整订阅终端流；后台冷 tab detach terminal stream 并释放 controller lease，只保留最后缓存画面并接收轻量 session/output revision。切回时复用已有设备 connection 请求 snapshot，同步完成前不发送或排队用户输入。
- 冷 tab 的首屏恢复不无条件传输整个 scrollback；更早的有界历史按需分页。资源充足时可保留少量 warm tab，但这只是优化。
- 未来多端同时连接采用一个可写 controller 加多个只读 observer，并支持显式、原子控制权转移；多写者协作不是默认行为。
- 协议具有显式主版本和可选能力协商；未知可选事件可被旧客户端安全忽略。
- 2.0 之前，Codex、OpenCode 等只是 PTY 中的普通程序。未来 Agent 状态观察、结构化事件与通知必须位于 session 旁路，不进入 Iroh transport、设备认证、PTY 生命周期或原始终端兼容回退路径；第一阶段不实现占位插件框架。

## 6. 第零与第一阶段验收标准

### Z. 第零阶段环境与公网 Relay

以下勾选项保留 2026-08-21 第零阶段当时实际执行过的历史证据；其中 digest、metrics与回滚 smoke 不再是当前或后续发布 Gate。现行 Relay 运维契约由子任务 `08-21-simplify-relay-release-deployment` 与 `.trellis/spec/backend/relay-deployment.md` 取代。

- [x] 当前开发机经“探测后补齐”具备精确固定的Rust/Cargo 1.98.0、rustfmt、Clippy、Cargo质量检查、Docker Engine/Compose和仓库固定Protobuf生成环境；已有Rust/Xcode/Homebrew配置未被无故覆盖，空workspace的check/fmt/clippy/test/deny命令通过，浮动stable升级不会绕过版本变更与质量门。
- [x] `deploy/relay` 只下载并校验官方 Iroh v1.0.3 `iroh-relay` release artifact，不包含自有转发实现；本地镜像固定上游版本/checksum和最终digest，Compose config、启动、health、私有metrics、日志轮转、停止与回滚smoke通过。
- [x] 本地验证通过后助手明确暂停并通知用户提供公网服务器连接方式；提供之前没有服务器连接尝试，SSH私钥、token、真实`.env`等秘密没有写入仓库、任务产物或日志。
- [x] 获得连接方式并完成只读preflight后，公网服务器以Docker运行单实例relay；`relay.zenithconsulting.cn` 的外部TLS与真实Iroh握手、宿主回环38451/9090、Everyone/no-limits、health和metrics隔离通过smoke。UDP/QAD保持关闭且没有防火墙变更；文档明确它是可选直连辅助而非relay回退依赖。
- [x] 固定digest回滚演练通过；relay停止不会影响本地开发环境，服务器与容器持久卷/日志中没有zterm终端明文、PairTicket或设备私钥。第零阶段证据完成前不进入第一阶段Gate 0。

### A. 安装、权限与进程生命周期

- [ ] 在干净的 macOS x86_64/arm64 和主流 glibc Linux x86_64/arm64 普通用户账户中，可以使用官方 HTTPS installer 完成安装与 `zterm setup`，全程不请求管理员授权，也不要求 Node/npm 或本地 Rust 工具链。
- [ ] installer 脚本、版本 manifest 与 release artifact 可全部从项目官方 GitHub 仓库/Release 获取；无参数安装最新稳定版，`--version` 可精确安装一个稳定 tag 或签名的开发 prerelease，且默认路径绝不选择 prerelease。
- [ ] installer 校验 release artifact 后原子安装到当前用户可写目录；安装结束但尚未运行 setup 时，没有设备密钥、运行配置、启动项或常驻 zterm 进程。重复 setup 保留原 EndpointId 和授权且只运行一个 daemon。
- [ ] 第一阶段所有持久用户数据都落在对应账户的 `~/.zterm/`，目录和私钥权限阻止其他普通用户读取；两个本地 OS 用户可以独立安装和运行各自 daemon，拥有不同 EndpointId、数据目录、socket、授权与 session，无法通过 zterm 跨越用户权限。
- [ ] 从普通终端或 SSH detached-spawn daemon 后，关闭该终端不会结束 daemon 或 PTY；doctor 能报告会清理整个登录 session 的已知平台限制。
- [ ] 重启宿主后无人运行 zterm 时控制端明确显示设备离线；本地首次运行需要 daemon 的命令会自动拉起它，原配对恢复可用但旧 PTY 不被伪装成仍存活。
- [ ] 不显式运行 `zterm update` 时不会检查、下载或安装更新。下载、manifest 签名、checksum 或候选自检在 daemon 停止前失败时，现有 session 不受影响。
- [ ] `zterm update` 遇到活动 session 时默认拒绝并列出影响，确认或显式 `--force` 后才停止 daemon、结束 PTY 并原子激活候选；激活失败恢复旧二进制。卸载同样先处理活动 session，并在明确警告后删除 `~/.zterm/` 与程序；重新安装生成不同 EndpointId，任何旧配对都不能被新安装直接使用。

### B. 配对与授权

- [ ] 无账号、注册或业务 API 即可通过一次性文本票据完成两台设备配对；控制端重启后直接使用长期身份连接。
- [ ] A 生成票据、B 导入后，B 可以控制 A，A 不因此获得控制 B 的权限；只有反向再完成一次配对才建立反向授权，两个方向可以分别撤销。
- [ ] 配对票据默认通过交互式 TTY/stdin 导入，不作为命令行参数暴露在 shell history 或进程参数中；完整票据和 pair secret 不进入 zterm 日志。
- [ ] 成功消费后的票据不能重放；过期、篡改和并发重复消费均失败且不产生半授权。后续二维码可以直接承载同一版本化字节格式。
- [ ] 已配对设备无需逐 session 授权即可管理全部当前及未来 session；未经配对的 EndpointId 即使能使用 relay，也不能通过 zterm ALPN/握手访问任何 RPC 或终端数据。
- [ ] 撤销在线 controller 后，其全部 connection、stream、attachment 与 lease 立即失效，并发重连和未提交输入不能绕过撤销；PTY 与任务继续，另一已授权设备可以 reattach，daemon 重启后授权不会复活。

### C. 网络、隐私与 relay

- [ ] macOS 与 Linux 之间可以完成 CLI 连接与终端交互；在可直连网络中能观测 direct path，在阻断直连时自动使用配置的自建 relay，切换期间 PTY 不变。
- [ ] 抓取 direct 与 relay 路径以及检查 relay 日志，都不能得到终端或 zterm 应用协议明文；relay 不持有可解密业务载荷的密钥，也不落盘应用载荷。
- [ ] 一台干净公网 Linux 服务器可按文档通过现有 TLS 反代和最小 Docker Compose 启动 relay；只发布宿主回环 38451，配置为 Everyone 且无 token、名单、限速、QAD或metrics，手动 `pull`/`up -d` 后一次 health 与 authenticated Relay handshake 通过。
- [ ] 项目默认 relay 与一套独立自建 relay 均通过端到端验收；网络观测证明测试没有把 Iroh 公共 relay 当作业务流量回退路径。
- [ ] relay 与地址查询可以分别替换而不改变 EndpointId、配对授权或终端协议；诊断能显示实际 profile、home relay、地址来源和当前 direct/relay path。
- [ ] 公开 Pkarr 记录只含签名的 home relay 路由，不含宿主 direct IP。阻断 `dns.iroh.link` 后，只要缓存 relay 可用，已配对设备仍能重连；缓存也失效时给出明确错误且 session 继续。
- [ ] relay 完全不可用且没有既有 direct path 时，新连接明确失败而不发布 direct IP；已有 direct path 与宿主 session 不被主动终止。

### D. Session、终端恢复与连接复用

- [ ] 首次裸连接创建并进入 `main`，以后默认回到同名 session；可以再创建至少两个独立 session，并分别 list、rename、attach、detach、takeover 和 close。
- [ ] session 使用当前 OS 账户的交互式 login shell；默认 cwd 为 home，合法 `--cwd` 生效，无效目录不留下 session；创建接口不接受任意启动命令。
- [ ] 在 session 中运行持续任务后强制断开网络或退出 CLI，任务不收到由 transport 断开导致的挂断信号；恢复后 reattach 到同一 session ID，取得正确当前 screen 与有界近期历史并继续输入。
- [ ] 无人连接、无输入或无输出的 session 不会自动关闭；前台子进程退出并返回 Shell 不回收 session，只有根 Shell 退出、显式 close 或经确认停止 daemon 才结束。
- [ ] 同一控制设备同时打开多个 session 时只建立一条到宿主的 Iroh connection，各 attachment 使用独立 stream；关闭或拖慢一个 attachment 不影响其他 session 或控制 RPC。
- [ ] 强制断开共享 connection 后，全部本地 attachment 显示重连状态，宿主 session 继续；恢复一条 connection 后分别 reattach 并取得各自正确 snapshot。
- [ ] snapshot 与 revision 增量的交接无丢失、无重复；长时间高输出、慢客户端和历史淘汰不会阻塞 PTY reader、破坏当前 screen 或造成无界内存增长。
- [ ] 对 create、rename、close 等状态变更注入“服务端已提交但响应丢失”的故障后，客户端重试得到同一操作结果，不产生重复 session 或错误的二次副作用。
- [ ] 已有 controller 时普通第二次 attach 不注入输入、不 resize、不打断原 controller；显式 takeover 后原 controller 明确 detach，新 controller 接续同一 PTY 与 session ID。
- [ ] 第一阶段由另一台已配对的桌面 CLI 在 macOS/Linux 宿主的 `main` 或命名 session 中启动任务并 detach，用户可以在该宿主本机通过 same-UID local IPC attach 相同 SessionId，取得相同进程、cwd、当前 screen 与近期历史并继续输入；本机 detach 后远端 CLI reattach 仍是同一会话。第二阶段 Android 必须复用并实机验收同一契约，不另建手机专用 session 路径。
- [ ] 本机 self attach 不要求配对，不建立到自身 EndpointId 的 Iroh connection；在外网、DNS/Pkarr 和 relay 全部不可用时仍可工作。远端控制端（第二阶段可为手机）仍持有 controller 时本机普通 attach 返回 occupied，显式 takeover 才原子转移 lease。
- [ ] 本机创建、rename、close 的 session 与远端设备看到的是同一对象；两个本地 CLI 同样遵守单 controller 规则，其他 OS 用户无法通过 local IPC 查看或控制该用户的 session。
- [ ] setup 后运行裸 `zterm` 直接 attach 本机 `main`；未 setup 时不静默创建身份，只提示运行 `zterm setup`；`zterm --help` 稳定显示命令帮助，显式 `zterm connect local` 与裸命令行为等价。
- [ ] 在没有程序名特判的同一通用路径中，tmux 与 Herdr 均通过键盘、Unicode、颜色、光标、alternate screen、bracketed paste、连续 resize、断线存活和 snapshot 恢复验收。
- [ ] `Ctrl+] .` 只 detach；`Ctrl+] Ctrl+]` 向远端发送原始前缀；改键和禁用前缀后行为正确。

### E. 兼容性与文档

- [ ] 协议文档分别定义 device、connection、session、attachment、controller lease、revision 和 capability，且未来增加 observer 或 Agent 可选事件无需替换 session 身份或基础 terminal 流。
- [ ] 第一阶段代码、协议和 CLI 不根据 Codex、OpenCode、tmux 或 Herdr 的进程名和输出格式改变行为，也不包含未验证的 Agent 插件框架。
- [ ] 恶意或畸形终端输出不能让 daemon 崩溃，也不能通过未过滤的 OSC 52、DCS/APC 或未知图形序列驱动控制端本地操作；基础 screen 恢复仍保持正确。
- [ ] 运维与用户文档准确披露：默认 relay 的开放准入与成本风险、`dns.iroh.link` 可见元数据及无 SLA、1.0 无开机自启、daemon 重启会终止 PTY、终端内容不落盘且 alternate-screen 完整历史不受保证。
- [ ] 支持矩阵明确写出第一阶段只承诺 macOS 与主流 glibc Linux 的 x86_64/arm64；在 Alpine/musl 或 NixOS 原生环境上安装时应给出可操作的 unsupported-platform 错误，不选择不兼容二进制。
- [ ] 第一阶段的核心协议测试和平台抽象可在不改变既有设备身份与授权语义的前提下继续实现 Android、Windows、桌面 GUI 和 iOS 路线图。

## 7. 明确不在第一阶段范围内

- macOS/Linux/Windows 桌面 GUI，Android App，Windows daemon/CLI，iOS App。
- 开机或登录自动启动、无人登录冷启动可达、systemd linger、LaunchAgent/LaunchDaemon、Login Item、用户 crontab、自研 supervisor 和 daemon crash 自动重启。
- PTY 或 Agent 进程跨 daemon 崩溃、重启、升级或宿主重启存活。
- 自动更新和静默 daemon 重启。
- npm、Homebrew、mise、Nix 等包管理器的官方第一阶段发行渠道；任意 commit/branch 构建、会过期的 GitHub Actions artifact、自动跟随 nightly，以及未签名的开发包。第一阶段只安装 GitHub Release 中发布、签名并带 checksum 的稳定版或显式指定的开发 prerelease。
- 多个客户端同时观看或同时输入同一个 session；未来 observer 只保留模型兼容性。
- 账号、云端设备目录、云端同步、中央授权或全局撤销。
- 卸载前 `RevokeSelf` 广播、中央证书撤销列表和卸载时保证已复制私钥在全部离线宿主上失效；泄露设备继续使用逐宿主 device revoke。
- 分享 connection/session、访客链接、per-session ACL、低信任只读角色或额外沙箱。
- 云端终端内容存储、解密、检查、索引、回放或 Agent 推理。
- 自建 `iroh-dns-server`、relay 的 token/名单/限速/自动封禁控制面、多地域、高可用、自动扩缩、计费、正式 SLA 和完整抗 DDoS 平台。
- 任意命令作为 session 根进程、终端内容录制、完整 transcript、文件传输、端口转发、SFTP 或 SSH 兼容层。
- 2.0 之前的 Agent 专用启动、状态/审批识别、恢复协议、完成通知或厂商 UI。
- Alpine/musl 原生二进制、NixOS 原生包装与主流 glibc Linux x86_64/arm64 以外的 Linux 发行/架构承诺。

## 8. 研究索引

- `research/phase-zero-development-relay.md`：当前开发机探测、第零阶段人工服务器检查点、官方 `iroh-relay` 复用边界与 Zedra 源码证据。
- `research/reference-baseline.md`：Zedra 与旧项目基线。
- `research/install-version-identity-lifecycle.md`：GitHub 托管、稳定/开发版本选择、Zedra 身份模型与卸载失效边界。
- `research/local-self-attach.md`：桌面本机视图接续手机 session 的 local IPC、控制权与跨阶段契约。
- `research/cloud-trust-boundary.md`：Iroh 地址查询、relay、E2EE 与元数据边界。
- `research/persistent-agent-session.md`：connection 与持久 PTY 的生命周期拆分。
- `research/terminal-reconnect-state.md`：Herdr/Zedra 的 VT、scrollback 与恢复方案对比。
- `research/connection-multiplexing.md`：一条设备 connection 复用多个 session stream。
- `research/authorization-scope.md`、`research/device-revocation.md`：主机级信任与即时撤销。
- `research/per-user-daemon.md`：Herdr、Zedra、Paseo 与 1.0 非自启决定。
- `research/distribution-update-channel.md`：Herdr 公开发行历史、npm 与 direct installer 的升级所有权取舍。
- `research/shell-session-startup.md`：login shell 与 cwd 约定。
- `research/relay-access-policy.md`：Everyone、无限速 relay 的已确认风险。
- `research/planning-review-2026-08-21.md`：历史决策对照、反方审阅发现与决策收敛状态。
