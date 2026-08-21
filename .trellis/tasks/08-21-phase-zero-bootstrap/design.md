# Phase Zero Technical Design

## Change Boundary

当前仓库只有规划与 Trellis 元数据，没有 Rust workspace、容器开发环境或 Relay 部署物。本任务只建立开发/部署基础设施，预期新增：

- 根级 Cargo/toolchain/lint/deny 配置、README 与 CI；
- `crates/{core,proto,platform,daemon,cli}` 和 `proto/`；
- `deploy/relay/`、`tests/relay/` 与相关文档；
- 本机工具版本与可重复启动说明。

明确不实现终端流、PTY、daemon 生命周期、设备身份、配对、网络协议或会话状态。

## Environment Model

- `rust-toolchain.toml` 精确固定 `1.98.0` 并声明 rustfmt/Clippy/rust-src；本机已有其他 toolchain 保持不变。
- Docker 客户端与 Compose 来自 Homebrew，Linux VM 使用 Colima；不依赖 Docker Desktop。
- protobuf 生成由 Rust 构建依赖下载固定的 vendored `protoc`，从而避免系统包差异。
- cargo-deny 版本写入环境文档/CI，升级必须显式验证。

## Workspace Model

依赖保持单向：

```text
zterm-core
   ^       ^
   |       |
proto   platform
   ^       ^
    \     /
     daemon
        ^
        |
       cli
```

第零阶段中的各 crate 只包含能证明边界、编译链和 protobuf 生成可用的最小代码。CLI 占位命令只输出版本/阶段信息，不读写用户配置或启动后台进程。

根 `[workspace.package].version` 是所有 zterm 产品组件的 lockstep 版本源，
首个版本为 `0.1.0`，五个产品 crate 均通过 `version.workspace = true` 继承。
以后 CLI、daemon、协议、平台库、App 和 Relay wrapper 一起升级，不为 monorepo
组件维护独立版本。隔离在 workspace 外的 handshake probe 只是验收工具，不是
产品发布物，因此保留自己的非产品版本。

## Relay Image Model

- 将 `IROH_VERSION=1.0.3`、官方 release URL、架构映射和 SHA-256 清单集中到构建上下文。
- BuildKit 根据 `TARGETARCH` 选择官方 linux x86_64/aarch64 产物；未知架构和 checksum 不匹配都立即失败。
- 最终镜像只保留 relay 可执行文件、一次性静态 HTTP 健康探针、CA 证书和非 root 运行账户，不包含 shell、编译器或下载工具；健康探针不是常驻 monitor。
- Compose 通过运行时环境注入 hostname/ACME 等部署值；示例文件不含真实 secret。
- 配置必须由 Iroh 1.0.3 的官方源码或 CLI/schema 验证。metrics 仅绑定宿主回环或内部网络，公网不暴露。
- Docker 原生日志轮转替代自定义 monitor sidecar；Relay 采用上游 Everyone access policy。
- `.github/workflows/relay-image.yml` 是 Relay registry 镜像的唯一构建入口；
  单次运行构建一个 linux/amd64 + linux/arm64 manifest，所有 Action 固定完整
  commit SHA，工作流只授予 contents read/packages write，并生成 provenance
  与 SBOM。稳定 GitHub Release tag 必须是 canonical
  `vMAJOR.MINOR.PATCH`，去 `v` 后与 workspace SemVer 完全一致；workflow 检出
  原始 Git tag，但以去 `v` 的标签发布到 `ghcr.io/leonfox28/zterm-relay` 并
  更新 `latest`。prerelease 必须是 canonical
  `vMAJOR.MINOR.PATCH-PRERELEASE`，完整去 `v` SemVer 同样与 workspace 精确
  一致，只发布到 `ghcr.io/leonfox28/zterm-relay-dev` 且不更新 `latest`。
  build metadata 因无法无歧义映射到 OCI tag 而拒绝。manual dispatch 也只进开发 package，
  用户输入的非空合法 OCI tag 原样使用（例如 `phase-zero`），只禁用保留值
  `latest`。manual tag 是可重复发布的开发 alias，调用方应避免复用 prerelease
  tag，工作流输出的 image+manifest digest 才是不可变交付标识。生产 package
  隔离保证 manual alias 不会覆盖稳定 release；部署不使用任何 mutable tag。
- 本地 Buildx/Compose 仍用于 checksum、架构和运行时验证，但其 image ID
  只是开发证据。两份生产 Compose 移除 `build` 并要求
  `RELAY_IMAGE=ghcr.io/leonfox28/zterm-relay@sha256:<digest>`；开发 package
  即使以 digest 引用也必须被生产 preflight 拒绝。

## Production Deployment Modes

- 保留 `compose.production.yaml` 作为容器直接持有公网 80/443/7842、由
  `iroh-relay` 自行完成 TLS/ACME 与 QAD 的通用自建模式。
- 当前项目默认服务器采用同机 OpenResty 终止 TLS：
  `compose.reverse-proxy.yaml` 只把纯 HTTP Relay 发布到宿主
  `127.0.0.1:38451`，metrics 只发布到宿主回环，不占用 80/443，也不运行
  ACME。该模式运行普通 1.0.3 配置且不使用 `--dev`。
- 默认服务端 Compose 根目录遵循现有 1Panel 自建编排布局：
  `/opt/1panel/docker/compose/zterm-relay`。9090 是 relay 内置的 Prometheus
  运维指标端口，不承载客户端 relay 数据且不得通过反代公开。
- OpenResty 必须透明保留 `/relay` 的 HTTP/1.1 WebSocket upgrade、子协议和
  Iroh 认证 header；公网完成门使用真实 Iroh 客户端握手验证，不能只验证
  `/healthz`。若域名启用Cloudflare代理，还要确认WebSockets开关开启、Argo
  未用于该流量、WAF/限速不阻断初始101，并验证边缘断开后的Iroh重连；
  Cloudflare或OpenResty连接重建不得影响宿主PTY生命周期。

### QAD / NAT Boundary

Iroh 1.0.3 明确允许无 `[tls]` 的纯 HTTP Relay，并把 QAD 默认为关闭；因此
反代模式是合法的 relay-only 部署。但 HTTP 反代无法转发 UDP QAD，当前
38451-only 模式设置 `enable_quic_addr_discovery = false`。这不削弱 relay
转发：direct/NAT 打洞成功时数据路径直连，失败或失效时仍可完整回退到该
Relay；QAD 只负责报告观察到的公网地址，可能提高部分网络的 direct 成功率，
不参与也不是密文 relay 回退的前提。第一阶段客户端必须用
`RelayConfig::new(url, None)` 明确匹配该部署；从裸 `RelayUrl` 构造会默认
尝试 UDP 7842。

第一阶段先在 QAD 关闭的真实部署上对代表性 NAT 组合记录 direct/relay
path events 和直连成功率，再依据证据决定是否需要 QAD-only 服务；第零阶段
不预设这个结论。若实测后要求该服务器提供 QAD，需要独立批准公网 UDP、
证书和防火墙暴露面。上游
1.0.3 支持以 `enable_relay = false`、启用 QAD、`cert_mode = "Manual"` 运行
单独的 QAD-only 进程；该进程与证书只读挂载不属于当前部署范围。不得用
上游标记为开发用途的 `--dev` TLS bypass 冒充生产组合模式。

## Verification Model

1. 静态检查固定版本、checksums、非 root、架构映射和 Compose 暴露面。
2. 构建两种目标架构，并验证错误 checksum/未知架构会失败。
3. 在 Colima 中启动 Compose，通过真实 HTTP 端点验证健康状态、relay 端点、私有 metrics 和日志配置；生产 schema 探针必须禁用容器外部网络。
4. 单独启动同机反代生产模式，确认无 `--dev`、只存在宿主回环
   38451/9090 映射、没有 80/443/UDP 映射，并验证 `/relay` 返回合法
   WebSocket `101` upgrade。
5. 静态验证发布工作流权限、Action SHA、GHCR 动态 owner、双平台 manifest、
   生产/开发 package 隔离、stable/prerelease/manual 标签矩阵、Git tag 去
   `v` 映射、workspace 版本相等门禁、build metadata 拒绝、provenance/SBOM
   和完整 image+digest 输出；记录本地 image ID、开发 digest 与生产
   GHCR digest 的不同用途，再执行 secret scan。
6. 本地检查全部通过后硬停止；不得自行读取 SSH 配置并尝试服务器连接。

## Public Deployment Gate

用户已选择现有 OpenResty 反代 `http://127.0.0.1:38451` 的部署约束。远程
执行仍由主会话负责：先做只读检查，再单独确认可能影响系统或网络的变更。
该模式验证 DNS/TLS、外部 WebSocket Relay 握手、宿主回环端口、metrics
隔离和回滚，并明确记录 QAD/UDP 未部署；不得开放 UDP 7842 或修改防火墙，
也不能把 UDP 项伪报为已通过。

目标公开 GitHub 仓库已创建，`main` 已推送。首次 manual 开发工作流已从
`43b06ff` 构建并公开发布 `zterm-relay-dev:phase-zero`，其 multi-platform
digest、匿名拉取和 amd64/arm64 运行均已验证。Compose 项目已迁移到 1Panel
根路径并通过公网握手；已部署的本地校验 arm64 image ID 仍是显式临时例外，
只有生产 `zterm-relay` package 经稳定 release 产生真实 digest 后，才能完成
生产 provenance 切换。`zterm-relay-dev` digest 不能替代该门禁。
