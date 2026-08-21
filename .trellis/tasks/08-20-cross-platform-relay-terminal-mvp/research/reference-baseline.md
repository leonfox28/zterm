# 参考实现与既有项目基线

## 当前仓库

- `/Users/huyuanzhe/projects/zterm` 目前只有 Trellis/agent 脚手架，没有产品源码、依赖清单或历史提交。因此语言、UI 框架、协议边界和发布方式都尚未由本仓库决定。

## 旧 zTerm 项目

- `/Users/huyuanzhe/prj-code/zTerm` 是一个已有的 Electron 桌面终端项目，而不是概念原型。
- 它已经支持本地 PTY、SSH、系统凭证库、多标签、分屏、文件浏览与自动更新（`/Users/huyuanzhe/prj-code/zTerm/README.md:3-15`）。
- 现有技术栈是 Electron + React + xterm.js + node-pty + ssh2（`/Users/huyuanzhe/prj-code/zTerm/README.md:17-28`），并已有 macOS、Windows、Linux 打包目标（`/Users/huyuanzhe/prj-code/zTerm/package.json:17-61`）。
- 现有架构将 PTY/SSH/SFTP 放在 Electron Main，React Renderer 通过 IPC 使用这些能力（`/Users/huyuanzhe/prj-code/zTerm/README.md:64-78`）。
- 旧项目没有 iroh、移动端或远程 PTY 中继协议。
- 用户已明确：旧项目在 GitHub 上已更名为 `zterm_old`；当前项目只是与它历史重名，是完全独立的新产品，不迁移、不兼容也不复用其功能范围或代码。以上旧项目调查仅保留为排除依据，不参与后续架构选择。

## Zedra 可借鉴的已验证模式

### 平台角色

- Zedra 明确采用非对称架构：Linux/macOS/Windows 运行 `zedra-host` 守护进程和 PTY，iOS/Android 运行控制客户端（`/Users/huyuanzhe/projects/zedra/README.md:11-34`、`/Users/huyuanzhe/projects/zedra/docs/ARCHITECTURE.md:3-24`）。
- 当前 Zedra 并不是“五个平台都同时充当宿主和控制端”的实现证据。

### 连接与协议

- Zedra 当前 Rust workspace 使用 `iroh`/`iroh-relay` 0.96 和 `irpc`/`irpc-iroh` 0.12（`/Users/huyuanzhe/projects/zedra/Cargo.toml:42-56`）。
- 宿主 endpoint 配置自建 relay map；正常模式发布 pkarr 地址，`relay_only` 模式则关闭 IP transport 与发布（`/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/iroh_listener.rs:32-76`）。
- 客户端配置相同 relay map、`PkarrResolver::n0_dns()`，以仅含 endpoint id 的地址发起连接（`/Users/huyuanzhe/projects/zedra/crates/zedra-session/src/connect.rs:472-560`）。
- 建连通常先经过 relay，随后持续探测并可无缝升级为直接 IP path；代码用 path watcher 暴露路径类型与 RTT（`/Users/huyuanzhe/projects/zedra/crates/zedra-session/src/connect.rs:66-78`、`846-934`）。
- QR ticket 的当前源码只包含 host endpoint id、一次性 handshake secret 和 session id；路由信息在连接时动态解析（`/Users/huyuanzhe/projects/zedra/crates/zedra-rpc/src/pairing.rs:20-77`）。`docs/ARCHITECTURE.md:51-53` 关于 ticket 含 relay/direct 地址的文字已经过时，规划应以源码和 `docs/NETWORK_TRANSPORT.md` 为准。
- 配对流程采用 QR possession HMAC、独立 Ed25519 客户端身份、host challenge-response、session ACL 和轮换 session token（`/Users/huyuanzhe/projects/zedra/docs/NETWORK_TRANSPORT.md:9-140`、`144-204`）。
- 每个终端通过独立 QUIC bidi stream attach，并以 sequence/backlog 支持重连后的输出补发（`/Users/huyuanzhe/projects/zedra/docs/ARCHITECTURE.md:101-122`）。

### 中继与验证现状

- Zedra 自建多区域 `iroh-relay`，对端不可直连时转发端到端加密流量；建立 relay path 后仍继续尝试打洞（`/Users/huyuanzhe/projects/zedra/docs/RELAY.md:1-45`）。
- 集成测试覆盖本地 relay 上的 endpoint 互通、RPC framing 和 PTY 输入输出闭环（`/Users/huyuanzhe/projects/zedra/crates/zedra-host/tests/integration.rs:334-425`、`581-660`）。
- 当前测试没有覆盖真实双 NAT 网络、直连升级或移动系统后台恢复，因此 zterm 的验收测试不能只复制现有单机 relay 测试。
- CI 在 Linux 上检查非 app Rust crates、在 macOS 上检查 iOS Rust target；release matrix 构建 macOS/Linux/Windows host。Android app 和真实设备网络行为不在该自动化矩阵内（`/Users/huyuanzhe/projects/zedra/.github/workflows/ci.yml:9-82`、`release.yml:11-38`）。

## 从历史会话确认的风险

- `trellis mem` 会话 `019ffb76-b3f8-74f3-9c41-3f0b94dedc3a`：iroh pkarr 发布是异步的；Zedra 曾在创建新 workspace 后过早让客户端解析，触发 `no addressing information available`。连接状态机应等待可寻址状态或对首次 lookup 做退避重试。
- `trellis mem` 会话 `019fffc2-c287-7542-89ab-d036cb2a07f6`：Zedra 使用 `PkarrResolver::n0_dns()`；iroh 0.96.1 在无法读取系统 DNS 时会退回 Google DNS，Android 更容易触发。DNS/代理兼容性应列为移动端验证项。
- `trellis mem` 会话 `01a01d18-c028-7c43-a422-c2a96e7343a6`：Android 后台无 foreground service/WakeLock 保活，UDP/QUIC path 可能因系统冻结与 idle timeout 失效；回前台探测与重连策略必须作为产品行为明确测试。

## 规划结论

- 最小技术闭环应优先证明：安全配对 → 建连 → PTY 双向流 → resize/close → 断线重连 → 可观察的 direct/relay path。
- `zterm_old` 已被明确排除；后续只研究 Zedra 中可借鉴的 Rust/iroh 网络、配对和移动端经验，并为新产品独立作出技术选择。
