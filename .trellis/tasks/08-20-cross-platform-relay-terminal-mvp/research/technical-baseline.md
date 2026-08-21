# 第一阶段技术基线（首次调查 2026-08-20，复核 2026-08-21）

## 2026-08-21 复核结论

- 截至 2026-08-21，官方 Releases 仍将 `v1.0.3`（2026-07-20）标记为 Latest，本阶段暂无需改变版本基线。发布页同时提供 Linux `iroh-relay` 二进制与 SHA-256，因此 zterm 可以校验上游 artifact 后构建并固定自己的容器 digest，不假设上游已提供所需的 Docker image。来源：[Iroh 官方 Releases](https://github.com/n0-computer/iroh/releases)。
- 官方公共 DNS/Pkarr 服务仍是免费、有限速且不保证 uptime；默认记录可发布 home relay，默认 Pkarr publisher 不发布 direct IP。这支持当前“官方 DNS/Pkarr + 自己的 relay + relay-only 公开寻址”方案，但发布文档必须披露限速和无 SLA。来源：[Iroh DNS address lookup 文档](https://docs.iroh.computer/connecting/dns-address-lookup)与[Iroh hosting 说明](https://www.iroh.computer/services/hosting)。
- 上游 relay 日志不能被描述为“只有 zterm 自定义短 hash”。`iroh-relay` 在部分 tracing event 中使用缩短 EndpointId，QUIC address-discovery span 可包含 remote socket address；运维边界应是受控日志级别、轮转和保留期，而不是承诺 relay 无法观察连接元数据。来源：[`clients.rs`](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/server/clients.rs#L77-L113) 与 [`quic.rs`](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/quic.rs#L145-L147)。

## Iroh 版本与 API

- Iroh 官方最新稳定版本是 `1.0.3`，发布于 2026-07-20；`iroh`、`iroh-base` 与 `iroh-relay` 1.0.3 的 MSRV 都是 Rust 1.91。来源：[官方 release](https://github.com/n0-computer/iroh/releases/tag/v1.0.3) 与 [crates.io `iroh` 1.0.3](https://crates.io/crates/iroh/1.0.3)。
- Iroh 1.0 的 Endpoint 以长期 `SecretKey`/`EndpointId` 建立相互认证的 QUIC connection，并原生提供多个双向/单向 stream。`Connection::path_events()` 可观测 relay/direct path 的建立、选择和关闭，适合实现诊断与验收，而不需要修改终端协议。来源：[Iroh `Connection` API](https://docs.rs/iroh/1.0.3/iroh/endpoint/struct.Connection.html)。
- zterm 不能直接使用 `presets::N0`，因为该 preset 同时加入 n0 公共 relay。第一阶段应从 `presets::Minimal` 构建 Endpoint，显式配置 `RelayMode::Custom` 指向项目或用户 relay，再加入 `PkarrPublisher::n0_dns()`、`PkarrResolver::n0_dns()` 和 `DnsAddressLookup::n0_dns()`。这样沿用 `dns.iroh.link`，但业务流量没有公共 relay 回退。来源：[Iroh 1.0.3 preset 源码](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/endpoint/presets.rs) 与 [address lookup 模块](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/address_lookup.rs)。
- 官方 DNS/Pkarr 默认只发布 home relay URL，不发布 direct IP；只有显式使用 unfiltered address filter 才会发布 IP。来源：[Iroh DNS 文档](https://docs.iroh.computer/connecting/dns-address-lookup)与 [Pkarr publisher 源码](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh/src/address_lookup/pkarr.rs)。
- `EndpointAddr` 可以同时携带 EndpointId 与一个或多个 `TransportAddr::Relay`，因此配对票据和本地缓存可以提供 DNS/Pkarr 故障时的 relay 路由提示。当前 Iroh 实际只选择一个 home relay，但 Endpoint 的自有 relay map 可以配置多个候选。来源：[Iroh base `EndpointAddr`](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-base/src/endpoint_addr.rs)。

## Relay 服务

- `iroh-relay` 1.0.3 自带 TLS/ACME、QUIC 地址发现、健康/metrics、Everyone/allowlist/denylist/shared-token/HTTP access，以及可选连接与流量限制；无需 zterm 重写 relay 协议。来源：[官方 relay server 配置](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/main.rs)。
- 配置默认 `access = Everyone`，`limits` 缺省即不启用限速。这与用户确认的第一阶段策略和 Zedra 当前部署一致。
- relay 是无终端业务状态的密文转发组件。需要持久化的部署数据只包括 TLS/ACME 运维材料与日志策略，不包括 zterm 身份、授权、session 或终端内容。
- Relay密文转发与QUIC address discovery (QAD) 是独立开关。当前同机反代部署关闭QAD/UDP仍可完整承担relay fallback；QAD只可能改善direct候选地址发现。第一阶段先以`RelayConfig::new(url, None)`做真实NAT/path-events基线，是否增加QAD-only服务由实测决定。
- 官方文档支持在公网 IP/域名上运行 `iroh-relay`，并由客户端用自定义 relay map 连接；zterm 的 Docker Compose 只是对这个上游 server 的可复现封装。来源：[Iroh 自建 relay 文档](https://docs.iroh.computer/add-a-relay)。

## PTY 与权威 VT 候选

- `portable-pty` 0.9.0 提供 Unix PTY 与 Windows ConPTY 的统一接口，Zedra 已在 macOS/Linux/Windows host 路径使用其 0.8 版本。第一阶段可以用 0.9.x 隔离 PTY 创建、resize、读写与子进程等待，为第三阶段 Windows 保留实现路径。来源：[WezTerm `portable-pty`](https://github.com/wezterm/wezterm/tree/main/pty)与 [crates.io](https://crates.io/crates/portable-pty)。
- `vt100` 0.16.2 提供宿主内存 screen、主/备用 grid、有界 scrollback、mouse/bracketed-paste 等输入 mode，并能生成重建完整状态的 `state_formatted()` 和相对旧状态的 `state_diff()`；这直接匹配 snapshot + revision/delta 模型。来源：固定调查提交 [`fc26fd9`](https://github.com/doy/vt100-rust/tree/fc26fd9c3d72f9af8a214741c100920734500de7)。
- `vt100` 不是完整终端产品。它把未处理的 CSI/OSC 等暴露给 callbacks，但必要的设备状态查询响应、声明的 `TERM` 能力、Unicode 宽度和代表性 TUI 兼容性仍需由 zterm 验证。采用它之前必须通过一个有限时技术门：tmux、Herdr、alternate screen、bracketed paste、resize、DA/DSR 等查询；失败则在内部 `TerminalModel` 接口后替换实现，不能降低 PRD 的恢复保证。
- `avt` 0.18.0 也提供现代 ANSI parser、主/备用 buffer 与 scrollback，但官方明确不负责 input handling 或 rendering，不能单凭它解决查询响应和客户端增量呈现。本阶段保留为 VT 技术门的对照候选。来源：固定调查提交 [`4239dee`](https://github.com/asciinema/avt/tree/4239deeb3b5d65ad8504585aff9cc39c98aab6a3)。

## 跨端协议编码

- 第二阶段是 Android、第五阶段是 iOS，因此 wire format 不能依赖 Rust 内存布局或 Rust 专用序列化。第一阶段采用版本化 Protocol Buffers schema 与长度前缀 frame：Rust 使用 `prost`，以后 Kotlin/Swift 使用各自标准生成器。
- QUIC 只提供有序字节 stream，不替 zterm 定义消息上限、未知消息策略或业务状态机。协议必须显式限制 frame 大小，并让未知可选 frame 可以按长度跳过；不使用 gRPC，也不在单个 stream 上重新实现跨 session multiplexing。

## Linux libc 发行边界证据

- Rust 官方目标列表同时包含 `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu`、`x86_64-unknown-linux-musl` 和 `aarch64-unknown-linux-musl`；因此支持 musl 在工具链上可行，但不代表 Iroh、bundled SQLite、PTY 和本地 IPC 组合已经在 Alpine x64/arm64 上通过运行测试。来源：[Rust Platform Support](https://doc.rust-lang.org/rustc/platform-support.html) 与 [`aarch64-unknown-linux-musl` 目标说明](https://doc.rust-lang.org/rustc/platform-support/aarch64-unknown-linux-musl.html)。
- 用户已改为官方 direct installer，因此平台选择不再依赖 npm 的 `os`/`cpu`/`libc` 过滤。installer 必须在下载前自行识别 OS、arch、glibc 与已知不支持环境；未来若加入 musl，需要增加对应原生 artifact、target mapping 和 Alpine x64/arm64 实机/容器验收。
- Zedra 自身（不含 vendored Zed 的其他工具）当前 installer 把 Linux 映射为 `unknown-linux-gnu`，只列出 x86_64/aarch64 GNU 产物（`/Users/huyuanzhe/projects/zedra/scripts/install.sh:52-73`）。这说明类似产品先交付 glibc 是有现实依据的缩范围方案，但它不替代 zterm 的产品决定。
- 用户已确认第一阶段只正式支持主流 glibc x86_64/aarch64 发行版，把 Alpine/musl 与 NixOS 原生包装延后。这不改变 core/protocol/state，以后只需扩展 release artifact、installer target mapping 和验收矩阵；代价是首阶段不对 Alpine 或未配置兼容层的 NixOS 承诺可用。

## 设计结论

第一阶段固定 Iroh 1.0.3 与项目工具链 Rust 1.98.0；Iroh自身的Rust 1.91 MSRV只作为依赖兼容事实，不作为zterm的第二条CI工具链。采用 `Minimal + Custom relay + n0 DNS/Pkarr`，一个设备对一条 connection、每个 attachment 独立 QUIC stream。PTY 暂定 `portable-pty` 0.9.x；VT 暂定 `vt100` 0.16.2，但以兼容技术门作为继续实现的前置条件。协议使用 Protobuf，避免第一阶段做出阻塞 Android/iOS 的 Rust-only wire format。
