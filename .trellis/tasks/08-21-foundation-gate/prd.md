# Phase 1 Foundation Gate

## Goal

在完整配对、持久 session RPC 和用户界面实现之前，用一组可保留的 vertical spike 证明 zterm 第一阶段最危险的三个技术前提成立：Iroh 官方基础设施能直连优先并可靠回退、控制端消失不会结束或阻塞 PTY、宿主权威 VT 可以让新 attachment 从当前画面继续。Gate 失败时更换被封装的候选实现，不降低已批准的产品语义。

## Background

- Phase Zero 已完成可选的自建 Relay 与发布能力；本 Gate 根据 2026-08-21 的用户决策改用 Iroh 1.0.3 自带的 n0 公共生产 Relay，不再依赖该自建服务。Relay fallback 与 NAT direct 仍是独立路径。
- Gate 0 的可重复网络实验使用现有 Colima Linux VM，在其中临时创建 endpoint、NAT router 和隔离 network namespace/container；不准备两台 Linux VM，也不改动公网服务器。测试结束只删除临时网络资源，Colima 继续作为日常 Docker runtime。
- workspace 固定 Rust 1.98.0 和产品版本 0.1.1；当前五个 crate 仍是无副作用骨架，没有产品 Iroh、PTY、VT 或 session 实现。
- 父任务继续固定 `Iroh 1.0.3`；当前基础设施 profile 从 `presets::Minimal` 加入 Iroh 显式生产 DNS/Pkarr 常量与 production Relay map，等价保留官方 N0 生产能力，同时不允许 staging 环境变量污染。公共 Relay 免费但限速且无 uptime guarantee，只作为研发和当前 Gate 基线；正式生产是否使用托管或自建 Relay 留到发布前决定。
- PTY 首选候选是 `portable-pty 0.9.x`。VT 首选候选是内部 `TerminalModel` 后的 `vt100 0.16.2`；`avt` 或更完整实现只在同一 corpus 证明首选不合格时评估。
- 宿主 daemon 必须拥有权威 VT 状态；不能采用只在客户端保存 VT、宿主仅回放有限 raw bytes 的模型。
- tmux 与固定 Herdr 只作为黑盒通用终端样本。zterm 不识别它们的进程名或协议，也不实现 2.0 专有 Agent 能力。

## Requirements

### R1. Iroh profile and path evidence

- 建立最小的双 endpoint fixture，使用 Iroh 1.0.3 官方 `N0` profile；测试必须枚举并证明 effective map 恰为四个 n0 公共生产 Relay，不含 staging Relay 或自建 zterm Relay。
- 在现有 Colima 内构造 `Endpoint A -> NAT A -> simulated Internet -> NAT B -> Endpoint B` 的可重复双 NAT 拓扑；两个 endpoint 同时可访问 Iroh 官方生产 Relay。拓扑必须由脚本创建和清理，不依赖两台常驻 VM。
- 记录 home Relay、`DirectAddrType` 地址来源和 `path_events()`。先以产品预期配置（官方 QAD 开启、不注入外部地址）测量普通 Home NAT 的自动 direct upgrade；不能用实验专用配置冒充产品已自动直连。当前嵌套 Colima/Patchbay/TUN 的 Case A 未直连只说明该实验环境无法提供自动发现证据；在用户 2026-08-21 明确批准后，Case B 证明 direct 引擎、Case C 证明官方 Relay fallback 即允许 Foundation 继续，自动发现成功率留到父任务 M10 的两条真实网络补验。
- 另设一个“已知外部候选地址”的受控打洞样本，用于区分 Iroh 打洞本身与 reflexive address discovery；再阻断 endpoint 间 UDP，证明同一连接仍通过已部署的 zterm Relay 交换多条独立 QUIC stream。受控样本只是诊断证据，不进入产品 profile。
- Gate 中客户端使用 Iroh 官方 map 随附的 QAD 配置；不得硬编码复制官方 Relay 列表、使用 staging Relay、修改任何 Relay 服务端/防火墙或引入第二个云端组件。
- 网络断开、attachment 丢失或 Relay 路径中断只影响 transport；它们不得成为 PTY 结束条件。

### R2. PTY lifecycle

- 通过 `portable-pty` 启动当前账户的可控测试 shell/fixture，覆盖输入、输出、resize、根子进程退出和显式关闭。
- 没有 attachment 时 reader 仍持续排空 PTY；高输出进程不能因客户端离线填满 PTY 缓冲而阻塞。
- 丢弃客户端 reader/writer、关闭 Iroh connection 或停止测试 transport 不向 PTY 子进程传播意外 HUP/kill。只有根 shell/fixture 退出或显式 session close 才结束该 PTY。
- spike 使用正式 `zterm-platform` 边界，不把 Unix PTY 细节泄漏给未来 Windows ConPTY 调用方。

### R3. Authoritative terminal model

- 在 `zterm-core` 定义最小、库无关的 `TerminalModel` 边界；候选库私有类型不得进入未来 protobuf 或跨 crate 公共协议。
- daemon 侧把每个 PTY byte 顺序送入 parser，维护主/备用 screen、光标、样式、输入 mode、窗口尺寸和有界标准 scrollback。
- 固定 ANSI corpus 至少覆盖 main/alternate screen、清屏、scroll region、光标、256/true color、Unicode 宽字符与组合字符、bracketed paste、mouse/focus mode、连续 resize、DA/DSR 查询。
- 应用查询必须由宿主 terminal 端点按实际声明的 `TERM` 能力响应；未处理的 OSC/DCS/APC 不得被盲目转发到本地控制端产生副作用。
- 任意时刻的 full snapshot 加其 watermark 后连续 delta，必须与直接读取同一最新 `TerminalModel` 状态等价；慢 attachment 可以丢弃中间更新并 resync，但不能阻塞 PTY reader。

### R4. Black-box compatibility and resource baseline

- 自动化使用 deterministic fixture；真实 tmux 与固定提交 Herdr 作为黑盒测试，覆盖交互、resize、无人 attachment 持续输出和重新 attachment 后恢复当前画面。
- Codex/OpenCode 只作为人工全屏 TUI smoke，不对其易变输出写快照断言，也不增加程序名特判。
- 以 16 session、每 session 10,000 行候选 scrollback、512×256 最大候选 viewport 和 256 MiB 全局 terminal-state 预算做测量，不把候选值预先写成产品承诺。
- Gate 报告必须给出典型与高输出下的 CPU、内存、snapshot/delta 体积及最终建议默认值；无论建议值为何，必须证明至少 `main + 2` 三个 session 可用且资源有界。

### R5. Gate output and stop condition

- spike 代码、corpus、fixture、测试和 benchmark 按正式 crate 边界保留；不得创建一次性平行架构。
- 输出书面 Gate 0 报告，记录选定 VT 实现、依赖精确版本、path/PTY/VT/resource 证据、已知兼容差异和继续/停止结论。网络检查点只有在 B direct 或 C Relay fallback 失败时才硬停止；A 在当前嵌套实验室未直连必须记录为 deferred evidence，不能写成自动打洞已通过或官方 QAD 普遍失败。
- Gate 未通过前不实现完整 pairing、授权数据库、持久 session registry、远程 CLI UX、installer 或移动端协议。
- 若 `vt100` 不满足同一 corpus，保留 `TerminalModel` 和测试更换实现；若无法在资源有界前提下支持三个 session，或无法保证无 attachment 持续排空，则停止后续里程碑并报告原因。

## Acceptance Criteria

- [x] Iroh profile 的 effective map 恰为 v1.0.3 官方四个生产 Relay，均带官方 QAD 配置，且不含 staging 或自建 Relay；当前嵌套 Home NAT × Home NAT 的 Case A 未观察到自动 direct，报告明确把它限定为实验环境证据不足并把真实双网络验证延期，未用注入地址冒充产品自动发现通过。
- [x] 已知外部候选地址的对照样本能观察 direct path；阻断 endpoint 非 DNS UDP 后，两个 endpoint 仍能通过官方 Relay 建连并交换多条独立 QUIC stream。
- [x] 双 NAT 实验可在现有 Colima 内一条命令创建并清理；清理后不残留测试 container、network namespace、route 或 NAT rule，也不要求第二台 VM。
- [x] QAD 仅来自 Iroh 官方 profile；自建 Relay、OpenResty、Cloudflare、OCI 和防火墙未因 Gate 测试改变。
- [x] PTY fixture 的输入、输出、resize、无人 attachment 高输出、客户端/transport 丢失、根进程退出与显式关闭语义全部可重复测试。
- [x] 固定 terminal corpus 通过；snapshot + watermark 后 delta 与最新权威状态等价，慢 attachment resync 不反压 PTY。
- [x] tmux 与固定 Herdr 黑盒基线通过，且实现中没有程序名白名单；Codex/OpenCode 无提示词 smoke 的结果只记录能力/缺口。
- [x] 资源报告给出实测数据和最终候选默认值；至少三个 session 在有界内存与输出队列下可靠运行。
- [x] macOS arm64 实机测试通过；macOS x86_64、Linux x86_64/arm64 已完成 hosted CI 编译与可在相应 runner 执行的非平台特有测试，Windows x86_64 完成相同 workspace CI并保留非 Unix边界。
- [x] Gate 0 报告明确给出最终 go；format、Clippy、test、doc、依赖策略及相关集成测试全绿。Case A 的 deferred evidence、后续 PTY/VT/resource 结果和真实双网络补验位置分别写清，未伪装成已经通过自动打洞成功率验证。

## Out of Scope

- 完整 daemon 生命周期、SQLite schema、配对/授权/撤销、正式 session RPC 和 CLI 交互界面。
- Android、Windows、桌面 GUI 或 iOS 实现；本 Gate 只保留不会阻塞它们的抽象边界。
- 专有 Agent 状态识别、通知、Codex/OpenCode 输出解析或完整 TUI 对话历史。
- 自建/托管 Relay 的新部署、UDP/证书/防火墙变更、公共 Relay 的性能 SLA，或正式生产基础设施承诺。
- 两台真实异构 NAT 后设备的成功率矩阵；Gate 0 只证明路径语义和可重复性，家庭宽带、蜂窝网络、企业网等真实网络验证留到父任务 M10。
- OSC 52、OSC 8、Kitty graphics 等增强能力；它们不得破坏基础文本终端，但不作为 Gate 通过条件。
