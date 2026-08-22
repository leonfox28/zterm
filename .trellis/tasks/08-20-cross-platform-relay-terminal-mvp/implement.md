# zterm 第零与第一阶段实施计划

> **2026-08-21 后续决策（覆盖本文旧的默认自建 Relay 步骤）：** 第一阶段产品默认改用 Iroh 1.0.3 官方 n0 生产 Relay/QAD 与生产 DNS/Pkarr；现有自建镜像、Compose 和公网部署仍是可选能力。Foundation Step 1 的实际实施、验证和 no-go 停止线由 `08-21-foundation-gate` 管理。

## 1. 执行目标

本计划把 PRD 与技术设计转换为可验证的实施顺序。第零阶段先交付可重复的本机开发环境、Rust workspace和已经在用户公网服务器运行的固定上游relay；第一阶段最终交付：

- macOS 与 Linux 上的每用户非特权 zterm daemon；
- macOS 与 Linux 上通过官方用户级 direct installer 分发的 zterm CLI，以及完全手动触发的 `zterm update`；
- 基于 Iroh 的直连优先、项目 relay 引导与失败回退；
- 设备配对、授权、撤销和端到端加密；
- 与网络连接生命周期分离的多持久 PTY session；
- 一台 Linux 服务器可用 Docker Compose 部署的默认或自建 relay。

本任务当前只定义执行契约。用户批准规划后才创建源码和启动实施。

## 2. 实施原则与停止条件

1. 先完成第零阶段本机环境、relay本地smoke、人工服务器连接检查点、公网部署与一次health/authenticated-handshake验收，再进入M1 Gate 0；Gate 0 未证明 Iroh profile、PTY 生命周期和权威 VT 模型可行时，不进入 M2 之后的产品实现。
2. 协议与安全边界优先于 CLI 表面。设备身份、授权 generation、session/revision 标识和 frame limits 必须由共享 core/proto 定义，禁止在各命令中各自解释。
3. 每个里程碑都必须同时交付测试和诊断能力；不能把安全、资源限制或平台测试全部留到最后。
4. 第一阶段不创建 systemd、launchd、Login Item、crontab、自研 supervisor、后台更新检查或自动安装代码；只实现用户显式执行的 `zterm update`。
5. 第一阶段不创建账号服务、业务数据库、Web 控制面、自有 DNS/Pkarr 服务、relay token/名单/限速功能。
6. 依赖版本与外部兼容样本必须固定；升级 Iroh、PTY、VT 或 protobuf 工具链需要重新运行对应门禁。
7. 终端内容不得为了调试、重连或测试便利而写入产品日志或持久数据库。

Gate 0 的硬停止条件：

- 无法确保只配置项目/用户 relay，或会隐式回退公共 Iroh relay；
- PTY reader 在零 attachment 时不能持续排空，导致任务阻塞或被挂断；
- 候选 TerminalModel 无法可靠表达 snapshot、revision、delta、alternate screen、Unicode 宽度和输入 mode；
- tmux 与固定版本 Herdr 的黑盒基线不能通过，且替代 VT 候选也不能通过；
- 在所有资源有界的前提下，无法可靠支持 `main` 加两个额外 session（共 3 个）。

出现硬停止条件时只允许替换封装后的技术实现、重新测量阈值并更新设计证据；不得静默降低 PRD 验收标准。

## 3. 任务拆分与依赖

最终规划批准后、第一次运行 `task.py start` 之前，将实施拆成以下 Trellis 子任务。每个子任务都有独立 PRD/实施/检查上下文，并在子任务元数据中写明下表的依赖；不在本轮规划审阅中创建或启动它们：

| 子任务 | 包含里程碑 | 依赖 | 主要成果 |
| --- | --- | --- | --- |
| phase-zero-bootstrap | Z0-A、Z0-B | 无 | 本机环境、workspace、上游relay部署物、人工连接检查点与公网一次验收证据 |
| foundation-gate | M1 | phase-zero-bootstrap | Iroh/PTY/VT 可行性结论 |
| core-local-daemon | M2-M3 | foundation-gate | core/proto/platform、持久状态、本地 IPC 与 daemon 生命周期 |
| session-engine | M4 | core-local-daemon | 持久 PTY、权威 VT、snapshot/delta、controller lease |
| transport-auth | M5-M6 | core-local-daemon | Iroh connection broker、配对、认证、撤销 |
| remote-cli | M7-M8 | session-engine、transport-auth | 远程 session 协议、CLI 交互与复用 |
| distribution-release | M9 | foundation-gate；M8 前完成联调 | 原生 release artifacts、direct installer 与手动更新流程 |
| e2e-hardening | M10 | 全部前置任务 | 网络实验、安全、平台与发布验收 |

依赖主线：

    Z0-A → Z0-B(本地) → [通知用户并等待服务器连接方式]
                         → Z0-B(公网部署+一次验收) → M1 → M2 → M3 ─┬→ M4 ─┐
                                                               └→ M5 → M6 ─┴→ M7 → M8 → M9 → M10

M4 与 M5/M6 在 M3 契约冻结后可以独立推进；M7 负责把两条路径合并。Z0-B 部署的relay直接提供给M1和后续网络测试使用，不能把公网部署拖回第一阶段末尾。

## 4. 里程碑

### Z0-A. 本机开发环境、仓库与质量基线

工作内容：

- 先探测再安装：记录当前macOS/架构、Xcode、Homebrew、rustup/toolchain、Docker、`pkg-config`与生成工具状态；已有工具不重复安装、不覆盖用户配置。当前证据是Apple Silicon macOS 26.6.2，已有rustup + stable Rust/Cargo 1.98.0、rustfmt、Clippy、rust-analyzer、Apple/iOS/Android targets、Xcode/clang、Homebrew、Git、CMake，缺少Docker、`protoc`与`pkg-config`。
- 不再安装Rust 1.91。用`rust-toolchain.toml`精确固定1.98.0并验证现有rustfmt/Clippy，安装固定版本的`cargo-deny`等实际质量门工具。默认通过Homebrew安装Docker CLI/Compose与Colima并启动用户态Linux VM；用户若在执行前指定Docker Desktop，只替换本机runtime。
- Protobuf使用仓库固定/可重复的生成方案；开发环境可以按实现需要提供vendored `protoc`，但最终用户和release构建不能依赖机器上偶然存在的系统版本。
- 建立 Rust workspace：zterm-core、zterm-proto、zterm-platform、zterm-daemon、zterm-cli。
- 建立 proto、install、deploy/relay、tests 与 docs 的目录边界。
- 固定开发与release Rust版本1.98.0和Iroh 1.0.3；不声明或测试单独的zterm MSRV。未来Rust升级必须显式修改toolchain并重跑全部门禁。
- 选择可重复的 protobuf/prost 生成方式，避免要求最终用户安装 protoc。
- 配置格式化、Clippy、单元/集成测试、依赖许可证与漏洞检查。
- 为 macOS arm64/x64、主流 glibc Linux x64/arm64 建立 build/test matrix；平台特有测试只在对应 runner 运行，Alpine/musl 和 NixOS 只验证可操作的 unsupported-platform 诊断。
- 定义错误分类、日志字段、secret redaction、测试 fixture 与 benchmark 目录规则。
- 把 design.md 的信任/威胁边界固化为安全测试矩阵，并在开发前冻结精确的 Linux libc/发行支持矩阵。

完成标准：

- `cargo`在仓库中解析为精确1.98.0，rustfmt/Clippy和固定Cargo工具版本可复现；bootstrap重复运行不重装或破坏既有环境，浮动stable变化不能绕过仓库toolchain pin。
- Docker Engine与Compose在本机可用，Linux容器smoke通过；开发环境实际版本和非秘密配置有记录。
- 空功能 workspace 可在支持平台编译，所有质量命令通过。
- proto 生成结果可重复，工作树不会因重复生成产生差异。
- CI 不依赖管理员权限或系统级安装步骤。
- 仓库 README 明确当前仍是第一阶段开发版及支持矩阵。

验证：

    rustup show
    rustc --version
    cargo --version
    docker info
    docker compose version
    cargo check --workspace --all-targets
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo deny check

检查点与回退：

- Z0-A只建立开发环境与基础结构；若容器runtime或工具链选择失败，可在没有数据或wire兼容负担时替换，并卸载仅由本阶段新增且确认无其他项目依赖的组件。
- 未固定的生成器、动态下载脚本或平台条件编译警告不得带入Z0-B/M1。

### Z0-B. 固定上游 Iroh Relay、本地验证与公网部署

工作内容：

- 从Iroh官方v1.0.3 GitHub Release下载匹配Linux服务器架构的`iroh-relay`预编译artifact，校验官方SHA-256；不fork源码、不写自有转发服务，也不在服务器安装Rust或现场编译。
- 建立scratch、shell-free、UID/GID 65532镜像、单一`relay.toml`与最小`compose.yaml`；固定上游版本/checksum，镜像默认command指向config，Compose project与容器均名为`zterm-relay`。
- 为当前默认服务器和现阶段自建文档只保留同机TLS反代模式：容器仅绑定宿主回环38451，不使用`--dev`、ACME、metrics或UDP/QAD。配置明确Everyone，省略limits、token、名单和外部auth。
- GitHub stable Release使用原样`vX.Y.Z`镜像tag并更新`latest`；服务器只通过人工`docker compose pull`和`up -d`更新，普通重启不自动拉取。
- 在本机完成artifact checksum/篡改负向测试、双架构运行、最小Compose config与直接HTTP smoke；每项契约只由一个测试边界负责。
- 本地验证通过后硬停止，向用户明确报告“已到公网relay部署步骤”，并只在此时索取SSH入口/登录认证方式、relay域名与DNS状态；用户提供前不发起连接，秘密不写入Git、task artifact或命令输出。
- 获得连接方式后先只读检查服务器OS/架构、Docker/Compose、DNS、端口占用、防火墙和磁盘。已有条件满足时部署；需要安装Docker、修改防火墙或处理冲突时先报告精确影响，再执行获准的系统变更。
- 部署后只验证宿主health、公开HTTP与一次真实authenticated Iroh Relay握手。在服务器确认只有回环38451、Docker `local`日志、没有metrics/UDP/QAD或防火墙变更、Everyone/no-limits和无业务数据卷；通过即停止，不重复restart/reconnect或回滚演练。

完成标准：

- zterm仓库不存在relay协议/转发实现；镜像中的服务端二进制与官方v1.0.3 artifact checksum一致。
- 人工检查点确实发生在第一次服务器连接之前；连接凭据与真实`.env`不出现在仓库、文档或日志。
- 一台用户指定的公网Linux服务器以Docker运行`ghcr.io/leonfox28/zterm-relay:latest`，通过外部TLS、真实Iroh握手、回环38451、health与Docker `local`日志验收；metrics/QAD/UDP明确未部署且不影响密文relay回退。
- relay没有zterm业务数据库，不保存终端明文、PairTicket或设备私钥；运行异常直接recreate，只有实际镜像缺陷才人工选择上一版本tag。

验证：

    sh tests/relay/verify-upstream.sh
    sh tests/relay/build-platforms.sh
    docker compose -f deploy/relay/compose.yaml config --quiet
    sh tests/relay/smoke.sh
    sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn

检查点与回退：

- 服务器连接信息缺失不是失败：Z0-B在明确人工检查点暂停，收到用户信息后继续同一阶段。
- 公网变更前只读记录当前容器/端口状态；部署失败时报告实际状态并停止扩展操作，不运行自动fallback，也不删除服务器上不属于zterm的资源。
- 若官方binary与目标服务器不兼容，允许改为从固定官方tag可复现构建镜像，但必须记录原因与源码commit；仍不得fork转发逻辑。

### M1. Gate 0 技术可行性

工作内容：

- 用 Minimal preset、Custom relay、n0 DNS/Pkarr 构建两个 Iroh endpoint，显式断言 relay map 不含公共 Iroh relay。
- 验证 route lookup、home relay、QUIC 建连、多 stream、path events、direct/relay 切换与失败信息。
- 用 portable-pty 启动当前账户 login shell，验证输入、输出、resize、root child exit和无人 attachment 持续排空。
- 在 TerminalModel trait 后评估 vt100；以 avt 或更完整候选作为失败后的替换路径。
- 建立固定 ANSI corpus，覆盖 main/alternate screen、光标、样式、256/true color、Unicode 宽度、组合字符、清屏、scroll region、bracketed paste、mouse/focus mode、DA/DSR。
- 运行真实 tmux 与固定提交 Herdr 黑盒测试，并将 Codex/OpenCode 仅作为人工全屏 TUI smoke。
- 以 16 session、每 session 10k scrollback 和 256 MiB 全局 terminal-state 作为 Gate 0 候选压力矩阵完成测量；Foundation 报告最终固定产品准入值为 8 session、每 session 2,000 行、最大 240x80、128 MiB summed fixed-cell projection，进程 RSS 测量目标为 256 MiB。

完成标准：

- 输出书面 Gate 0 报告，记录最终 VT 实现、已知但可接受的兼容差异和资源基线。
- full snapshot 加连续 delta 与直接读取最新 TerminalModel 状态等价。
- client 退出、网络断开和 relay 停止均不关闭 PTY；root shell exit 才使 session 结束。
- direct 可用时能观察路径升级，阻断 direct 后仍可经唯一配置的自建 relay 工作。
- tmux 与 Herdr 基线可交互、resize、detach/reattach，不因持续输出死锁。

验证：

    cargo test -p zterm-platform --test pty_lifecycle
    cargo test -p zterm-core --test terminal_corpus
    cargo test -p zterm-core --test terminal_snapshot_delta
    cargo test -p zterm-core --test terminal_blackbox
    cargo bench -p zterm-core --bench terminal_state
    cargo test -p zterm-daemon --test iroh_profile_gate

检查点与回退：

- Gate 0 报告获得检查通过前，不创建完整 pairing/session RPC。
- vt100 未达标时保留 TerminalModel corpus 和 trait，替换实现后重跑全部测试；不在协议中暴露库私有结构。

### M2. Core、协议、平台抽象与持久状态

工作内容：

- 在 zterm-core 定义 DeviceId、SessionId、AttachmentId、AttachmentPrincipal（REMOTE_ENDPOINT / LOCAL_SAME_UID）、Revision、ControllerLease、能力位、资源限制和领域错误。
- 在 zterm-proto 定义 versioned protobuf envelope、控制消息、配对消息、session RPC 与 terminal snapshot/delta；实现 length-delimited framing。
- 对 frame 设 8 MiB 总上限、control payload 设 1 MiB 上限，并在分配前校验。
- 为所有会改变远程状态的 RPC 定义 128-bit client-generated operation ID、有界去重窗口和可重放的原结果。
- 在 zterm-platform 定义用户目录、原子文件写入、0600/0700 权限、同用户 single-instance lock、peer UID、本地 socket 和 detached process 接口。
- 建立 SQLite schema 与事务迁移，保存 identity metadata、authorized devices、revocation generation/tombstone、peer route cache 和配置版本。
- 长期 Iroh secret key 使用单独 0600 文件；terminal bytes、scrollback、PTY/session 运行态不入库。
- 不创建持久 audit-event 表；授权、撤销和迁移只记录有保留上限且已脱敏的结构化日志。
- 建立配置 profile 校验：默认项目 relay、自建 relay、n0 DNS/Pkarr；拒绝空 relay 或隐式公共 relay。

完成标准：

- 每个跨层对象只有一个共享定义与 decoder；unknown field 可按 protobuf 兼容规则忽略，unknown kind 明确拒绝。
- 所有持久写入可模拟崩溃并保持旧状态或完整新状态，不出现半写授权。
- Unix 权限和 symlink 攻击测试通过；非 owner 的 local IPC 被拒绝。
- schema 版本错误、wire major 不兼容和配置错误有可操作的诊断。

验证：

    cargo test -p zterm-core
    cargo test -p zterm-proto
    cargo test -p zterm-platform
    cargo test -p zterm-daemon --test persistence
    cargo test -p zterm-daemon --test config_profiles

### M3. 每用户 daemon、本地 IPC 与按需启动

工作内容：

- 用同一个 zterm 原生二进制提供用户命令与内部 daemon 模式，不增加第二个守护程序。
- 实现 zterm setup：确认设备名、生成 identity、写配置、初始化数据库并 detached-spawn daemon。
- 所有需要 daemon 的本地命令先连接用户 socket；socket 不存在或陈旧时使用 singleflight lock 拉起一次 daemon 后重试。
- Unix detached-spawn 关闭 stdin、重定向日志、setsid 并使用稳定 cwd；不创建任何启动项。
- 实现本地 IPC 的 request ID、deadline、取消和结构化错误；CLI 不直接读取 secret key 或 SQLite。
- 在 local IPC 定义统一 target selector 与本机 session control/attachment 消息；保留 `local` 直接路由到当前 daemon 的 SessionService，不解析为 device alias、不 self-dial Iroh。
- 实现 status、doctor、logs、daemon stop 的本地基础能力。
- stop/restart 前显示活动 session 数并要求明确确认；停止 daemon 可以结束 PTY。为 M9 暴露只限同用户 IPC 的 update preflight/stop 契约，但本里程碑不访问网络或替换程序。

完成标准：

- 多个并发 CLI 在 daemon 缺失时最终只产生一个实例。
- 关闭启动终端后 daemon 在受支持环境继续运行；doctor 能披露 logind 清理风险。
- 宿主重启后远端不可达，首次本地 zterm 命令自动拉起且无需重新配对。
- setup、运行、停止、升级全程不请求管理员权限，不写系统目录。
- repo 中没有 systemd/launchd/cron/Login Item、后台更新检查或自动安装实现。
- 外网、DNS/Pkarr 与 relay 不可用时，same-UID CLI 仍能通过 local target 完成 daemon readiness 和本机 session RPC；非 owner IPC 被拒绝。

验证：

    cargo test -p zterm-daemon --test local_ipc
    cargo test -p zterm-daemon --test single_instance
    cargo test -p zterm-cli --test daemon_autospawn
    cargo test -p zterm-cli --test setup_permissions

### M4. 持久 Session、PTY 与权威 VT 引擎

状态：**已完成**。子任务 `08-22-persistent-session-local-attach` 的本地完整门禁、独立 checker
和 GitHub Actions run `32570831589`（macOS arm64/Intel、Linux x86_64/arm64、Windows
shared/unsupported boundary）均绿色；这里只完成 transport-independent SessionService 与
same-UID local attachment，M5–M8 的远端 transport 和最终 CLI 仍保持未完成。

工作内容：

- 实现每个 session 一个 SessionActor，串行拥有 PTY、TerminalModel、revision、attachments、controller lease 和资源计数。
- 抽取唯一内部 `SessionService`/SessionActor 入口，让 remote QUIC adapter 与 local IPC self-target adapter 共用 list/create/attach/rename/close/takeover、snapshot/delta、input/resize 和结束原因；禁止复制第二套 local session registry。
- 建立稳定 main 默认 session；首次 attach 可按契约创建 main，后续可 new/list/attach/close 其他 session。
- 使用账户 login shell；默认 cwd 为 home，允许 session new --cwd，拒绝无效或无权目录。
- PTY reader 从创建到 root child exit 始终排空，与 attachment 数无关。
- 每次 PTY 输出先进入 TerminalModel，再生成有序 revision/delta；慢客户端不得反压 PTY reader。
- 新 attach 或 revision gap 发送权威 snapshot；客户端应用后发送 `SnapshotApplied(revision)`，服务端确认同步水位后再接受 input/resize。同步期间的普通输入直接丢弃，不排队、不在稍后重放；本地 detach 仍可用。
- 使用 Gate 0 已实测固定的准入值：2,000 scrollback rows/session、最多 8 session、最大 240x80、128 MiB summed fixed-cell projection；无有效初始 viewport 时使用 120x40，进程 RSS 测量目标保持 256 MiB。
- 实现一个 controller/session、显式 takeover、旧 controller 失效；内部 attachment map 为未来 observer 保留扩展形状。
- close session 是显式破坏性操作；idle、client disconnect、Iroh reconnect、无 controller 均不自动 close。

完成标准：

- 长任务在所有 client 断开后继续，稍后 attach 得到正确当前屏幕。
- main、alternate screen、连续 resize、Unicode 和高输出恢复测试通过。
- 多 session 互不混流，close 只终止目标 session；daemon stop/restart 终止所有 session 的已知边界有测试。
- controller takeover 不产生双写；陈旧 lease 的输入被拒绝。
- 内存 governor 在压力下拒绝新资源或裁剪已定义 scrollback，不 OOM、不写磁盘。

验证：

    cargo test -p zterm-daemon --test session_lifecycle
    cargo test -p zterm-daemon --test terminal_recovery
    cargo test -p zterm-daemon --test controller_lease
    cargo test -p zterm-daemon --test session_limits
    cargo test -p zterm-daemon --test tmux_herdr

### M5. Iroh Endpoint 与 Connection broker

工作内容：

- 按 design.md 建立默认和 self-host profile，只允许显式 relay URL 列表。
- 运行 n0 DNS/Pkarr publisher、resolver 与 DNS address lookup；默认只发布 home relay。
- 为每个 remote EndpointId 建立一条活动主 connection，提供并发 dial singleflight、确定性重复连接胜者和退避重连。
- 在一条 connection 上分发短 control RPC stream、pairing stream 和 attachment 双向 stream；只保留未来 `DEVICE_EVENTS` capability/stream kind 的升级边界，第一阶段不实现长生命周期 event stream。
- 缓存最近成功的 relay route，但不把 direct IP 写入长期公开记录。
- 暴露 path events 给 status/doctor/metrics；协议正确性不依赖路径标签。
- 对 connection、stream、RPC 并发、queue 和 frame 设界限；一个恶意 stream 不能阻塞其他 session。
- 对未授权 connection 和 pairing handshake 设全局/单 EndpointId 并发上限、首帧 deadline 与总字节上限，超限不能占住 session/PTY actor。

完成标准：

- 同一设备对的多个本地 CLI 和多个 session 只产生一条稳定 Iroh connection。
- 重复入站/出站竞态收敛到同一连接，失败方不误杀胜者。
- relay/direct 路径变化不改变 session/attachment identity。
- DNS 查询失败时，有有效 ticket/cache route 的场景仍能尝试项目 relay；不存在公共 relay 隐式回退。

验证：

    cargo test -p zterm-daemon --test connection_broker
    cargo test -p zterm-daemon --test duplicate_connection
    cargo test -p zterm-daemon --test stream_limits
    cargo test -p zterm-daemon --test path_migration

### M6. 配对、认证与撤销

工作内容：

- 实现 PairTicketV1 文本编码：host EndpointId、设备名、relay hints、128-bit offer ID、256-bit secret、到期时间和格式版本。
- offer 默认 10 分钟、一次性；只保存 secret verifier/必要状态，不在日志显示完整 ticket。
- 在 zterm-pair/1 ALPN 上以 HMAC-SHA256 绑定 transcript、双方 EndpointId、offer ID、nonce 与版本，完成双方持有证明。
- 实现 SSH-like 单向授权：票据创建宿主只持久化接收方的 AuthorizedDevice，接收方只持久化宿主 known device/route，不写入反向授权。宿主执行 `pair create` 和接收方主动导入票据就是双方显式意图，不再叠加一个只能在宿主本地操作的二次确认。普通 zterm/1 connection 每次建立和每个敏感 RPC 都校验 EndpointId 与 authorization generation。
- 实现 device list、rename 和 revoke。
- revoke 严格执行：持久化 tombstone/generation → 拒绝新操作 → 关闭全部该设备 connection/stream → 释放 attachment/controller；PTY 保留。
- 处理 revoke 与 reconnect、new stream、takeover 同时发生的竞态。

完成标准：

- 过期、已消费、篡改、重放、错误 EndpointId 和错误 secret 的 ticket 均失败且不授权。
- A 创建票据、B 导入后只生成 `A authorizes B`；没有反向票据时 A 不能控制 B，两个方向的 revoke 互不影响。
- 未授权 EndpointId 即使能使用公开 relay 也不能调用任何终端 RPC。
- revoke 返回成功后，旧连接、排队 RPC 和竞态重连都不能继续访问；daemon 重启后仍被撤销。
- 撤销一个控制设备不结束 session，不影响其他已授权设备的未来连接。

验证：

    cargo test -p zterm-core --test pairing_vectors
    cargo test -p zterm-daemon --test pairing_protocol
    cargo test -p zterm-daemon --test authorization
    cargo test -p zterm-daemon --test revoke_races

### M7. 远程 Session 协议与断线恢复

工作内容：

- 实现 session list/new/attach/rename/close、takeover、resize、input、`SnapshotApplied`、detach 与结构化错误。
- create/rename/close/takeover 等状态变更使用 operation ID 去重；覆盖“宿主已提交、响应丢失、客户端重试”。
- attachment handshake 携带 session ID、last applied revision、期望尺寸和 controller 请求。
- last revision 可连续时从 bounded delta window 恢复；否则发送 snapshot。任何 gap、queue overflow 或 model reset 都强制 resync。
- connection 断开只销毁 attachment；session actor、PTY 与 TerminalModel 继续。
- 重连后 local daemon 用现有 connection broker 恢复用户期望的单个 CLI attachment；不会自动恢复已经退出的 CLI 进程。
- 记录未来 GUI/Android 的 revision-only 列表兼容契约：冷 tab 不接收 terminal bytes，切换时重新 attach/snapshot；不提前实现其 event stream 或具体消息。
- 冷 tab/移动 App 后台 detach 完整 terminal attachment 并释放 controller lease；本机 attachment 没有隐式优先级，远端 controller 仍在线时普通 local attach 返回 occupied，显式 takeover 才转移。
- 第一阶段 snapshot 只携带有界近期历史；独立 history paging RPC 延后到 Android/GUI 的可选 `HISTORY_PAGING` capability。
- 保留未来 observer capability，但 v1 对第二个 controller 只返回占用信息或执行显式 takeover。

完成标准：

- 网络抖动、进程退出、路径切换、relay/direct 变化均不结束 session。
- revision gap 永不以猜测补齐；snapshot ACK 前不会把输入应用到未知屏幕。
- 多 session 使用独立 stream；一个 session 的大输出不造成其他 session 全局队头阻塞。
- 手机和 same-UID 本地 CLI 先后 attach 时观察同一个 SessionId、PTY、cwd、TerminalModel revision 与 scrollback；local path 不产生 Iroh connection、DNS/Pkarr 查询或 relay 流量。
- 协议 fixture 可被未来 Kotlin/Swift/Windows 客户端独立实现，不依赖 Rust enum 内存布局。

验证：

    cargo test -p zterm-proto --test compatibility
    cargo test -p zterm-daemon --test reconnect
    cargo test -p zterm-daemon --test snapshot_resync
    cargo test -p zterm-daemon --test multi_session_streams

### M8. CLI 产品面

工作内容：

- 实现 setup、pair create/accept、device list/rename/revoke、connect、session list/new/attach/rename/close、status、doctor、logs、daemon status/stop/restart 与 `reset --identity`；`connect/session` 的 device selector 接受保留值 `local`。
- `pair accept` 默认从不回显的交互式 TTY prompt 读取 ticket，自动化显式使用 stdin；不提供 ticket 作为位置参数，不让它进入 argv、shell history、日志或错误文本。
- connect 默认 attach main；每个交互 CLI 进程只显示一个 session，切换通过退出当前 attach 后执行另一个命令完成。
- `zterm connect local` 与 `zterm session ... local` 只经 local IPC；`local` 不能注册为设备 alias。setup 后裸 `zterm` 等价于 `zterm connect local --session main`；未 setup 时只提示 `zterm setup`，不静默创建身份；`zterm --help` 保持帮助入口。
- attachment 开始时即在 RAII guard 下进入 raw mode；同步期 drain/丢弃普通按键但仍处理本地 Ctrl+] 前缀，发送 `SnapshotApplied` 后才转发按键与最新 resize。前缀提供 detach、帮助、takeover 和必要控制动作。
- 使用 RAII/signal handling 保证正常退出、错误、Ctrl-C 和终止信号都恢复本地 TTY。
- 所有本地 CLI 经 IPC 复用 daemon 持有的 remote connection，不自行创建 Endpoint 或读取 key。
- 人类输出保持简洁，status/doctor 提供稳定 JSON 供诊断；错误区分本地 daemon、寻址、授权、relay、协议与 session 状态。

完成标准：

- 用户按 PRD 流程可在两台真实受支持的 macOS/主流 glibc Linux 机器安装、setup、配对、连接、创建/切换/关闭 session。
- CLI 异常退出后本地终端属性恢复，远端 session 保留。
- 两个 CLI 同时连接不同 remote session 时共享一条 Iroh connection。
- 另一台桌面 CLI 远程创建并使用的 session 在宿主本机 `connect local` 后保持相同 SessionId、进程、cwd 和 screen；本机创建的 session 也立即出现在远端列表。remote controller仍在线时 local普通 attach不打断它，`--takeover` 才切换；第二阶段 Android 复用并实机验收同一契约。
- 裸 `zterm`、显式 `zterm connect local` 和 `zterm --help` 的 setup前后行为分别通过 CLI fixture 固定，错误路径不启动半初始化 daemon或创建identity。
- Ctrl+] 不误传远端；普通 Ctrl-C/Ctrl-Z 等仍进入 PTY。

验证：

    cargo test -p zterm-cli
    cargo test -p zterm-cli --test raw_mode_restore
    cargo test -p zterm-cli --test control_prefix
    cargo test -p zterm-cli --test local_self_attach
    cargo test -p zterm-cli --test bare_entrypoint
    cargo test -p zterm-cli --test end_to_end

### M9. Direct installer、release 供应链与手动升级

工作内容：

- 为 `aarch64/x86_64-apple-darwin` 与 `aarch64/x86_64-unknown-linux-gnu` 生成可重复的原生压缩 artifact；名称、target、binary build ID、wire protocol 和 release version 由同一 release metadata 生成并交叉校验。
- 用项目官方 GitHub 仓库托管公开可审阅的 POSIX `install.sh`，用 GitHub Releases 承载稳定版和签名开发 prerelease。建立带 schema version、release version、classification、发布时间、target URL/size/SHA-256 的 versioned manifest、detached 签名与离线 fixture；release 同时生成 SBOM、checksums 和来源/provenance 说明。
- 实现 `install.sh` 的版本解析：无参数只解析 latest non-prerelease 稳定版，`--version <release-tag>` 精确解析已发布的稳定版或开发 prerelease；拒绝 draft、任意 branch/commit 和 GitHub Actions artifact。严格 target/glibc 检测、超时和大小限制、临时目录、签名/checksum 与候选自检、同文件系统原子安装。默认使用 `~/.local/bin`，允许当前用户可写目录；不调用 sudo、不改 shell rc，只给 PATH 指引。
- installer 不运行 setup、不生成 identity/config/state、不启动 daemon、不注册服务；发现受管理的现有安装时拒绝盲目覆盖并引导 `zterm update`。提供下载后审阅脚本和 versioned release 手工 checksum 安装文档，明确 `curl | sh` 的 HTTPS bootstrap 信任边界。
- 实现显式 `zterm update`：无参数选择 latest stable，`--version <release-tag>` 精确选择签名稳定版或开发 prerelease；用编译内置公钥验证 manifest 签名，再校验 artifact size/SHA-256、target、版本与候选自检。普通配置不能替换受信更新源，第一阶段不自动跟随开发/nightly channel。
- updater 必须在停止 daemon 前完成下载与全部可完成验证；随后通过 M3 IPC 读取活动 session。交互模式列出影响并确认，非交互无 `--force` 时失败；确认后停止 daemon 和 PTY，保留旧 binary，`fsync` 并原子激活新 binary，运行 post-activation self-check，失败自动恢复旧 binary。
- 更新成功不自动启动 daemon；下一条需要 daemon 的本地命令按需拉起。手工替换或回滚产生版本不匹配时只诊断并拒绝不兼容协议，不静默杀进程或降级 schema。
- 把 macOS/Linux 全部持久状态集中到 `~/.zterm/`，建立 `0700` 根目录、`0600` identity/config/database/install metadata 和受限滚动日志；runtime socket 继续使用平台安全临时目录并验证同 UID。
- 实现 Unix `zterm uninstall` 的受管理删除路径：明确展示活动 session、身份销毁和重新配对影响，交互确认；非交互必须 `--yes`，有活动 session 时还需 `--force`。停止 daemon 后删除整个 `~/.zterm/`，再删除 binary；中途失败可重试。跟随 Zedra 边界，不发送卸载前 `RevokeSelf`、不访问中央撤销服务；保留独立 `zterm reset --identity` 供不卸载程序的身份轮换，泄露凭证由逐宿主 device revoke 处理。
- 为未来 Windows 把 staging、verification、activation 抽象分离；第三阶段可用版本目录 + `current` 指针替换 Unix rename，而不改变 manifest 或 update UX。

完成标准：

- 干净 macOS x86_64/arm64 和主流 glibc Linux x86_64/arm64 普通账户可从本地测试 HTTPS release endpoint 完成安装、setup、更新、自动 binary 回滚和卸载，全程无权限提升、Node/npm 或 Rust 工具链。
- installer 选择正确 target 且只安装预期 binary；默认稳定版测试不会命中 prerelease，精确稳定/开发 tag 测试得到指定 build；安装前后扫描证明未创建 identity、运行配置、daemon 或启动项。Alpine/musl、NixOS 原生和不满足 glibc 基线的环境在下载 artifact 前返回可操作的 unsupported-platform。
- manifest 篡改、签名错误、checksum/size 错误、截断下载、错误 target、版本倒退和候选自检失败都在 daemon 停止前失败，当前 binary 与 session 保持不变。
- 有活动 session 时 update/uninstall 默认拒绝并准确列出影响；确认或 `--force` 后才结束 PTY。激活故障恢复旧 binary，下一次本地命令可重新启动旧 daemon，但已结束 PTY 不被报告为恢复。
- 旧 CLI/新 daemon 和新 CLI/旧 daemon 的 wire major 不兼容时明确拒绝，不产生数据损坏。setup 后全部持久数据只出现在 `~/.zterm/`；卸载确认后该目录与 identity/auth/config 均不存在，重新安装/setup 生成不同 EndpointId，旧配对连接失败并要求重新配对。
- release signing key rotation、受信来源变更、数据库 migration 后 binary rollback 与紧急手工安装都有书面操作边界和测试 fixture。

验证：

    shellcheck install/install.sh
    ./tests/install/release-artifacts.sh
    ./tests/install/release-selection.sh
    ./tests/install/manifest-authenticity.sh
    ./tests/install/clean-user-macos.sh
    ./tests/install/clean-user-linux.sh
    ./tests/install/manual-upgrade.sh
    ./tests/install/activation-rollback.sh
    ./tests/install/uninstall-identity-reset.sh
    ./tests/install/unsupported-platform.sh

### M10. 端到端、安全与发布验收

工作内容：

- 建立两 daemon、多 CLI、多个 session，以及手机模拟 controller → 宿主本机 CLI self attach 的 deterministic e2e harness。
- 在 Linux network namespace/容器网络构造 direct、双 NAT、relay-only、DNS/Pkarr 失败、relay 故障与网络切换；先在QAD关闭的默认部署上记录path events、打洞时延和direct成功率，只有证据表明QAD缺失是瓶颈时才提出独立QAD-only对照实验。
- 在真实受支持的 macOS/主流 glibc Linux 做 clean-account direct install、SSH 启动后退出、tmux、固定 Herdr、长任务断线恢复和手动升级/回滚验收；在 Alpine/NixOS 验证 installer 在 artifact 下载前明确拒绝。
- fuzz protobuf frame、ticket、控制前缀和 ANSI parser；执行 queue/frame/session/terminal size 资源攻击。
- 检查 secret/log redaction、ticket 不进入 argv/history、文件权限、peer UID、symlink、未授权 EndpointId、未认证连接限制、revoke 竞态与 relay 抓包。
- 在禁用外网、DNS/Pkarr 与 relay 的条件下验证 local self attach；抓包与 Iroh instrumentation 断言不存在到自身 EndpointId 的 connection attempt。
- 注入未识别 OSC/DCS/APC、OSC 52 和图形序列，验证不会绕过 TerminalModel 驱动本地终端；模拟状态变更已提交后响应丢失，验证 operation ID 重试无重复副作用。
- 对照 PRD A-E 逐项保存证据；补齐用户文档、运维文档、已知限制和故障排查。

完成标准：

- PRD 所有第一阶段验收项有自动化结果或明确的真实平台人工证据。
- direct 优先和 relay fallback 均被路径事件证明；测试断言不存在公共 Iroh relay。
- 测试证据明确区分QAD地址发现、NAT打洞和relay密文转发；不把QAD当作fallback依赖，也不在实测前预设需要QAD-only服务。
- 云端只能观察必要元数据与密文，不能解密终端数据；terminal 内容不在 daemon/relay 持久存储。
- daemon 或手动升级中断 PTY、无开机自启、Everyone/no-limits、dns.iroh.link 无 SLA 等限制在发布文档醒目披露。
- 所有质量门通过，无已知高危漏洞、secret 泄漏、无限资源增长或静默协议降级。

最终验证：

    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-features
    cargo test --doc --workspace
    cargo deny check
    shellcheck install/install.sh
    ./tests/install/release-artifacts.sh
    ./tests/install/manifest-authenticity.sh
    docker compose -f deploy/relay/docker-compose.yml config
    ./tests/e2e/run-network-matrix.sh
    ./tests/e2e/run-security-matrix.sh

## 5. PRD 追踪

| PRD 范围 | 实施里程碑 | 主要验收证据 |
| --- | --- | --- |
| R0 第零阶段环境与基础设施门 | Z0-A、Z0-B | 工具链探测/bootstrap、本地relay smoke、人工连接检查点与公网一次验收证据 |
| R1 每用户非特权 daemon | M2、M3、M9 | 权限测试、clean-user direct install、本地 IPC peer UID |
| R2 daemon 生命周期 | M3、M4、M9 | 按需拉起、无启动项扫描、手动升级中断测试 |
| R3-R4 云端、直连、relay、地址查询 | Z0-B、M1、M5、M10 | path events、network matrix、relay 配置与抓包 |
| R5 设备身份、配对、撤销 | M2、M6、M10 | ticket vectors、重放/过期、revoke race 与持久化 |
| R6 持久 Session/PTY | M1、M4、M7、M10 | 长任务断线、idle、close、daemon restart |
| R7 权威终端状态与兼容性 | M1、M4、M7、M10 | ANSI corpus、snapshot/delta、tmux/Herdr |
| R8 connection/stream/attachment/控制权 | M3、M4、M5、M7、M8、M10 | 单 connection、多 stream、same-UID self attach、takeover、冷 tab 与未来 event capability 契约 |
| R9 后续客户端兼容 | M2、M4、M7 | protobuf fixture、平台抽象、revision-only 事件 |
| 验收 Z、A-E | Z0-A、Z0-B、M10 | 第零阶段证据与发布前逐项 evidence checklist |

## 6. 数据迁移、兼容与回滚

- 新项目无旧 zterm 数据迁移；旧 zterm_old 的数据、协议、账号和配置均不读取。
- 每次 SQLite schema 变更先备份并在单事务迁移；失败保持旧 schema。不可逆迁移必须有恢复备份的演练。
- wire v1 仅做向后兼容字段新增；不兼容变更使用新 ALPN major。任何一端不支持时明确报错。
- identity key 与授权数据不随二进制回滚隐式删除或重建。
- relay 无 session 业务状态；运行异常直接recreate，只有实际镜像缺陷才由运维人员手动选择上一版本tag。
- profile 更新采用校验后原子替换；错误配置不能覆盖上一份已知可用配置。
- 每个子任务在进入下一个依赖里程碑前形成可运行检查点；回退只撤销该子任务新增行为，不破坏已验证的协议和持久状态。

## 7. 发布前定义完成

只有同时满足以下条件，第零阶段和第一阶段才分别算完成：

- Z0-A/Z0-B 已在第一阶段前完成；M1-M10 的完成标准全部满足，Gate 0 结论仍适用于最终依赖版本；
- PRD A-E 无遗漏，并有 CI、测试日志或真实平台验收记录；
- 默认 relay 已通过手动`pull`/`up -d`部署`latest`并完成一次health/authenticated-handshake验收，self-host反代文档可从零复现；
- macOS x86_64/arm64 与主流 glibc Linux x86_64/arm64 四个目标的原生 artifact 可由官方 installer 安装，显式 `zterm update` 与失败回滚通过验收，且安装/运行不要求管理员权限；
- 断线持久、终端恢复、远端桌面CLI↔宿主本机接续同一session、单connection多session、revoke和E2EE的关键主张均有端到端证据；第二阶段Android沿用该接续契约；
- 文档没有把 1.0 描述成开机自动启动、daemon crash 自动恢复、完整 transcript 保存或专有 Agent 平台；
- 代码检查与独立 review 通过，未解决风险只允许是 PRD 已明确接受并在文档披露的限制。
