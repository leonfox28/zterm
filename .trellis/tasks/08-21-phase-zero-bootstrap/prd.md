# zterm 第零阶段：开发环境与上游 Relay

## Goal

在任何终端产品能力实现之前，建立可重复的本机 Rust/Docker 开发环境与最小 Rust workspace，并将固定版本的官方 `iroh-relay` 封装为可审阅、可验证、可回滚的 Docker/Compose 部署物。

本地验证全部通过后，必须在第一次连接公网服务器之前暂停并通知用户；只有收到用户提供的安全连接方式、域名/DNS 信息和服务器 Docker 状态后，才进入公网部署。本子任务实现父任务 PRD 的 R0 与验收项 Z，不改变第一阶段产品行为。

## Current Evidence

- 开发机为 Apple Silicon macOS。
- 已有 Homebrew、Xcode、Git、CMake、rustup 1.29、Rust 1.98、rustfmt、Clippy、rust-analyzer，以及 macOS/iOS/Android Rust targets。
- 现有 Rust 1.95 工具链属于用户资产，必须保留。
- 开始时缺少 Docker CLI/Compose、Colima、pkg-config 与 cargo-deny；项目不依赖系统级 `protoc`。
- Iroh v1.0.3 官方 release 提供 Linux x86_64/aarch64 relay 构建产物及校验和。
- GitHub 目标仓库已确定为 `leonfox28/zterm`；在仓库实际推送且对应 GHCR
  channel 首次成功前，可以完成发布工作流与部署契约，但不能虚构尚未发布的
  package 或 registry digest。

## Requirements

### R0-A：本机开发环境

- 所有工具先检测再安装；不得重装 rustup、删除现有工具链、覆盖用户 shell 配置或要求 Docker Desktop/管理员授权。
- 项目通过 `rust-toolchain.toml` 精确固定 Rust 1.98.0；后续升级必须显式触发并通过质量门禁。
- 使用 Homebrew 安装 Docker CLI/Compose、Colima 与 pkg-config，并让本地 Docker/Compose 可重复启动。
- cargo-deny 固定到与 Rust 1.98.0 兼容的版本；protobuf 构建使用 vendored `protoc`，不要求机器全局安装。

### R0-B：最小 Rust workspace

- 创建 `zterm-core`、`zterm-proto`、`zterm-platform`、`zterm-daemon`、`zterm-cli` 五个最小 crate，并明确依赖方向。
- 创建 `proto/`、`install/`、`deploy/relay/`、`tests/e2e/`、`tests/relay/`、`docs/` 与 CI 目录。
- protobuf 生成可复现；CLI 只提供无副作用的占位入口，不提前实现 M1+ 的终端、会话、连接或 daemon 行为。
- 建立 fmt、Clippy、单元测试、文档、依赖审计和 CI 基线。

### R0-C：官方 Relay 部署物

- 固定并校验官方 `iroh-relay` 1.0.3 二进制；不 fork、不重写 relay 数据面。
- 构建可复现的 linux/amd64 与 linux/arm64 镜像；架构映射、下载 URL、版本和校验和集中管理，未知架构必须失败。
- 运行镜像保持最小化、使用非 root 用户，并产出镜像 digest。
- 镜像只由 GitHub Actions 构建：稳定非 prerelease 发布到生产 package
  `ghcr.io/leonfox28/zterm-relay`，prerelease 和手动构建发布到独立开发
  package `ghcr.io/leonfox28/zterm-relay-dev`；每次只生成一个覆盖
  linux/amd64 + linux/arm64 的 manifest。Action 固定完整 commit SHA，权限
  限于 contents read/packages write。
- 稳定 release 保留 release tag 并更新生产 package 的 `latest`；prerelease
  只在开发 package 保留 release tag，不得接触生产 package 或 `latest`；手动
  发布将用户给出的非空合法 OCI tag 原样写入开发 package，只禁止保留值
  `latest`。开发 tag 可以作为 mutable alias 重复发布，真实交付以工作流输出
  digest 为准；package 隔离保证它不能覆盖稳定生产 release。
- `latest` 是工作流管理的稳定 alias，不允许作为 GitHub release tag 或手动
  输入。
- 两份生产 Compose 不提供 `build` 回退，必须消费工作流实际产出的 GHCR
  immutable digest；本地构建只保留作开发与 CI 验证。
- 提供与 Iroh 1.0.3 实际配置 schema 一致的 `relay.toml`、Compose、环境变量示例、健康检查、仅私网可见的 metrics、日志轮转、部署与回滚文档。
- Relay 采用 Everyone 策略：不增加 token、白名单、zterm 专属限速或自定义 monitor sidecar；不记录终端载荷，不引入业务数据库。
- 不照搬 Zedra 针对旧版 Iroh 的配置假设，必须以 1.0.3 官方行为和源码/文档为准。

### R0-D：公网部署检查点

- 先完成全部本地构建、测试与 Compose smoke test，随后强制暂停。
- 暂停前不得尝试连接公网服务器，也不得请求用户把私钥粘贴到聊天或仓库。
- 用户提供连接入口后，先做只读 preflight；任何 Docker、端口、防火墙或系统级变更都要先报告影响。
- 仓库、构建日志和部署文档不得包含服务器凭据、私钥或真实 secrets。
- 默认服务器 Compose 根目录为
  `/opt/1panel/docker/compose/zterm-relay`；9090 只承载 loopback Prometheus
  指标，不是 relay 流量，也不得交给 OpenResty/Cloudflare。

## Acceptance Criteria

- [x] 项目精确使用 Rust 1.98.0，且用户已有 Rust 1.95.0 工具链未被删除或修改。
- [x] Docker CLI、Compose 与 Colima 可工作并可按文档重复启动；所有本机质量门禁通过。
- [x] 五个最小 crate 能构建与测试，且没有提前实现 M1+ 产品功能。
- [x] 官方 1.0.3 relay 二进制通过 checksum；多架构与篡改失败路径有自动化验证。
- [x] Compose、健康检查、私有 metrics、Everyone 策略、日志轮转和回滚流程通过本地 smoke test；不存在自定义 relay 或 monitor 服务。
- [x] 第一次公网服务器连接前已暂停通知用户，并完成仓库 secret scan。
- [x] 获得用户授权后，所选同机反代模式的公网 DNS/TLS、真实 relay 握手、宿主回环 TCP 38451/9090、私有 metrics 与回滚能力均验证通过；确认未发布 UDP/QAD 端口且未修改防火墙，子任务才可完成。
- [x] 文档和第一阶段计划明确区分 direct/NAT 打洞与 relay 回退：QAD 只是可选地址发现辅助，不参与 relay 转发；是否增加 QAD-only 服务必须等第一阶段真实 NAT 路径测试后再决定。
- [x] GHCR 发布工作流、生产/开发 package 隔离、双架构 manifest、稳定/
  prerelease/手动标签规则、完整 SHA 固定、最小权限、provenance/SBOM、完整
  image+digest 输出、digest-only Compose 与 1Panel 路径已有静态门禁。
- [x] 公网服务器运行时 Compose 已迁移到
  `/opt/1panel/docker/compose/zterm-relay` 并重新通过 authenticated relay
  handshake；旧目录可恢复地改名保留，而非直接删除。
- [ ] GitHub remote 建立并推送后，分别按需实际运行开发或稳定发布工作流并
  记录真实 GHCR multi-platform digest；生产部署必须等稳定 release 产生
  `ghcr.io/leonfox28/zterm-relay@sha256:<digest>`，将该 package 设为 public
  （或配置只读拉取认证），再替换公网服务器的临时本地镜像 ID。开发 package
  的 digest 不能用于生产替换。

## Out of Scope

- M1+ 的终端、PTY、daemon、配对、E2EE、会话和客户端功能。
- 自建 DNS discovery、控制平面、relay 数据面或监控守护程序。
- 本地检查点前的远程服务器连接。
- 删除已有工具链、安装 Docker Desktop或全局改写 shell 配置。

## Dependency

本任务没有前置实现依赖；父任务中的后续 foundation gate 必须等待它完成。
