# zterm 第零与第一阶段技术设计

> **2026-08-21 后续决策（覆盖本文旧的 `Minimal + Custom relay` 默认设计）：** 第一阶段产品 profile 使用 Iroh 1.0.3 官方 n0 生产 Relay/QAD 和生产 DNS/Pkarr；自建 Relay 只保留为显式可选部署。当前代码边界、A/B/C 结果与停止线见 `08-21-foundation-gate` 和 `.trellis/spec/backend/relay-deployment.md`。

## 1. 设计目标与不变量

本设计先覆盖第零阶段的开发机 bootstrap 与单节点上游 relay 公网部署，再覆盖第一阶段 macOS x86_64/arm64、主流 glibc Linux x86_64/arm64 daemon + CLI；从第一天保留 Android、Windows、桌面 GUI、iOS、多观察端和 2.0 Agent 事件的兼容边界。

不可破坏的六个不变量：

1. **Session 不属于 connection**：任何 transport、stream 或控制端消失都不能进入 PTY 终止路径。
2. **授权以宿主本地 EndpointId 为准**：relay、DNS/Pkarr、票据字符串和 session ID 都不是长期授权真相源。
3. **一对设备一条主 connection**：多个 session/attachment 用独立 QUIC stream 复用，慢 stream 不阻塞其他工作。
4. **宿主拥有权威 VT**：断线恢复来自当前 snapshot + revision 增量，不依赖客户端仍保留旧 parser，也不依赖完整原始字节回放。
5. **当前用户权限是唯一 OS 权限边界**：无 root daemon、无跨用户代理、无虚假的 per-session 安全隔离。
6. **本机与远端是同一 session**：桌面 local view 与 Iroh remote attachment 必须落到同一个 SessionActor、PTY、VT revision 和 controller lease；不能靠复制 terminal 或依赖外部复用器实现接续。

## 2. 总体架构

```text
┌──────────────────────────── 控制设备（当前 OS 用户） ────────────────────────────┐
│ zterm CLI A ─┐                                                               │
│ zterm CLI B ─┼─ user-only local IPC ─> zterm daemon / connection broker      │
│ future GUI ──┘                         ├─ self target ─> local SessionRegistry │
│                                       └─ remote target ─> Iroh connection pool│
└───────────────────────────────────────────────┬────────────────────────────────┘
                                                │ EndpointId-authenticated QUIC
                                                │ direct preferred / relay fallback
                                                │ control + attachment (+ future event) streams
┌──────────────────────────── 宿主设备（当前 OS 用户） ────────────────────────────┐
│ zterm daemon                                                                  │
│  ├─ authorization / pairing / persistent state                                │
│  ├─ connection registry                                                       │
│  └─ session registry                                                          │
│       ├─ Session main   = PTY + login shell + authoritative VT + revision      │
│       ├─ Session build  = PTY + login shell + authoritative VT + revision      │
│       └─ Session review = PTY + login shell + authoritative VT + revision      │
└────────────────────────────────────────────────────────────────────────────────┘

dns.iroh.link: signed EndpointId → home relay lookup only
zterm/default or self-host relay: address discovery + opaque E2E ciphertext forwarding only
```

桌面 daemon 同时承担宿主和控制侧 broker。CLI 永远不直接读取设备私钥或单独创建 Iroh Endpoint；这样同一用户的多个 CLI 才能真正共享远端连接，也能直接 attach 本机 daemon 持有的 session。self target 只经同 UID local IPC 进入同一个 SessionRegistry，不 self-dial Iroh、不查询地址服务、不经过 relay，也不把自己加入 `device_auth`。Android/iOS 没有独立 daemon，由单个 App 进程承担 controller broker 职责。

## 3. 第零阶段与 Rust workspace 结构

### 3.1 当前环境与本机 bootstrap

2026-08-21 的最新只读探测基线是 Apple Silicon macOS 26.6.2，已有 Homebrew 6.0.18、Xcode/clang 21、Git 2.50.1、CMake 4.4.2、rustup 1.29.0 与 stable Rust/Cargo 1.98.0；rustfmt 1.9.0、Clippy 0.1.98、rust-analyzer及aarch64 Apple/iOS/Android targets均已安装，Docker、`protoc` 和 `pkg-config` 当前不可用。第零阶段以“探测后补齐”为原则：不再安装Rust 1.91，也不重装rustup；`rust-toolchain.toml`精确固定1.98.0，补齐`pkg-config`与固定Cargo质量工具。Protobuf使用仓库固定的生成器/预编译产物，不要求最终用户安装系统`protoc`。

`1.98.0`是项目当前可复现的开发/release工具链，不把浮动`stable`写入仓库。以后采用新版stable时，提交显式toolchain变更并重跑全部质量、兼容、release artifact与Gate 0相关门禁；Iroh 1.0.3的Rust 1.91 MSRV仅作为依赖事实，不形成zterm的第二条CI工具链。

当前Mac默认选择Homebrew Docker CLI/Compose + Colima作为本地Linux容器运行时，避免把Docker Desktop变成默认前提；实际安装前若用户指定Docker Desktop，只替换本机runtime，不改变Compose或服务器产物。bootstrap脚本/文档必须可重复执行，所有系统修改和最终版本都有记录。

### 3.2 公网 Relay 人工检查点

第零阶段先在本机构建并验证`deploy/relay`，然后硬停止并向用户报告已经到达公网部署步骤。此时才接收SSH入口、登录/认证方式、relay域名与DNS状态；不要求用户把私钥或token复制进仓库。获得方式后先执行只读preflight，确认服务器OS/架构、Docker/Compose、DNS、端口占用与防火墙可达性。已有Docker和网络条件满足时可部署；若需要安装系统软件、修改防火墙或处理端口冲突，先展示精确变更影响。

当前完成条件是 GitHub Release `vX.Y.Z` 直接发布同名 GHCR tag 与 `latest`，默认服务器通过显式 `docker compose pull` / `up -d` 更新后，只执行一次宿主 health、公开 HTTP 与 authenticated Iroh Relay 握手验收。Compose只监听宿主回环38451，使用Docker `local`日志驱动，不开放metrics、UDP/QAD，也不把digest或回滚演练作为Gate；第一阶段Gate 0必须使用这套relay，不临时依赖公共Iroh relay，并单独验证真实NAT直连。

### 3.3 仓库结构

```text
Cargo.toml
rust-toolchain.toml             # 精确固定 Rust 1.98.0；升级必须显式评审
crates/
  zterm-core/                   # 纯领域类型、session/auth 状态机、无 Iroh/CLI 依赖
  zterm-proto/                  # .proto 生成物、frame codec、版本与限制
  zterm-platform/               # 路径、用户身份、PTY、login shell、detach、local IPC
  zterm-daemon/                 # state store、Iroh、pairing、connections、sessions
  zterm-cli/                    # clap 命令、raw TTY、终端呈现；产出 zterm 二进制
proto/zterm/v1/                 # 跨语言 Protocol Buffers source of truth
deploy/relay/                   # Dockerfile、Compose、relay.toml、运维文档
install/                        # 官方 shell installer、manifest schema 与安装测试 fixture
tests/e2e/                      # 多进程、NAT/relay、PTY 与安全验收
xtask/                          # 生成、打包、发布与验收辅助
```

第一阶段对用户只发行一个原生 `zterm` 可执行文件。daemon 是同一文件的内部 `zterm daemon run` 模式，不发行第二个 supervisor；各 crate 是代码边界，不代表额外常驻进程。

核心依赖基线：Iroh/iroh-relay 1.0.3、Tokio、portable-pty 0.9.x、prost 0.14.x、rusqlite（bundled SQLite）、clap、tracing。VT 候选暂定 vt100 0.16.2，但必须先通过第 15 节的兼容技术门。

## 4. 本地目录、持久状态与权限

### 4.1 路径

通过 `zterm-platform` 集中解析平台目录，不在业务模块散落 `$HOME` 拼接。用户已选择一个跨桌面平台可预测的持久根目录：

- macOS/Linux：`~/.zterm/`
- Windows（第三阶段）：`%USERPROFILE%\.zterm\`
- Android/iOS：应用私有 sandbox 中的等价内部结构，不承诺可见的 home 路径。

第一阶段目录至少包含 `config.toml`、`identity.key`、`state.sqlite3`、`install.json` 与 `logs/`。根目录权限为 `0700`，普通持久文件不宽于 `0600`；Windows 阶段以当前用户 ACL 表达相同边界。

- runtime socket：Linux 优先使用经 ownership 校验的 `$XDG_RUNTIME_DIR/zterm/daemon.sock`；macOS 优先使用 `$TMPDIR/zterm-<uid>/daemon.sock`；回退目录必须短、由当前 UID 拥有且权限 `0700`。

runtime socket 是可重建的进程间通信端点，不是配置文件，因此不要求位于 `~/.zterm/`。socket 目录为 `0700`，socket 只允许当前用户访问，并在 Linux 用 `SO_PEERCRED`、macOS 用 `getpeereid` 再核对 peer UID。单实例锁放在稳定 state 目录；遗留 socket 只有在锁与连接探测都证明无存活 daemon 后才清理。

### 4.2 文件与数据库

- `identity.key`：Iroh 长期 SecretKey，创建即 `0600`，采用 create-new + 原子 rename；永不记录到日志。
- `config.toml`：版本化基础设施 profile、设备显示名、CLI 设置和资源上限，不含长期私钥或配对 secret。
- `state.sqlite3`：bundled SQLite，由 daemon 内单一 store actor 访问。安全状态变更使用事务和 `synchronous=FULL`。
- 活动 PTY、VT screen、scrollback、terminal input/output 和配对 offer 只存在内存，不进入数据库。

最小表模型：

```text
metadata(schema_version, device_name, created_at)
device_auth(endpoint_id PK, display_name, status, generation, paired_at,
            revoked_at, last_seen_at)
known_devices(endpoint_id PK, local_alias UNIQUE, remote_name,
              cached_relay_routes, routes_verified_at)
```

`device_auth` 保留 revoked tombstone 与单调 generation，而不是物理删除后把代数归零；这使旧 connection/stream 即使与同一公钥后来重新配对，也无法复用旧授权 lease。第一阶段不额外建立持久审计事件表；配对、撤销和迁移诊断进入有保留上限的结构化日志，且不记录命令、按键、终端内容、cwd 或完整票据。

active session 不持久化。daemon 重启后 session registry 从空状态开始；下一次裸连接重新创建名为 `main`、但具有新 session ID 的 PTY，不展示幽灵 session。

## 5. 安装、setup 与 daemon 生命周期

### 5.1 官方 direct installer

第一阶段只维护一个由项目 GitHub 仓库与 GitHub Releases 承载的官方 direct-install 渠道，不同步维护 npm、Homebrew、mise 或 Nix 包。每个稳定版和可安装开发版都发布以下四个原生压缩 artifact：

```text
zterm-aarch64-apple-darwin.tar.gz
zterm-x86_64-apple-darwin.tar.gz
zterm-aarch64-unknown-linux-gnu.tar.gz
zterm-x86_64-unknown-linux-gnu.tar.gz
```

项目官方 GitHub 仓库提供公开可审阅的 `install.sh`，GitHub Release 提供 versioned manifest、签名和 release artifact。manifest 至少包含 schema version、release version、channel/classification、发布时间、每个 target 的 URL、字节大小与 SHA-256；发布页同时提供 checksum、SBOM 和来源/provenance 信息。未来可以增加项目域名作为到 GitHub 的稳定入口，但不是第一阶段部署依赖，URL 也不进入 wire 或设备身份。

版本选择契约：

- `install.sh` 无参数时使用 GitHub 的 latest non-prerelease release，即最新稳定版。
- `install.sh --version <release-tag>` 精确选择一个 GitHub Release；tag 可以是稳定 SemVer，也可以是形如 `v1.1.0-dev.20260821.<shortsha>` 且标记为 prerelease 的开发版。
- 开发版必须经过与稳定版相同的签名、checksum、target 与候选自检；任意 branch/commit 的临时构建和 GitHub Actions artifact 不是安装源。
- installer 发现受管理的已有安装时不覆盖，而是引导 `zterm update [--version <release-tag>]`。无参数 update 回到/跟随最新稳定版；开发版只能显式选择，不建立自动 nightly 行为。

Unix installer 的顺序固定为：

1. 由 `uname`、glibc 探测和已知不支持环境判断精确 target；在下载前明确拒绝 Alpine/musl、NixOS 原生环境、未知 OS/arch 或低于最终 glibc 基线的系统。
2. 经 GitHub HTTPS 获取有大小与超时限制的目标 release manifest，在临时目录下载对应 artifact，校验 manifest 签名、长度和 SHA-256，再解压并运行无副作用的 `zterm --version`/artifact self-check。
3. 默认安装到当前用户的 `~/.local/bin/zterm`，允许显式指定其他当前用户可写目录；在目标目录创建同文件系统临时文件、设置权限、`fsync` 后原子 rename。installer 不调用 `sudo`、不自动修改 shell rc；目录不在 `PATH` 时只输出可复制的设置说明。
4. installer 不运行 `setup`、不生成 identity/config/state、不启动 daemon、不注册服务。发现由 zterm 管理的现有安装时不直接覆盖，而是引导用户运行 `zterm update`。

初次执行 `curl | sh` 的 bootstrap 信任边界是 GitHub HTTPS、项目仓库和当次脚本；checksum 防止下载损坏，但不能在 bootstrap 脚本本身失陷时建立独立信任。文档同时提供先下载再审阅脚本、从 immutable versioned Release 手工安装并核对 checksum/签名的路径。已安装的 `zterm update` 使用编译进二进制的 release 公钥验证稳定版与开发 prerelease 的 manifest 签名，签名公钥不从待验证的网络响应动态取得；密钥轮换必须由旧受信密钥签署过渡信息或要求明确手工重装。

### 5.2 setup 与按需拉起

`zterm setup` 顺序：

1. 校验当前安装路径、数据目录与权限。
2. 交互确认设备名和基础设施 profile。
3. 在不存在时创建长期身份与数据库；已存在时只做幂等校验。
4. 写入配置并运行 doctor 的关键检查。
5. 调用统一 `ensure_daemon()`，等待 local IPC readiness 后成功返回。

所有需要 daemon 的本地命令都先调用同一个 `ensure_daemon()`：探测 socket；不存在时尝试取得 spawn lock；启动子进程并等待 readiness；未取得锁的并发调用只等待，不再 spawn。

Unix detached-spawn 将 stdin 连接 `/dev/null`，stdout/stderr 追加到 daemon log，在 child `pre_exec` 中调用 `setsid()`，并使用确定的工作目录。它只解决控制终端/SIGHUP，不宣称对抗 systemd-logind 整个用户 cgroup 的清理。

### 5.3 停止与手动升级

- `zterm daemon stop|restart` 先通过 IPC 列出活动 session；非空时交互确认，非交互必须 `--force`。
- 第一阶段不做后台版本检查、下载或安装。`zterm update` 是唯一官方更新入口，只有用户显式执行时才访问 GitHub Release metadata；无参数只选 latest stable，`--version` 可以精确选择签名稳定版或开发 prerelease。普通配置不能把受信更新源指向任意 URL，开发版也不会被后台或默认路径选中。
- updater 先在与安装目标相同的文件系统 staging 候选，验证 manifest 签名、artifact size/SHA-256、目标平台、版本单调性和候选自检。上述任一步失败都发生在 daemon 停止前，因此不影响 session。
- 候选就绪后通过本地 IPC读取 daemon build/protocol 和活动 session。存在活动 session 时列出数量与名称并确认“这些 PTY 将结束”；stdin 非交互时没有 `--force` 就失败。确认后有界通知 attachment、终止 PTY 根进程组并等待 daemon 完整退出。
- updater 保留当前二进制作为本地回滚候选，再以同目录临时文件 + `fsync` + 原子 rename 激活新版本并运行无状态 post-activation self-check。激活失败自动恢复旧二进制；session 已经结束，不声称恢复进程。成功后不自动拉起 daemon，下一条需要 daemon 的本地命令按既有 `ensure_daemon()` 路径启动新版本。
- 通过手工替换、恢复备份或崩溃形成新旧 CLI/daemon 不一致时，握手只给出可操作的 stop/restart/reinstall 诊断并拒绝不兼容协议，不能自动降级 schema 或静默杀进程。
- `zterm uninstall` 使用同样的活动 session 确认边界，并明确警告将结束 PTY、删除设备身份且所有旧配对都需重建。交互流程必须确认；非交互流程必须显式 `--yes`，有活动 session 时还需 `--force`。实现先停止 daemon，再删除整个 `~/.zterm/`（包括 `identity.key`、宿主授权、known devices、配置、安装元数据与日志），最后移除受管理程序文件；流程可安全重试，不能先删掉唯一可完成清理的可执行文件。
- restart 先有界通知 attachment，再终止 PTY 根进程组、等待短暂 grace period，最后强制回收；本地记录结束原因，但不承诺任务恢复。
- `zterm reset --identity` 仍可作为不卸载程序时主动轮换本机身份的独立破坏性命令，采用同一警告与二次确认边界。

删除 `identity.key` 保证这次安装和正常重装不能再证明旧 EndpointId；宿主侧删除自己的 `state.sqlite3` 也清空它曾授予其他设备的权限。它不等于中央全局撤销：另一宿主仍可保留对旧 EndpointId 的 stale authorization；只要旧私钥没有副本就无法再利用，但被复制或备份的旧私钥仍必须在每台宿主上分别撤销。若产品要求即使私钥副本存在也能在一次卸载后全局失效，就需要新增当前明确排除的在线控制平面或逐宿主撤销协调。

用户已确认跟随当前 Zedra `a30bc6c` 的跨平台一致模型：host/App 卸载只销毁本机身份，正常重装后重新配对；1.0 不实现卸载前 best-effort `RevokeSelf`，也不增加中央撤销服务。设备/私钥疑似泄露时，用户在每台曾授权该 EndpointId 的宿主上执行既有 device revoke；该操作仍按第 7.3 节立即持久化并切断在线连接。

## 6. Iroh Endpoint 与基础设施 profile

第一阶段固定 Iroh 1.0.3。Endpoint 从 `presets::Minimal` 构建：

1. 载入持久 SecretKey。
2. 用 `RelayMode::Custom` 加入 profile 中的项目或用户 relay URL；绝不混入 Iroh public relay map。
3. 加入 `PkarrPublisher::n0_dns()`、`PkarrResolver::n0_dns()` 和 `DnsAddressLookup::n0_dns()`；默认 publisher 保持 relay-only address filter。
4. 注册正常协议 ALPN `zterm/1` 与配对 ALPN `zterm-pair/1`。
5. 等待 home relay online，并把发布/解析/relay/path 状态暴露给 status/doctor。

配置概念模型：

```toml
version = 1
active_profile = "default"

[profiles.default]
relay_urls = ["https://relay.example.com"]
allow_public_iroh_relays = false       # v1 必须为 false

[profiles.default.address_lookup]
kind = "n0-dns-pkarr"
publish_direct_addresses = false       # v1 必须为 false
```

代码只依赖结构化 `InfrastructureProfile`，`dns.iroh.link` 的 publisher、HTTP resolver 与 DNS origin 常量集中在一个 adapter。以后切换自建 iroh-dns-server 时，替换 profile adapter，不改变 EndpointId、授权表或 zterm wire protocol。

连接已授权设备时，把 EndpointId、缓存 relay routes 和地址查询结果合并为候选。只有完成 EndpointId 认证并成功 zterm handshake 后，才更新本地 `cached_relay_routes`；不把临时 direct IP 持久化。DNS/Pkarr 失败但缓存 home relay 可用时继续；两者都失败时返回可诊断错误。

## 7. 配对、正常认证与撤销

### 7.1 票据

`PairTicketV1` 是版本化 protobuf，文本编码为带固定前缀的 base64url（无 padding）；二维码以后直接编码完全相同的文本：

```text
format_version
host_endpoint_id
host_display_name
home_relay_urls[]
offer_id          # 128-bit random
pair_secret       # 256-bit random
expires_at
```

默认 TTL 10 分钟；daemon 同时最多保留少量 offer。offer 只在内存中保存，daemon 重启即失效。票据不包含任何私钥，仍应按临时高敏凭证对待；拥有者可在过期前抢先配对，因此不使用便于猜测的短数字码。

### 7.2 配对握手

配对握手固定为 SSH-like 单向授权：创建票据的宿主写入对票据接收方的 `device_auth`，接收方只把宿主加入本地 `known_devices`，不写入宿主的反向授权。两台桌面设备互相控制时要反向再完成一次配对；两个方向的 authorization generation 和 revoke 完全独立。授权方向必须在票据提示和 revoke 输出中明确显示，不能仅用模糊的“已配对”掩盖。

1. 控制端从票据得到宿主 EndpointId 与 relay route，通过 `zterm-pair/1` 建立 Iroh connection；Iroh 已验证连接对端确实持有票据中的宿主私钥。
2. 宿主发送随机 challenge 与协商的协议版本。
3. 控制端发送自己的显示名、由 Iroh connection 已认证的 controller EndpointId，以及 `HMAC-SHA256(pair_secret, canonical_transcript)`。transcript 绑定双方 EndpointId、offer_id、challenge、版本和过期时间。
4. 宿主在 PairingManager 锁内把 offer 从 `Ready` 原子转为 `Consuming`，验证 HMAC 与 TTL；并发消费者只能有一个进入。
5. 宿主事务性写入/更新控制端的 `device_auth(status=authorized, generation+1)`，提交成功后从内存删除 offer；该事务绝不要求控制端写入宿主的反向 `device_auth`。
6. 控制端收到成功后在本地写入/更新宿主 `known_devices` 和已验证 relay route；这是可连接地址簿，不是授权宿主反向控制自己。
7. 若成功响应丢失，控制端尝试普通 `zterm/1` 连接即可确认授权并补写本地 known device；不得为了可重试而让同一票据成功授权第二个 EndpointId。

### 7.3 正常连接授权

Iroh 的 remote EndpointId 是设备身份，不再额外发明一套长期公钥。接受 `zterm/1` 后，宿主先查 `device_auth`，只有 `status=authorized` 才完成应用 handshake。握手交换：

- wire major/minor range；
- build/version 与 platform（用于诊断，不用于授权）；
- 支持的 capability 集合；
- connection attempt ID（用于去重）；
- 双方设备显示名。

每条后续 stream 都持有 `AuthLease(endpoint_id, generation, cancellation)`，并在打开时再次验证。未知或未授权 EndpointId 在读取任何业务请求之前被拒绝。

### 7.4 revoke 的竞态边界

每个 EndpointId 有一个 `AuthorizationGate`：业务提交路径持有 read permit，revoke 持有 write permit。

1. revoke 取得 write permit，阻止新的输入/RPC 到达提交点。
2. SQLite FULL transaction 把状态改为 revoked 并递增 generation；提交失败则不关闭连接并向用户报错。
3. 更新内存 generation，触发 cancellation，关闭该 EndpointId 的全部新旧 connection 与 stream，释放所有 controller lease。
4. 释放 write permit。此后旧 generation 的任何请求都失败。

PTY input 的“提交点”是 SessionActor 在持有 auth read permit 时执行实际 PTY write。revoke 提交之前已经完成的 write 无法撤回；数据库提交之后，没有旧 lease 可以再写。revoke 永不调用 session close 或给 PTY 发送信号。

## 8. Wire protocol 与 stream 契约

### 8.1 编码与版本

- source of truth：`proto/zterm/v1/*.proto`；Rust 用 prost，Android/iOS 以后用标准 Kotlin/Swift protobuf generator。
- 每条 QUIC/local IPC stream 是 `varint length + WireFrame` 序列。`WireFrame` 含数值 `kind` 与 opaque protobuf payload；未知可选 kind 可按 frame 长度跳过。
- 单 frame 最大 8 MiB；控制请求另限 1 MiB；字符串、列表、terminal size 和批量项都有字段级上限。超过限制立即以 typed protocol error 关闭该 stream，不关闭同 connection 的其他 stream。
- ALPN 承载不兼容的 major 版本；v1 内只做向后兼容的字段新增。handshake capability 决定可选行为，不能让未来 Agent capability 成为普通终端前置条件。
- 不使用 gRPC，不在一个 QUIC stream 内重新实现所有 session 的 multiplexing，也不依赖 Rust enum/内存布局。

### 8.2 对象与标识

```text
DeviceId       = Iroh EndpointId
ConnectionId   = 一次 transport attempt；可重建
SessionId      = daemon 生命周期内稳定的随机 128-bit ID
AttachmentId   = 一次 client view ↔ session 订阅（local 或 remote）
AuthGeneration = 某 EndpointId 的单调授权代数
Revision       = 某 session 的单调 VT 输出版本
```

每个 attachment 另带不可伪造的入口来源：`REMOTE_ENDPOINT(endpoint_id, auth_generation)` 或 `LOCAL_SAME_UID(own_endpoint_id, local_view_id)`。两者共享 lease 和终端状态机；后者由 local socket peer UID 建立信任，不查询 `device_auth`。同一设备的多个本地 CLI/GUI view 仍用不同 AttachmentId，不能因共享 own EndpointId 绕过单 controller 规则。

session name 在单个 daemon 内唯一，`main` 是保留的默认名称；wire 操作最终使用 SessionId，名称只做选择和展示，避免 rename 改变身份。

### 8.3 Stream 类型

每条 stream 的首 frame 是 `StreamOpen {kind, request_id, ...}`：

`request_id` 是 128-bit client-generated operation ID。只读 RPC 可以只用于追踪；create、rename、close、takeover 等状态变更由本地 broker 在 transport 重试时复用同一 ID，宿主在每个已授权 EndpointId 下维护有界结果窗口。窗口内重试必须返回原结果；超出声明的重试期后，服务端必须返回 `operation_outcome_unknown` 而不把旧 ID 当作新操作执行。具体使用 client epoch + monotonic sequence 或等价有界水位方案，由 M2 的状态机测试固定。daemon 重启会结束全部 session，因此 session-operation 去重状态无需跨 daemon 持久化。

1. **Handshake stream**：每个 connection 必须先完成一次，其他 stream 在此前被拒绝。
2. **Control RPC bidi stream**：一个请求/响应，包含 session list、create、rename、close、takeover 和 remote status。独立 stream 避免长输出队头阻塞。本机 session control 与授权设备管理走 user-only IPC；本机 session adapter 和远端 stream adapter 调用同一个内部 `SessionService`，不为 self target 建 QUIC stream。
3. **Future Device event stream**：留给 Android/GUI 的可选 `DEVICE_EVENTS` capability，以后可承载 session create/end、controller change、output revision 和 path 状态；不含 terminal 内容和 Agent 解析结果。第一阶段不实现这条长生命周期 stream，CLI 使用显式 RPC 和当前 attachment 状态。
4. **Terminal attachment bidi stream**：client→server input/resize/detach，server→client snapshot/delta/status/end reason。
5. **Future History page bidi stream**：留给 Android/GUI 阶段的可选 `HISTORY_PAGING` capability；第一阶段不实现独立分页 RPC，attachment snapshot 只携带配置上限内的近期历史。

首版 connection 级并发 stream、RPC、attachment 和排队字节数均有上限。QUIC stream 隔离了可靠有序传输，但仍共享连接拥塞控制，因此高输出 session 的验收必须包含另一个交互 session 的延迟。

### 8.4 主要协议消息

```text
AttachRequest {
  session_id
  attachment_id
  requested_mode = CONTROLLER
  takeover = false
  viewport { rows, cols, pixels }
  known_revision?              # 仅优化；daemon 仍可发 full snapshot
}

Attached {
  session metadata
  granted_mode
  snapshot { revision, ansi_state, bounded_recent_history }
}

TerminalDelta {
  from_revision
  to_revision
  ansi_diff
}

ClientTerminalMessage = SnapshotApplied(revision) | Input(bytes) | Resize(size) | Detach
ServerTerminalMessage = Snapshot | Delta | SyncRequired | LeaseLost | SessionEnded
```

客户端原子应用 snapshot 后发送 `SnapshotApplied(revision)`；服务端在确认该 attachment 的同步水位前不接受其 input/resize。同步期间 CLI 必须继续读取并丢弃普通 stdin（仍允许本地 detach 前缀），不能把内核输入缓冲留到同步后补发；App 同样不发送或排队 input、paste 或 mouse。客户端只在本地状态等于 `from_revision` 时应用 delta；不匹配就请求新 snapshot。

## 9. Connection broker 与重连

### 9.1 一对设备一条主 connection

daemon 的 `ConnectionRegistry` 以 remote EndpointId 为 key，使用 single-flight dial：并发本地请求共享同一个 dialing future，成功后共享一个 connection actor。每个 remote device 最多一个 designated primary connection；短暂重复的 inbound/outbound connection 用双方可见的 initiator EndpointId + random attempt ID 确定同一排序，loser 以 retryable reason 关闭，其本地 attachment 在 winner 上重开。

一台设备连接多个宿主时每个宿主各有一条 connection。不同控制设备连接同一宿主时各有自己的 connection；不同 EndpointId 之间绝不共享认证 transport。

### 9.2 本地 CLI 如何复用

每个 CLI 通过 local IPC 建立一个 local view，并选择 remote device 或保留 selector `local`：

- remote target：daemon 为 view 维护 `DesiredAttachment`，在相应 remote connection 上打开独立 terminal stream；
- local target：daemon 直接为 view 在自己的 SessionActor 中创建 attachment，snapshot/delta 经 local IPC 返回，不经过 ConnectionRegistry。

两条 adapter 都调用同一 `SessionService` 契约。CLI 退出时只删除对应 attachment；remote view 同时关闭 remote stream，local view 只释放本机 lease。最后一个 remote view 退出后 connection 可按空闲超时关闭，但任何宿主 session 都不受影响。

local IPC 复用同一 protobuf/frame codec，但使用单独 local service message，永远不把长期私钥传给 CLI。peer UID 检查是入口前置条件。

self target 在 daemon 缺失时仍走 `ensure_daemon()`；新 daemon 没有 `main` 时执行与远端相同的原子 create-and-attach。daemon 已经由手机使用时，本机 attach 直接看到现存 registry，不能创建同名替代 session。外网、DNS/Pkarr 或 relay 故障不影响 self target。

### 9.3 断线恢复

- connection actor 观察 Iroh close 与 path events。只要 remote-target local view 仍打开，就按 250 ms 起步、指数退避至 10 s并带 jitter 重连；明确的 revoked/incompatible 错误不无限重试。self-target view 不进入网络重连状态。
- connection 消失时所有 remote attachment stream 结束，DesiredAttachment 进入 `reconnecting`；host session registry完全不接收 close。
- 新 connection handshake 成功后，各 view 独立 reattach并取得 full snapshot。不同 session 失败不影响其他 session。
- direct/relay path 在同一 Iroh connection 内切换时不触发 zterm reattach，只更新诊断事件。
- GUI/Android 后台冷 tab 以后只保留轻量 event subscription；切回前台在现有 connection 上新开 attachment，不重新寻址或打洞。

## 10. Session、PTY 与权威 VT

### 10.1 SessionActor

每个 session 由 daemon registry 创建一个独立 SessionActor：

```text
SessionActor
  id / unique name / timestamps / last size
  PTY master + root child process
  TerminalModel + monotonically increasing revision
  bounded scrollback and resource accounting
  attachments map
  optional ControllerLease
  bounded command/input channel
  cancellation and typed end reason
```

处理 connection 的 task 只能发送 attach/input/resize/close 命令，不能持有 PTY master 或 child handle。connection task drop 的唯一效果是 detach。SessionActor 观察根 child 退出后才自然结束；Codex、tmux 等前台子进程退出并返回仍存活的 Shell 不会触发结束。

### 10.2 Shell 与 cwd

- Unix 通过账户数据库（`getpwuid_r` 等）解析当前有效 UID 的 home 与 login shell，不继承 daemon 的 `$SHELL` 或 cwd。
- 使用该 shell 的 login 模式启动交互式 PTY；具体 argv0/`-l` 适配封装在 `zterm-platform`，并分别测试 bash、zsh、fish。账户配置为不可登录 shell 时不绕过系统选择。
- 默认 cwd 是账户 home；`--cwd` 在 spawn 前 canonicalize并验证目录存在、属于可进入路径。验证或 spawn 失败不会先登记 session。
- 初始 terminal size 使用发起 create/connect 的本地 TTY；没有可用尺寸时是 120×40。无人连接时保留最后有效尺寸，不因 detach 自动 resize。
- 只有在兼容门通过后设置经验证的 `TERM`（首选 `xterm-256color`）；未验证 truecolor 前不设置 `COLORTERM=truecolor`。

### 10.3 PTY 数据流

portable-pty 提供阻塞 reader/writer。每个 PTY reader 只把 bytes 交给本 session 的高优先级 TerminalActor；它不等待任何 network attachment。TerminalActor 顺序解析全部 bytes、生成必要终端查询 reply 写回 PTY、递增 revision，再通过 `watch`/轻量通知唤醒 attachment writer。

reader→TerminalActor channel 有界但不允许丢弃 PTY bytes，因为任意缺口会破坏权威 parser。该链路不包含客户端队列，容量耗尽表示 daemon 自身解析能力不足，需以 metrics/测试暴露；不能通过丢 bytes 假装恢复。客户端侧只观察 revision，可任意合并中间更新。

input 与 resize 由 SessionActor 序列化。input 在持有有效 AuthorizationGate read permit 时写 PTY；resize 只有当前 controller 可提交，并同时更新 PTY 与 TerminalModel。

### 10.4 TerminalModel 与 snapshot/delta

内部接口隔离具体 VT library：

```text
ingest(bytes) -> terminal replies / side events
resize(size)
revision()
full_snapshot(viewport, recent_history_budget)
diff_from(baseline)
resource_usage()
```

首选实现以 vt100 0.16.2 的 screen clone、`state_formatted()` 和 `state_diff()` 为基础，并由 zterm adapter 处理 DA/DSR、title/bell、未支持 query、mode 与安全策略。采用前必须通过技术门；接口允许换成更完整的 emulator而不修改 session、protocol 或客户端。

attach 在 SessionActor 同一个序列化点取得 `(screen clone, revision)` 并注册 attachment，先发送 full ANSI state，再从该 revision 生成增量，保证无窗口缺口。每个 attachment writer只保存自己的上次发送 baseline 和一个“最新 revision”通知；多个 PTY 更新会合并为旧 baseline→当前 screen 的一个 diff，不排队每个 raw chunk。若 diff 超过阈值或 baseline 不可用，发送新的 full snapshot。

第一阶段 snapshot 只包含配置上限内的近期行，不携带整个 scrollback，也不暴露独立 history cursor。后续实现 `HISTORY_PAGING` 时，cursor 必须绑定 history revision/epoch；输出导致 cursor 过期或旧行被淘汰时返回 `history_changed`/`history_gap`，而不是读取不一致内容。

### 10.5 资源默认值

Gate 0 已完成测量并固定以下产品准入值。它们与 `ResourceLimits` 和 terminal-model spec 共用同一 source of truth：

- 每个 daemon 最多 8 个活动 session；达到上限拒绝创建，不回收旧 session。
- terminal size 最大 240 columns × 80 rows；超限 resize 明确拒绝。没有有效初始 viewport 时使用 120×40，detach 后保留最后有效尺寸。
- 每 session 固定保留至多 2,000 行标准 scrollback；全部 live Session 的 summed fixed-cell projection 不超过 128 MiB。创建和 resize 在分配或变更前拒绝超限，不对已存在 Session 做动态历史收缩。
- 256 MiB 是 Foundation 的进程 RSS 测量目标，不冒充可由 projection 精确保证的硬内存限额。
- 每 connection 最多 32 个活动 attachment、32 个并发 control RPC；每 attachment只保留一个待发送 revision，单 frame 8 MiB。
- 输入、名称、cwd、设备列表与结构化日志字段均有独立长度/数量上限。

## 11. Controller lease 与 takeover

1.0 SessionActor 的 attachments map 为未来 fan-out 保留多项，但只授予一个 `ControllerLease`；普通 attach 请求在 lease 已占用时返回 `occupied`（包含安全的设备显示信息），不创建 observer。

本机 `LOCAL_SAME_UID` attachment 不拥有隐式优先级。远端手机仍控制 session 时，本机普通 attach 同样返回 `occupied`；只有显式 `--takeover` 才执行下述原子转移。Android/GUI tab 进入冷后台时关闭完整 terminal attachment并释放 lease，因此用户正常从手机回到电脑时通常不需要 takeover。

显式 takeover 在 SessionActor 内原子执行：

1. 验证新设备仍被授权。
2. 创建新 attachment并完成 snapshot 准备。
3. 把 lease 从旧 AttachmentId 切换到新 AttachmentId。
4. 更新 PTY size为新 controller viewport。
5. 向旧 stream 发送 `LeaseLost(TAKEN_OVER)` 并取消其 input/resize权限，但不关闭 session。

所有 input/resize 消息都携带 AttachmentId，并在实际提交时对照当前 lease；网络中在途的旧 controller 消息在切换后失败。未来 observer 是 attachment mode 的新增值，不改变 session 或 output revision；多写者若以后需要，必须是新的显式协作语义。

## 12. CLI 产品面

第一阶段命令面：

```text
zterm setup
zterm status [--json]
zterm doctor
zterm pair create [--ttl 10m]
zterm pair accept [--stdin] [--name <alias>]
zterm device list|rename|revoke
zterm connect <device|local> [--session main] [--takeover]
zterm session list <device|local>
zterm session new <device|local> <name> [--cwd <host-path>]
zterm session attach|rename|close ...
zterm daemon status|stop|restart
zterm logs
zterm reset --identity
```

`pair accept` 默认从不回显的交互式 TTY prompt 读取完整 ticket；自动化必须显式使用 stdin，不能把 bearer ticket 放进 argv、shell history、日志或错误文本。`local` 是保留 selector，不能被设备 alias 占用，未来 GUI 展示为 “This Device”。`connect` 对 local/remote 都在 `main` 不存在时原子 create-and-attach、存在时 attach；`session new` 成功后直接 attach。setup 完成后的裸 `zterm` 等价于 `zterm connect local --session main`；未 setup 时返回带 `zterm setup` 指引的 typed error，不静默生成身份；`zterm --help` 始终显示帮助。close、revoke、stop/restart、identity reset 等破坏性命令显示精确目标与影响并确认；脚本使用明确 `--yes`/`--force`。

交互 CLI 在开始 attachment 时就通过 `TerminalGuard` 进入 Unix raw mode，以便在同步期及时 drain stdin 并识别本地 detach。snapshot 尚未原子应用并发送 `SnapshotApplied` 时，普通按键只丢弃、不远程转发；同一 client→server stream 上 `SnapshotApplied` 与后续 input 有序，因此不再增加一次 server ACK。进入已同步状态后才传递按键与最新 resize，并监听 SIGWINCH；`TerminalGuard` 在所有正常、错误、panic/signal 退出路径恢复 termios、cursor、mouse 与 bracketed-paste mode。

本地控制前缀是一个很小的 byte state machine：默认 `Ctrl+] .` detach，`Ctrl+] Ctrl+]` 写一个原始 `Ctrl+]`；超时或未识别组合按配置原样发送。配置可换键或禁用，不根据远端程序改变。

断线时 CLI 明确显示 reconnecting，并禁用 input；重连 full snapshot 原子应用后恢复。授权撤销、takeover、session exit、协议不兼容和普通网络断开使用不同结束原因，不统一显示为“连接失败”。

## 13. Relay 部署设计

`deploy/relay` 只封装官方 `iroh-relay` 1.0.3，不实现或fork转发核心。Iroh官方v1.0.3 Release已经提供Linux x86_64/aarch64预编译产物与SHA-256，因此优先下载匹配服务器架构的官方binary并校验，而不是在服务器安装Rust或现场编译：

- Dockerfile下载官方 release binary并校验 SHA-256；scratch runtime无shell，以 UID/GID 65532运行，并提供默认config command。
- 当前只支持同机 OpenResty/Cloudflare 终止TLS的反代模式。唯一Compose project与容器均名为`zterm-relay`，镜像直接使用`ghcr.io/leonfox28/zterm-relay:latest`，只发布宿主回环 `127.0.0.1:38451`，不开放80/443/9090/UDP、不修改防火墙。
- Cloudflare代理必须保留WebSocket Upgrade/Connection/subprotocol；公网验收只做一次完整Iroh认证握手，transport中断时PTY仍由daemon独立存活，但不在每次Relay发布中重复reconnect演练。
- `relay.toml` 明确 `access = "everyone"`、`enable_metrics = false`、`enable_quic_addr_discovery = false`，省略 `limits`、token、名单、外部 auth与TLS。
- Compose只保留literal `:latest` image、只读config bind mount、回环38451、`restart: unless-stopped`和`logging.driver: local`。更新必须由人显式执行`pull`后`up -d`；普通重启不自动拉取。
- 运维验收只检查宿主`/healthz`、公开HTTP路径和一次authenticated Iroh Relay握手。无状态运行异常直接recreate；只有实际确认镜像缺陷时才人工选择上一版本tag，不提供自动回滚或常规演练。
- direct TLS/ACME/QAD模板、metrics、Docker health state、自研health probe和monitor sidecar均不预建；真实消费者或Phase 1网络证据出现后再作为独立需求设计。
- 本地smoke通过后执行第3.2节的人工检查点；真实SSH入口、私钥或token只存在于用户指定的安全位置，不提交到Git。服务器只读preflight与一次部署验收分别留痕。

项目默认 relay 与当前自建文档使用相同的反代配置形状。第一阶段单节点 best-effort；客户端配置接受多个 URL，但不宣称已经实现多地域 SLA。

## 14. 安全与可观察性

### 信任与威胁边界

- **信任**：宿主当前 OS 用户及其本地同 UID 进程、宿主操作系统、用户主动授权的完整设备。已配对设备等价于该用户的远程 Shell 公钥，不防范其读取文件、运行命令或调用本地 zterm CLI。
- **不信任**：公网网络、relay、DNS/Pkarr 服务、未配对 EndpointId、畸形协议帧以及 PTY 中程序产生的任意终端控制序列。
- **保证**：网络、relay 与 DNS/Pkarr 不能伪造已签名设备身份或读取/修改未被检测的 zterm 明文；未知 endpoint 不能访问终端 RPC；终端控制序列不能绕过 TerminalModel 直接驱动控制端。
- **不保证**：可用性、匿名性、流量分析防护、宿主或配对设备失陷后的隔离，以及公开无限制 relay 的成本上限。DNS/relay 可以丢弃、延迟或返回旧的签名路由，因此失败表现必须可诊断，但不能把可用性失败误报为授权失败。

### 安全控制

- 所有 remote operation 以 Iroh-authenticated EndpointId + 本地 authorization generation 校验；session name、ticket route和 relay access都不构成授权。
- frame 与字段长度在分配前检查；malformed protobuf、unknown kind、stream flood、过大 terminal size和过多 session都有明确限制。
- 未认证 connection 与 pairing handshake 具有全局/单 EndpointId 并发上限、首帧 deadline 和总字节上限；超时或超限只回收对应 connection，不占住 daemon 的 session/PTY actor。
- secret key、pair secret、完整 ticket、terminal bytes、input、cwd中可能出现的敏感内容不进入日志。
- local IPC校验目录 ownership、mode和 peer UID；拒绝 symlink/非本用户 socket目录。
- state/config migration在事务中执行；identity key不随数据库 reset隐式重建。
- snapshot/delta 只由 TerminalModel 的受控状态生成；未处理的原始 OSC/DCS/APC 不透传。OSC 52 clipboard、文件链接、图片协议等高风险增强功能第一阶段不启用；未知序列不能导致 daemon panic或控制端任意本地操作。

### 诊断与 metrics

- `zterm status --json`：daemon/build/protocol版本、EndpointId、home relay、发布/查询状态、每个 remote connection path、session资源和授权设备摘要。
- `zterm doctor`：目录权限、single-instance、受管理 binary/daemon build 与 protocol 版本、release/install metadata、login shell、PTY、DNS/Pkarr、relay reachability、public-relay禁用、logind session清理风险。
- 本地 metrics：PTY bytes、VT parse latency、snapshot/diff大小、resync次数、queue saturation、connection/path/reconnect、session/attachment数；无内容标签。
- relay 当前没有metrics消费者，因此服务端显式关闭metrics。上游 1.0.3 日志在部分连接事件中会记录缩短后的 EndpointId，并可能记录 remote IP；生产使用Docker `local`日志轮转，不声称 relay 完全看不到这些元数据。

## 15. 验证策略与技术门

### Gate 0：依赖与 VT 可行性（必须先过）

只有第零阶段开发环境、上游relay公网部署与一次health/authenticated-handshake验收完成后，才在搭建完整产品前做有限时 vertical spike；产物仍按正式 crate边界和测试保留：

1. Iroh 1.0.3 `Minimal + Custom relay + n0 DNS/Pkarr` 两 endpoint连接，确认没有 public relay候选并能读取 path events。
2. portable-pty启动账户 login shell，验证 reader、input、resize、root child exit及断开 reader不会给 child发送 HUP。
3. TerminalModel候选解析固定 ANSI corpus，验证 full snapshot→delta等价性、主/备用 screen、Unicode宽度、颜色、cursor、bracketed paste和连续 resize。
4. 真实黑盒运行 tmux与固定版本 Herdr direct attach，验证常见 DA/DSR查询和无人 client时持续排空。
5. 以 16 session、10k scrollback 与 256 MiB 候选压力矩阵完成内存/CPU量化；报告据此固定产品准入为 8 session、2,000 scrollback、240x80、128 MiB projection，并保留 256 MiB RSS 测量目标。

Gate 0通过条件不是“能显示 hello world”，而是候选 VT 达到 PRD 的通用恢复与黑盒兼容基线。vt100不通过时先在 `TerminalModel` 后评估 avt或更完整实现，再继续协议/session功能；这一替换不需要用户重新决定产品范围。

### 自动化层次

- **unit**：frame limits、version/capability、ticket/HMAC/TTL/重放、auth generation、controller takeover、control prefix、path/profile解析、snapshot revision交接。
- **property/fuzz**：protobuf frame decoder、ticket decoder、terminal input prefix、任意 ANSI bytes不 panic；fuzz corpus不得记录真实终端数据。
- **session integration**：真实 PTY持续任务、CLI drop、reattach、root shell exit、invalid cwd、idle不清理、慢 attachment和资源上限。
- **multi-process integration**：两个 daemon + 多个 CLI，证明同 remote只有一条 Iroh connection、多 session独立 stream、revoke竞态与版本不匹配。
- **network lab**：Linux network namespace/容器构造双 NAT；先用当前 `RelayConfig::new(url, None)`、QAD关闭的真实部署记录path events、打洞时延与direct成功率，允许direct时观察direct，阻断后观察自建relay，停止relay后PTY继续。测试配置断言public Iroh relay map为空。只有这些结果显示QAD缺失是实际瓶颈时，才提出另行批准的QAD-only对照实验与服务设计。
- **platform matrix**：macOS arm64/x64、主流 glibc Linux x64/arm64 compile/test；在真实受支持的 macOS/Linux 做 direct-installer clean-account、手动 update/rollback、SSH detach、tmux/Herdr 验收，并对 Alpine/NixOS 做 installer unsupported-platform 负向验收。
- **relay smoke**：干净Linux VM + 公网域名启动唯一反代Compose，检查外部TLS、真实Iroh WebSocket握手、宿主回环38451、Docker `local`日志、Everyone/no-limits，并断言没有9090/metrics、UDP/QAD或防火墙变更；通过即停止，不做回滚/restart/reconnect演练。
- **security acceptance**：tcpdump/relay side观察不到明文、未授权 EndpointId被拒绝、文件/socket mode、过大 frame/stream flood隔离、revoke持久先行。

真实 Codex或OpenCode只作为普通全屏 TUI做最终人工 smoke，不对其输出写断言；自动化以自有 deterministic fixture和 tmux/Herdr黑盒为主。

## 16. 后续阶段兼容形状

- **Android**：复用 protobuf与 Iroh身份语义；App进程持有 connection pool。前台 attachment、后台 revision-only、回前台 snapshot，不要求 daemon常驻或 session迁移到手机。
- **Windows**：`zterm-platform` 新增 ConPTY、Windows用户目录/Named Pipe、当前用户 ACL 与用户级 detached process；Named Pipe 必须支持与 Unix local socket相同的 self-target attachment。新增 PowerShell installer，并让 updater 用版本目录 + 原子切换 `current` 指针避免覆盖运行中的 exe。manifest、签名与 update确认语义不变；第一阶段不假装 Herdr Unix测试能证明 Windows。
- **桌面 GUI**：通过同一 local IPC与 daemon通信，不直接读取 key或再建 Endpoint；本机 tab使用 self target，远端 tab使用 connection broker，两者都是 DesiredAttachment的 UI，CLI继续存在。
- **iOS**：与 Android相同的 controller-only协议；二维码只是 PairTicketV1呈现方式。
- **多端观察**：attachments map开放多个 subscriber，新增 OBSERVER mode；输出 revision和 snapshot格式不变，只有 ControllerLease可 input/resize。
- **Agent 2.0**：在 session旁路新增协商后的结构化 event stream和平台通知消费；原始 terminal attachment始终可单独工作。不在 v1创建空插件注册表或解析 terminal文本。

## 17. 迁移、升级与回滚

这是新仓库，无旧 zterm数据迁移。仍从第一版建立以下边界：

- SQLite schema带版本；迁移在单事务中进行，破坏性迁移前复制数据库备份。旧 daemon不读取已升级的不兼容 schema，而是明确拒绝。
- wire v1的兼容字段只新增；不兼容变更使用新 ALPN major并在至少一个手动升级窗口内给出明确版本错误。
- `zterm update` 由用户触发，先验证 artifact再确认 session终止；安装激活失败自动恢复旧二进制。identity与auth表不随二进制回滚改变；若新 daemon 已执行不可降级 schema migration，回滚步骤恢复升级前数据库备份并明确可能丢失升级后的非终端元数据。
- relay无 zterm业务状态；运行异常直接recreate。只有实际确认新镜像缺陷时，运维人员才手动改用上一版本tag；不建立自动回滚或常规演练，终端/session无需迁移。
- profile切换不重建 EndpointId。配置验证失败时保留上一份已知可用 config，并拒绝启动错误 profile。

## 18. 关键取舍与剩余风险

| 选择 | 获得 | 接受的代价 / 风险 |
| --- | --- | --- |
| 宿主权威 VT | 新客户端、长断线和未来 observer都能恢复当前 screen | terminal emulator兼容性成为第一阶段最高技术风险，必须先过 Gate 0 |
| 单 daemon直接持有 PTY | 进程模型与升级简单 | daemon crash/restart会结束任务，1.0明确接受 |
| 本地命令按需 detached-spawn | 无管理员权限、无额外 supervisor | 宿主重启后首次本地命令前不可达；某些 logind策略会清理进程 |
| Iroh官方 DNS/Pkarr + 自建 relay | 少部署一个后端，路由可更新 | 地址查询元数据暴露给官方服务，受限速与无 SLA影响 |
| 固定官方 `iroh-relay` binary + 最小部署外壳 | 不承担转发协议实现与维护，可直接跟随Iroh安全修复 | 项目仍需负责checksum、镜像、配置、服务器暴露面与一次上线验收 |
| Pkarr不发布 direct IP | 减少公开网络元数据 | relay完全故障时可能无法启动本来可直连的新连接 |
| Everyone且无 limits | 无账号/凭据即可开箱使用 | 非 zterm滥用、容量与带宽成本不可预测 |
| 主机级设备信任 | 与同 OS用户任意 Shell权限一致，模型诚实 | 配对设备可管理全部 session和本地 zterm CLI，不能宣称细粒度隔离 |
| Protobuf +独立 QUIC streams | Android/iOS跨语言且无全局队头阻塞 | 需要严格 schema/生成物同步和资源限制 |
| GitHub 托管的用户级 direct installer + 手动 updater | 原生程序无 Node依赖；latest 默认稳定；显式 tag 可重现稳定/开发版；停 daemon 前完成验证 | 项目承担 bootstrap信任、manifest签名、原子替换、回滚和跨平台 installer安全 |
| 卸载删除 `~/.zterm/` 与设备私钥 | 正常重装成为新 EndpointId，旧配对不可复用，符合用户对“卸载”的直觉 | 无中央控制平面时不能让已复制的旧私钥全局吊销；泄露密钥仍需逐宿主 revoke |
| local IPC self attach 同一 SessionActor | 用户从手机回到电脑后无需 tmux/Herdr即可接续同一 PTY；离线也可用 | local/remote adapter必须共享状态机；本机视图也必须遵守 controller lease，不能产生隐式双写 |

2026-08-21 的安装补充已确定 GitHub Release 托管、latest stable 默认、精确签名 prerelease、`~/.zterm/` 与卸载重建身份；卸载撤销边界已确认跟随 Zedra，本机销毁身份、泄露凭证逐宿主 revoke。新增的桌面 self attach 已确定必须复用同一 SessionActor 与 controller lease，第一阶段覆盖 macOS/Linux、Windows阶段保持同语义；裸 `zterm` 已确定为本机 `main` 快捷入口。随后新增第零阶段：先完成本机开发环境和上游`iroh-relay`部署物，本地验证后等待用户提供服务器连接方式，再部署公网默认relay。剩余技术不确定性包括 VT候选、资源默认值、最低 glibc基线、正式 GitHub仓库 URL和平台实现细节，由后续技术门收敛，不改变既定用户行为。
