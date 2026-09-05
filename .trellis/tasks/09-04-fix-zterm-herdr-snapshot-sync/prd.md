# 统一 zterm 本地/远程 Daemon 连接并修复快照同步错误

## Goal

把 zterm terminal view 统一建模为“连接一个目标 daemon 并 attach 其 Session”。目标 daemon
只拥有一套 Session/attachment/PTY/terminal 状态，不拥有“本地 view”和“远程 view”两种
运行模式。已选择 daemon-owned network + opaque stream tunnel：本机每个 CLI 的 SessionClient
通过自己独立的 same-UID IPC connection 直达本机 daemon Session；远程每个 CLI 通过自己独立
的 same-UID IPC tunnel，映射到 viewer daemon 所持唯一设备间 Iroh connection 中的一条 QUIC
service stream，再直达目标 daemon Session。viewer daemon 只拥有身份、连接、stream admission
和 path observation，不拥有或改写 attachment/revision/resume 语义。两种 route 共享同一
SessionClient、终端界面与底部状态栏。同时，直接执行 `zterm` 连接本地 daemon 后，启动会切换
主/备用屏幕的 TUI（用户报告的 Herdr 0.8.2）时，不再因错误确认快照而退出，并继续保持正确的
viewport、输入和同步状态。

## Background and Confirmed Facts

- 用户可稳定观察到：直接执行 `zterm`，在其中输入 `herdr` 后客户端立即退出并报告
  `not_synchronized: attachment is not awaiting a snapshot`。
- 用户确认当前通过 `zterm connect <target>` 的远程路径运行 Herdr 正常。代码解释了这一可见
  差异：`crates/daemon/src/remote_attachment.rs:1403-1421` 的现有 semantic bridge 只在自身
  `EpochPhase::Synchronizing` 且 revision 精确匹配时才把 `TerminalSnapshotApplied` 转发给目标，
  其他状态或 revision 会被本机 bridge 直接丢弃；本地路径则把 UI 命令直接交给目标
  `SessionWireServer`，因而暴露目标端严格的 `not_synchronized` 校验。当前远程成功说明旧 bridge
  遮蔽了公共 UI 的错误命令，不说明未来 D 路径中的公共 SessionClient 已经正确。
- 用户确认新的产品不变量：连接本地 daemon 和连接远程 daemon 是同一种“连接目标 daemon”
  操作；local/remote 只描述到达目标 daemon 的 connection route，不能形成两套 terminal UI。
- 用户进一步确认状态所有权：例如 Mac 上运行目标 daemon 时，Mac 本机执行 `zterm` 与另一台
  电脑执行 `zterm connect Mac` 最终都 attach Mac daemon 中同一个 SessionService/SessionActor；
  目标 daemon 不得因入口是 same-UID 还是 authenticated remote 而建立两类 terminal/attachment
  状态。不同入口只保留鉴权身份和 transport recovery 所必需的信息。
- 2026-08-20 原始讨论中，用户明确要求的是：同一控制设备查看目标 daemon 中多个持久 Session
  时，未来 GUI/Android 的 tab 或卡片必须复用一条设备间 connection，不能每个 Session 建一条
  connection。此前设计又从“首版可用本地 terminal emulator 的多个 tab/window 分别运行 CLI”
  推导出多个 desktop CLI 进程共享 daemon connection pool；后者不是用户独立提出的硬需求。
- 设备 A 到设备 B 只有一条设备 connection，设备 C 到 B 则是另一条 connection；A 与 C 若
  attach B 上同一个 SessionId，面对的是 B daemon 中同一个 SessionActor、PTY、TerminalModel
  和其中同一个 Agent 进程，而不是两份同步副本。
- 用户已确认硬不变量：每一对设备只有一条活动 network connection。这里的 connection 是
  设备级已认证 Iroh/QUIC 通道，不等于一个 terminal 界面，也不等于一个 Session。A 可以运行
  多个 CLI，未来 GUI/App 也可以有多个 tab；每个可见 view 各用同一 A→B connection 上的一条
  独立 stream attachment，可指向相同或不同 Session，不会建立第二条 A→B network connection。
  用户已撤回“A 上不会有多个 CLI”的限制，不得再把单 frontend 假设作为架构前提。
- 用户确认 local 状态栏只显示 `<device> | local` 两段，不显示延迟、第三段或占位符；remote
  状态栏继续显示目标、direct/relay 和 RTT 三段。
- `.trellis/tasks/archive/2026-09/08-31-remote-terminal-ux/prd.md:48-55,80` 曾明确把状态栏限定
  为 remote attachment，并把 local attachment 状态栏列为 out of scope；当前 remote-only
  行为忠实于当时的显式要求，并非当时漏做。用户本次要求取代这一旧的产品范围决定。
- `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/design.md:20-44,341-359` 与对应 PRD
  `:170-178` 已规定：桌面 CLI 始终通过 same-UID IPC 进入自己的 daemon/connection broker；
  self target 直接进入本机 SessionService，remote target 由 broker 经共享 Iroh connection
  转发同一 Session 协议到目标 daemon。两条 adapter 共享同一个 SessionActor、PTY、VT
  model、revision 和 controller lease。
- 远程调用的完整拓扑包含两个 daemon：远端电脑上的本地 daemon 是出站 broker，Mac daemon
  才是目标和 Session owner。direct/relay、RTT、网络断线与重连属于出站 broker/route；Mac
  daemon 的 SessionActor 只看同一 attachment 合同。`AttachmentPrincipal` 仍用于 same-UID/
  authenticated-device 鉴权、撤销和 resume 身份绑定，但 principal kind 不得改变终端语义。
- 用户已选择 D 架构：CLI 不持有设备私钥、不创建 Endpoint；viewer daemon 独占身份、Endpoint
  和每设备对唯一 connection，但只提供 authenticated opaque stream tunnel。每个 CLI 拥有自己
  的 IPC connection/tunnel 和 SessionClient。self target 的 IPC 直接终止于本机 SessionWireServer，
  不 self-dial Iroh、不查询地址服务、不经过 relay；remote tunnel 的 Session payload 由 CLI 与
  target daemon 端到端解释，中间 daemon 不解码或改写。
- `crates/daemon/src/device_directory.rs:57-93` 的
  `ResolvedSessionTarget::{Local, Device}` 和 `crates/daemon/src/local_ipc.rs:648-770,
  2033-2078` 的 Session request 已统一表达目标，但
  `crates/daemon/src/operations.rs:392-475` 的 operations/UI 边界又使用
  `remote_alias: Option<String>` 同时推导目标显示名、状态栏是否启用、是否为远程连接、初始
  同步状态和是否接受远程连接事件。目标 daemon 抽象因此没有贯穿到 presentation。
- `crates/cli/src/terminal_ui.rs:651-710` 在处理一个连续 delta 时，可能因 Main↔Alternate
  布局变化先提交 mode-driven resize，并把本地 transport state 改为 `Synchronizing`；同一
  分支随后仅根据修改后的 state 调用 `snapshot_applied(delta.to_revision)`。
- 统一 local/remote 的 target/view/UI 路径只能消除两套实现的分叉，不能自动证明公共路径
  正确。状态栏统一会让两类连接使用相同行数，但 Main screen 预留 gutter、Alternate screen
  使用全宽的列数变化仍然存在；已经处于 Active 的任一路由都可能进入同一个 delta/resize
  决策。因此 Herdr 修复不能由连接架构重构隐式替代。
- resize 与 snapshot acknowledgement 通过同一 duplex stream 顺序写入，但 revision writer
  与 request reader 并行。daemon 因 resize 产生的新 revision 可能尚未或已经把 attachment
  转为 `Awaiting`：旧 delta acknowledgement 因而分别会直接命中 `not_synchronized`，或被
  当作 revision mismatch 触发额外 replacement snapshot。后一种顺序随后会在重复确认时命中
  同一错误。
- `crates/daemon/src/session.rs:3470-3505` 已正确严格执行既有合同：只有 `Awaiting`
  attachment 才接受 acknowledgement，且 revision 必须精确匹配；该服务端约束不应放宽为
  幂等忽略。
- 既有 `cargo test -p zterm-cli --test daemon_autospawn` 通过，但
  `crates/cli/tests/daemon_autospawn.rs:120-124,365-384,638-850` 未让真实 `run_terminal`
  路径触发 Main↔Alternate mode-driven resize，所以没有覆盖本次竞态。
- 既有 `tests/foundation/terminal-blackbox.sh --mode herdr` 在 Herdr 0.8.2、alternate screen、
  resize 和 resync 场景通过，说明 terminal parser/model/driver 能正确承载 Herdr；快照缺陷
  位于上层 attachment UI 的同步决策。
- 生产代码不能按 Herdr、进程名、屏幕文字或其他应用身份分支。

## Root-Cause Classification

### A. Herdr 触发 `not_synchronized`

**Localized viewer implementation defect（不是 Local route 专属缺陷）.** attachment/session
架构已有单一明确的 acknowledgement
合同；CLI 的 delta handler 把“进入该事件前已处于同步、因此该 delta 是激活屏障”与“处理该
delta 时由客户端自己发起 resize、因此接下来应等待新快照”混成同一个修改后的
`Synchronizing` 状态。修复应保留事件入口处的同步语义，并只确认真正承担既有同步屏障的
连续 delta。

这不是由旧 semantic broker 引起的 architecture/boundary defect：target daemon 已拥有唯一
attachment sync state 和严格 revision 校验；修复不需要新增第二份服务端真相、放宽协议或应用
识别。D 会把 remote desired-view 状态移到公共 SessionClient，但该状态机仍必须显式修正这一
错误转移。旧 remote semantic bridge 当前会过滤这条无效 acknowledgement，所以同一个 UI bug
在 remote 上没有变成用户可见错误；D 删除该过滤层后，如果只改路由而不修状态机，remote 也会
开始暴露它。

也就是说，Herdr 不存在合法的“local 行为”和“remote 行为”。如果同一个目标 Session 在两条
route 上结果不同，就是 route 泄漏或公共状态机错误；网络时序只能改变 bug 的触发概率，不能
成为可接受的语义差异。完成 D 与状态机修复后，等价事件序列必须是两条 route 同时成功或测试
同时失败；当前旧 Remote 路径成功不能作为跳过状态机修复的依据。

### B. 目标 daemon 抽象未贯彻到 presentation 边界

**Presentation-boundary defect exposed by a product-scope change.** remote-only 状态栏是此前
明确选择的产品范围，因此当前行为不是对当时要求的实施遗漏；核心 SessionService、local
adapter 与 remote adapter 也已按“同一目标 daemon 服务、不同 route”设计。缺陷在于该抽象
没有继续贯穿 operations → prepared view → terminal UI 边界，新要求使这个缺口成为必须修复
的架构边界问题。

实现不能只把 local 名字填进现有 optional alias 或把一个 UI 布尔量改成 `true`；必须将所有
attachment 都有的目标 daemon 身份/显示信息、route 特有的 direct/relay 路径观察与 reconnect、
以及 route 无关的 attachment 同步生命周期分别建模。wire 仍只承载真正的远程网络 path
sample，不应为了本地静态状态栏伪造远程事件，也不应重写已经统一的 SessionService 或
connection broker。

### C. Viewer daemon 持有 remote attachment 语义

**Superseded architecture decision（不是原实现遗漏）.** 既有 daemon-owned
`DesiredAttachment`、stable-local/epoch-remote ID 映射和 reconnect bridge 是早期“一 daemon
同时充当 host 与完整 controller broker”设计的有意实现。用户现已选择更窄的边界：daemon 仍
必须持有唯一设备身份和共享 network connection，但 active CLI/GUI 的公共 SessionClient 持有
attach/resume/revision/mutation 语义。现有厚 bridge 因新产品决定成为需要替换的架构边界，而
不是靠局部修补继续扩展的实现 bug。

## Requirements

### Unified target-daemon connection model

1. 每个 terminal request 必须先解析为一个稳定的目标 daemon，再由 transport factory 给公共
   SessionClient 提供一个 duplex Session stream：Local 是该 CLI 自己的 same-UID IPC connection，
   直接进入本机 SessionWireServer；Remote 是该 CLI 自己的 same-UID IPC tunnel，经 viewer
   daemon 打开的 authenticated QUIC service stream 进入 target SessionWireServer。两者不得使用
   两套 Session client/event/command 合同。
2. 目标 daemon 的稳定身份、用户安全显示名和发起端 connection route 必须是显式且彼此独立的
   typed metadata；不得再用 alias 是否存在、状态栏是否启用、selector 字符串或 UI 布尔量
   反向推断 route。route metadata 只供发起端 broker、连接观测和状态栏投影使用，不进入目标
   SessionService/SessionActor 的 terminal 决策。
3. 每个活动 CLI/view 必须拥有一条独立 local IPC connection。对 Remote，该 IPC tunnel 在每个
   stream epoch 映射 viewer daemon 与 target daemon 的共享 Iroh connection 中一条独立 QUIC
   service stream；多个 CLI 不共享同一 IPC byte stream，也不得为同一设备对建立第二条 network
   connection。
4. viewer daemon 只验证 same-UID caller、冻结 exact target、检查 outbound authorization、取得
   connection demand、打开有界 service stream、执行背压安全的 opaque payload 转发，并在独立
   tunnel control plane 投影 connection/path 状态。它不得解码 Session payload、生成/替换
   ResumeViewId、缓存 Session revision/viewport、改写 attachment ID、排队 terminal control，或
   替 CLI 重试 attachment/mutation。
5. CLI/GUI 进程中的 transport-independent SessionClient 是 Local 与 Remote 共用的唯一 client
   语义 owner。Remote stream 断开时，它保留 stable ResumeViewId、frozen SessionId、last applied
   revision 和 latest viewport，重新申请 tunnel 并 reattach/resume；Local daemon/IPC 消失按
   target daemon 生命周期错误结束，不伪装成 remote reconnect。
6. SessionService、TerminalModel、revision、controller lease、snapshot/delta/input/resize/
   detach 合同继续只有一套。目标 Session 可以接收鉴权所需的 same-UID 或 remote-authenticated
   principal，但 principal kind 对 controller 没有优先级，也不得选择不同的渲染、resize、同步
   或 acknowledgement 状态机；connection/path 观测只留在 viewer transport adapter。

### Unified attachment chrome

7. 物理终端至少两行时，local 与 remote attachment 都必须在最底部保留一行 zterm 状态栏；
   child viewport 均先扣除该行，再应用共享资源上限。物理终端只有一行时，两者都临时隐藏
   状态栏并把唯一一行留给 child，恢复到至少两行后自动显示。
8. 两类 attachment 必须复用同一个 status composition、显示单元截断、reverse-video 整行
   样式、cursor/style 保存恢复和 resize 投影，不建立 local 专用 renderer。
9. 第一字段始终是 attach 时冻结的目标设备安全显示名：remote 使用本地目录中的目标 alias；
   local 使用当前本地 daemon 已提交配置中的 device name。显示信息不得决定 route、路由身份
   或同步生命周期。
10. local 状态栏严格显示 `<device> | local` 两段，不显示第三段、延迟或尾随 ` | --`。
11. remote 状态栏继续显示
   `<device> | <direct|relay|--> | <integer ms|-->` 三段；RTT 是当前 selected network path
   的有界整数毫秒值或 `--`。
12. local 状态稳定且不消费 remote connection-status event；remote 的 unknown、direct、relay、
    reconnect、path migration 和 RTT 刷新行为保持不变。这些是发起端 connection route 的旁路
    观测，不是目标 daemon 的 attachment 状态，也不得改变 initial acknowledgement、takeover、
    resize 或普通 delta 处理。
13. 状态栏不得显示 Device ID、IP、Relay URL、ticket、Unix socket path、终端内容或其他敏感
    transport 信息，也不得进入 child PTY input/output。

### Snapshot synchronization

14. CLI 必须根据处理 delta 之前的 attachment transport state 决定该 delta 是否需要
    acknowledgement；同一 delta 触发的 mode-driven resize 不得把旧 delta 误判成新快照屏障。
15. 已处于 `Synchronizing` 的连续 delta（例如 remote resume/reconnect 的激活 delta）仍须按
    其精确 `to_revision` 确认，保持现有 reconnect 和 takeover 行为。
16. Active 状态下的 Main→Alternate 与 Alternate→Main 各自仍只提交一次所需 viewport
    resize；resize 产生的 replacement snapshot 由 snapshot 路径精确确认，不能产生同步循环
    或重复确认。
17. daemon 的 `AttachmentSync::Awaiting` 严格校验、target Session acknowledgement 消息合同
    和错误类型保持不变；不得通过忽略重复 acknowledgement 掩盖客户端状态错误。
18. 修复必须应用无关，生产代码和测试断言均不得依赖 Herdr 进程名、版本、输出文字或启动
    时序。
19. 连接模型与状态栏统一不得被视为 snapshot 修复的替代品；公共 delta handler 不得接收或
    查询 Local/Remote route 来作 acknowledgement 决策。两条 route 投影出的相同 attachment
    事件必须执行同一个事件入口 acknowledgement 转移。

## Selected Architecture: Daemon-owned Network, CLI-owned SessionClient

1. **network owner = viewer daemon**：设备私钥、Iroh Endpoint、寻址、direct/relay、认证
   connection、connection pool、stream admission 和 path observation；
2. **Session-client owner = active CLI/GUI process**：attach/resume、target attachment ID、last
   applied revision、latest viewport、同步、控制请求及 mutation retry/ambiguity。

Local 与 Remote 的准确拓扑是：

```text
Local CLI 1 -- IPC connection 1 -----------------> local target daemon SessionWireServer
Local CLI 2 -- IPC connection 2 -----------------> local target daemon SessionWireServer

Remote CLI 1 -- IPC tunnel 1 -> viewer daemon -- QUIC stream 1 --┐
Remote CLI 2 -- IPC tunnel 2 -> viewer daemon -- QUIC stream 2 --┼-> target daemon SessionWireServer
                                      one shared Iroh connection ┘
```

每个 CLI 有独立 IPC connection；多个 CLI 只共享 daemon 的 listener/socket path、设备身份和
设备间 Iroh connection，不共享一条 IPC byte stream。每个 remote tunnel 在任一时刻承载一个
QUIC stream epoch；stream/connection 失败后由该 CLI 的 SessionClient 重新申请 tunnel 并 resume。

viewer daemon 的 tunnel control plane 可以承载 opened/failed/closed、backpressure/half-close
和 direct/relay/RTT 等有限 transport metadata；Session payload 必须作为 opaque bounded bytes
封装，daemon 不得在 payload 中注入会破坏 frame 边界的事件，也不得理解 terminal 语义。

未选择最直观的 **shared-key CLI-direct**：

```text
CLI -> Iroh -> target daemon
```

Iroh 1.0.3 的 `same_endpoint_id_relay` 上游测试证明：第二个使用同一 SecretKey/EndpointId 的
Endpoint 连上 home relay 后，新流量会转给第二个 Endpoint，第一个收到“不再接收消息”的警告。
因此 daemon 与多个 CLI 不能同时加载现有设备密钥建 Endpoint；这会互相顶掉 relay 可达性，也
破坏当前每设备一个 primary connection 的假设。

即使产品明确规定 A 同时只运行一个连接 B 的 CLI，这个冲突仍未消失：A 的 daemon 本身也要
保持 Endpoint，才能让 A 同时作为 target 被其他设备连接。除非产品禁止设备同时充当 host 和
controller，或重做 transport identity，否则该 CLI 仍不能直接复用 daemon 的长期密钥。

“每个设备对最多一条主 connection，多个 CLI/GUI/Session attachment stream 在其上复用”现已
确认。由于多个独立 CLI 进程不能共同持有一条 userspace QUIC connection，network owner 必须是
A 上所有 frontend 都能访问的共享长驻进程；在当前产品中就是 A 的 daemon。若另设 connection
manager，它在架构角色上仍然是一个 daemon/broker，并没有消除中间 owner。

用户已选择 **D. daemon-owned network + opaque stream tunnel**。现有
**A. daemon-owned semantic broker** 是被替换的旧边界：其 `DesiredAttachment`、两套 attachment
epoch、ID 改写、revision/viewport 缓存、控制排队和重连语义必须迁移到 frontend 进程中的公共
SessionClient 或被删除，而不是继续作为 remote 特例。

**C. delegated-identity CLI-direct** 只在放宽“一对设备一条 connection”或重新禁止并发 CLI 时
成立：每个 CLI 的临时 Endpoint 都会建立自己的 connection。若 C 再通过共享进程复用 connection，
它就退化为 D。它因此不再是当前确认产品不变量内的候选。

`research/remote-route-ownership-options.md` 保留完整比较和排除依据。D 的选择不替代 Herdr
acknowledgement 修复：它让 Local 与 Remote 运行同一个 SessionClient 状态机并消除中间语义
bridge，但当前错误位于该公共 viewer delta/resize 决策。若迁移时原样保留错误转移，两个 route
都会错误；若时序变化使症状暂时消失，也不构成修复。必须用应用无关回归先证明失败，再显式
修正并让同一测试跨 Local/Remote transport fixture 通过。

## Acceptance Criteria

- [ ] local/remote 的 list/create/attach/rename/close/takeover 继续经过同一个 resolved-target 与
  Session 协议入口；prepared/live view 的公共消费者不再依据 selector 字符串或 optional alias
  判断连接类型，route-specific 行为只读取显式 route metadata。
- [ ] 对同一目标 daemon 而言，本机 `zterm` 与另一设备 `zterm connect <target>` 最终调用同一个
  SessionService/SessionActor 合同；目标端没有 Local/Remote terminal mode。除 principal 鉴权、
  remote resume 身份绑定外，target Session 的状态转移不按入口类型分支。
- [ ] local 与 remote attachment 在相同物理终端尺寸下得到相同的 chrome 几何；例如 24x80
  物理终端中，两者的 Main child viewport 均为 23x79，Alternate 均为 23x80，最底行为状态栏。
- [ ] local 状态栏严格按 `<device> | local` 两段显示且没有尾随第三段；remote 继续严格按
  `<device> | <direct|relay|--> | <integer ms|-->` 显示。两者使用相同 reverse-video 整行样式、
  Unicode 显示单元安全截断和 cursor/style 恢复。
- [ ] local 目标名来自 daemon 的已提交 device name；remote 目标名仍是 attach 时冻结的安全
  alias。目标名存在本身不会让 local 启用 remote route reconnect 或 connection-event 路径，
  也不会改变公共 attachment synchronization。
- [ ] 单行、窄窗口、连续 resize、main/alternate transition、initial snapshot、delta、history
  pinned、detach 和 terminal restoration 下，两类 attachment 均保持相同 chrome 规则且不振荡。
- [ ] `N` 个 local CLI 使用 `N` 条互不共享字节流的 same-UID IPC connection；`N` 个 remote
  attachment 使用 `N` 条 IPC tunnel，并在同一设备对当前唯一的 Iroh connection 上映射为 `N`
  条独立 QUIC service stream。不得退化为 `N` 条设备间 connection，也不得让多个 CLI 复用同一
  条 IPC byte stream。
- [ ] local direct IPC 与 remote opaque tunnel 向 frontend 暴露同一套 transport-independent
  SessionClient/Session frame contract；viewer daemon 只解释 tunnel envelope 和 transport
  metadata，不解码、改写、确认或重试内部 Session payload。target daemon 生成的 attachment ID
  原样由 frontend 持有。
- [ ] 外网、DNS/Pkarr 与 relay 不可用时 local target 仍可工作；remote target 保留
  direct/relay migration、认证、撤销与重连行为。多个 desktop view 继续复用 viewer daemon 的
  Iroh connection；单条 tunnel/service stream 失败只中断对应 view，peer connection 丢失时各
  个存活 SessionClient 分别重开 tunnel 并使用自身 ResumeViewId/known revision 恢复。
- [ ] 一个使用真实 `run_terminal`、本地 daemon 和外层 pseudo-TTY 的自动化回归，通过通用
  DECSET/DECRST alternate-screen 序列触发 Main→Alternate→Main，并在修复前稳定暴露快照
  错误、修复后正常完成。
- [ ] Active delta 自己触发 resize 时，不发送该旧 delta 的 `TerminalSnapshotApplied`；入口处已
  `Synchronizing` 的连续 delta 仍发送精确 acknowledgement。
- [ ] route metadata 不进入公共 delta/resize/snapshot handler；本机和远程 viewer 对相同目标
  attachment 事件得到相同 acknowledgement、viewport 和继续运行结果，唯一可见差别是约定的
  状态栏内容以及真实网络断线期间的连接提示。
- [ ] 用户报告的 `zterm` → `herdr` 路径不再出现
  `not_synchronized: attachment is not awaiting a snapshot`，且 Herdr 退出后仍可回到原
  Session shell。
- [ ] 相关 CLI composition/unit/PTY 端到端测试、daemon operations/session/IPC 同步测试、
  两 daemon direct integration 和 Herdr 0.8.2 黑盒继续通过；relay 状态投影与现有 reconnect
  合同回归保持通过。本任务不以外部公网 relay 可用性作为代码正确性的新增前置条件。

## Out of Scope

- 不修改 Herdr、增加 Herdr 专用分支或改变 TUI 输入/渲染策略。
- 不放宽 daemon acknowledgement 合同，不改变 Session 持久化模型。
- 不让桌面 CLI 读取设备私钥、创建独立 Iroh Endpoint 或绕过本地 daemon/connection broker；
  self target 不自配对、不自拨 Iroh，也不依赖网络可用性。
- 不把 same-UID IPC 与 authenticated Iroh 强行压成同一种认证、重试或重连实现；统一的是目标
  daemon/Session/attachment 合同，发起端 route adapter 保留物理与安全语义差异。目标 daemon
  仍可识别 principal 以执行鉴权、撤销和 remote resume，但不能据此选择不同终端行为。
- 不重新设计 gutter、history viewport、selection、clipboard 或一般 resize UX。
- 不增加 local IPC RTT 测量、local 第三字段、额外状态栏字段、颜色配置或网络地址诊断。
- 不承诺 local hop、IPC copy 或 daemon failure domain 的成本为零。D 的目标是移除 remote
  semantic proxy，而不是规避所有本机开销；本任务要求有界背压和多 view 故障隔离测试，完整
  throughput/RSS benchmark 可另立 architecture/performance task。
- 不改变 target Session wire major、持久化 schema 或 acknowledgement contract；允许在仅限
  same-UID local IPC 的 viewer-daemon tunnel control/envelope 中增加版本化消息。

## Risks and Validation Notes

- 统一状态栏会改变 local child 的行数，并与 Main/Alternate 引发的列数变化共同进入 resize/
  snapshot 路径；状态栏统一和同步修复应在同一设计中验证，避免分开修改后遗漏组合状态。
- `remote_alias: Option<_>` 当前跨越 presentation 与 transport 边界；重构必须用显式 target
  daemon + route metadata 取代这一推断，防止 local 因拥有 display name 而被误判为 remote，
  也必须防止 remote reconnect/RTT 语义退化。
- 快照缺陷包含 request reader 与 revision writer 的两种合法调度顺序；回归必须验证最终可观察
  行为和 viewport 收敛，不能依赖某一个线程调度或仅测试一个布尔 helper。
- Herdr 是外部 smoke fixture；主要回归使用通用 alternate-screen 控制序列，以覆盖同类 TUI。
- 本任务跨 daemon operations / CLI composition / PTY integration，属于复杂任务；开始实施前
  必须有完整 `design.md` 与 `implement.md`。
- D 已被选定：viewer daemon 是共享 network owner，CLI/GUI 是 Session-client owner；不得在实现
  中重新引入 daemon-owned desired attachment、ID 改写或 revision/viewport 语义缓存。shared-key
  direct 已因 Iroh 同 EndpointId 冲突被排除，delegated direct 又因每 frontend 独立 connection
  违反已确认的不变量。
- tunnel/SessionClient 重构可能改变调度，让 Herdr 症状暂时消失或更稳定地出现；只有明确的
  acknowledgement 状态机回归通过才算修复，时序上的“测不到了”不算。
