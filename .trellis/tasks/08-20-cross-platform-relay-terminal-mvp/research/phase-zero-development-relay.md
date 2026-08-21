# 第零阶段：开发环境与上游 Relay 部署基线

日期：2026-08-21

## 1. 目标与执行边界

第零阶段在任何 zterm 产品功能实现之前完成两件事：

1. 把当前开发机整理成可重复验证的 Rust、Protobuf、Docker 与质量检查环境，并建立空 Rust workspace；
2. 把固定版本的上游 `iroh-relay` 封装成最小 Docker/Compose 部署物，先本地验证，然后在用户明确提供公网服务器连接方式后部署并完成公网 smoke。

公网部署设显式人工检查点：本地镜像、Compose、配置校验和 smoke 都通过后，助手停止并告知用户“已到公网 relay 部署步骤”，列出需要的 SSH 入口、登录用户/认证方式、relay 域名与 DNS 状态。用户提供这些信息前，不连接服务器；凭据、私钥和真实 `.env` 不写入仓库、任务文档或日志。若只读 preflight 发现服务器缺少 Docker、需要修改系统防火墙或存在端口冲突，先报告具体变更，再执行相应系统级操作。

## 2. 当前开发机证据

2026-08-21 在 `/Users/huyuanzhe/projects/zterm` 实测：

- Apple Silicon，macOS 26.6.2；
- Homebrew 6.0.18、完整 Xcode/Apple clang 21、Git 2.50.1 已安装；
- `rustup` 1.29.0 已安装；用户随后重新运行官方rustup installer，将现有stable更新为Rust/Cargo 1.98.0，rustfmt 1.9.0、Clippy 0.1.98和rust-analyzer均已安装；
- 已安装targets为`aarch64-apple-darwin`、`aarch64-apple-ios`和`aarch64-linux-android`；
- Docker CLI/Engine/Compose 未安装或不在 PATH；
- `protoc` 与 `pkg-config` 未安装，CMake 4.4.2 已安装。

因此第零阶段不得重复安装rustup或Rust 1.91。项目按用户决定使用当前最新版，并在`rust-toolchain.toml`精确固定Rust 1.98.0，而不是提交浮动`stable`；未来升级由显式版本变更触发并重跑全部门禁。按实际构建需要补齐`pkg-config`和固定版本的Cargo质量工具。Protobuf生成采用仓库固定/可复现方案，避免把系统`protoc`变成最终用户依赖。当前Mac没有Docker，默认建议用Homebrew管理Docker CLI/Compose与Colima的用户态Linux VM；若用户已经更偏好Docker Desktop，可在实际安装前替换这一实现，不改变仓库或服务器部署契约。

## 3. Relay 是否需要自己实现

不需要、也不应重写 relay 转发协议或服务端核心。zterm 直接使用与客户端 Iroh 版本匹配的官方 `iroh-relay` 1.0.3 release binary，并校验上游 SHA-256；zterm 自己只维护：

- 下载与校验固定上游产物的 Dockerfile/构建脚本；
- `relay.toml`、Docker Compose、环境变量模板与端口/TLS 配置；
- health check、内置私有 metrics、Docker 日志轮转；
- 部署、升级、固定 digest 回滚和故障排查文档/脚本。

第零阶段不写自有 relay 数据平面、不 fork `iroh-relay`、不嵌入业务认证服务，也不复制 Zedra 的自定义 monitor sidecar。使用上游 relay 自带 health/metrics 加服务器/云厂商现有监控即可；仍保持已经确认的 `Everyone`、无 token、无名单、无 zterm 自定义限速策略。

Iroh v1.0.3 的官方 GitHub Release 已提供 Linux x86_64/aarch64 的 `iroh-relay` 预编译产物和 SHA-256，所以 zterm 无需在公网服务器上安装 Rust 或编译 relay。最终镜像必须固定上游版本、校验和与产出 digest。

## 4. Zedra 对照证据

核对本地 Zedra 提交 `a30bc6c69d812afacbe0e1fb6ad4d25665d4030e`：

- `deploy/relay/Dockerfile:5-13` 从 n0-computer/iroh 的固定提交构建 `iroh-relay`，没有 Zedra 自己的转发实现；
- `deploy/relay/Dockerfile:19-27` 只把上游二进制、配置模板和 entrypoint 放入运行镜像；
- `deploy/relay/docker-compose.yml:1-39` 负责端口、证书卷、权限、健康检查与日志轮转；
- `deploy/relay/README.md:84-104` 将架构明确写为 Docker Compose 中的 `zedra-relay` 加自有监控外壳；
- `docs/RELAY.md:17-47` 说明实际数据平面是 `iroh-relay`，只转发 Endpoint 之间的密文并继续尝试 direct path。

所以“Zedra 也是这么做的”准确说法是：它没有重写 relay 核心，但写了自己的镜像、Compose、部署脚本和额外监控。zterm 采用相同的上游复用原则，并在第零阶段进一步减少组件，只保留必要部署外壳。

## 5. 第零阶段完成门

第零阶段只有在以下条件全部满足后才能进入第一阶段 Gate 0：

1. 仓库精确固定Rust/Cargo 1.98.0，格式化、Clippy、测试和依赖检查命令可在本机运行；工具链升级不能由浮动stable静默发生；
2. Docker Engine 与 Compose 可用，固定 `iroh-relay` 镜像能在本机启动、健康退出并通过配置检查；
3. 已到达并执行人工服务器连接检查点，未把任何连接秘密提交到仓库；
4. 所选同机反代部署的公网 DNS/TLS、真实 relay 握手、宿主回环38451/9090、health、私有 metrics、Everyone/no-limits 与日志轮转通过 smoke；UDP/QAD保持关闭且无防火墙变更；
5. 镜像 digest、非秘密配置、验证记录和回滚步骤被记录，旧镜像回滚演练通过；
6. relay 容器和宿主均没有 zterm 终端内容、PairTicket 或设备私钥持久化。

第一阶段的 Iroh/NAT/relay-only Gate 0 测试必须使用这套已部署 relay，而不是临时依赖公共 Iroh relay。

2026-08-21补充边界：QAD是可选的观察地址来源，可能提高部分网络的direct成功率，但不参与relay数据转发，也不是打洞失败后密文回退的前提。第一阶段先用`RelayConfig::new(url, None)`在真实NAT组合中记录direct/relay path events和直连成功率；只有实测证据支持时，才另行评审QAD-only服务、证书、UDP和防火墙暴露面。

## 6. 证据入口

- Iroh v1.0.3官方Release与预编译relay artifacts：`https://github.com/n0-computer/iroh/releases/tag/v1.0.3`
- Iroh官方relay服务端源码：`https://github.com/n0-computer/iroh/tree/v1.0.3/iroh-relay`
- Zedra固定上游relay镜像：`/Users/huyuanzhe/projects/zedra/deploy/relay/Dockerfile`
- Zedra Compose与监控外壳：`/Users/huyuanzhe/projects/zedra/deploy/relay/docker-compose.yml`
- Zedra relay架构说明：`/Users/huyuanzhe/projects/zedra/docs/RELAY.md`
