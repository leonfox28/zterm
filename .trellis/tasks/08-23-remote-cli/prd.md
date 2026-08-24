# 远程 Session 与公开 CLI

## 目标与用户价值

完成父任务 Phase 1 的 M7-M8：让已配对的 macOS/Linux 设备通过 daemon 已有的
Iroh connection broker 使用同一个持久 `SessionService`，并提供可日常使用的 `zterm`
命令行。用户可以在远端 `main` 或命名 Session 中工作，断网、CLI 退出或 direct/Relay
路径变化时宿主 PTY 继续运行；重新连接后回到同一 SessionId、进程、cwd 和权威终端画面。
宿主本机 CLI 仍通过 same-UID IPC 直接接续完全相同的 Session，不依赖 tmux、Herdr、
Iroh、DNS 或 Relay。

## 已确认事实与产品决定

- M4 已交付唯一、transport-independent 的 `SessionService`、持久 PTY/TerminalModel、
  local unary/duplex IPC、snapshot/delta、operation replay、单 controller 与显式 takeover。
  本任务必须复用它，不能创建 remote Session registry、第二套 terminal parser 或第二套 replay。
- M5-M6 已交付 daemon 唯一 Iroh Endpoint、官方 n0 production profile、按 remote DeviceId
  singleflight 的 `ConnectionDemand`、认证 generation、受限 bidirectional stream、配对和撤销。
  当前业务 stream 在认证后有意返回 `service_not_implemented`；这正是 M7 接入点。
- 一台控制设备到一台宿主的多个 Session 共用一条 normal-ALPN Iroh connection；每个
  attachment 和 control RPC 使用独立 bidirectional stream。连接或 stream 不是 Session 生命周期。
- setup 后裸 `zterm` 等价于 `zterm connect local --session main`；未 setup 时只提示
  `zterm setup`，不得静默生成 identity。`zterm --help` 始终是帮助入口。
- 首版每个 CLI 进程只显示一个 Session，不实现内嵌 tab UI。默认进入 `main`，用户通过
  `session` 子命令管理其他 Session，并可在多个本地终端窗口同时连接不同 Session。
- `local` 是保留 target，不能成为设备 alias。local attach 没有隐式优先权；现有 remote
  controller 在线时普通 attach 返回 occupied，只有 `--takeover` 原子转移控制权。
- 默认本地控制前缀是可配置、可禁用的 `Ctrl+]`：`Ctrl+] .` 仅 detach，
  `Ctrl+] Ctrl+]` 向 PTY 发送一个原始 `Ctrl+]`。
- `pair accept` 默认从不回显的交互 TTY prompt 读取 ticket；自动化必须显式使用
  `--stdin`。完整 ticket 不得出现在 argv、shell history、环境、日志、错误或 Debug 输出。
- 第一阶段运行平台仍是 macOS 与主流 glibc Linux（x86_64/arm64）。Windows 只保持
  shared core/proto/daemon/CLI 编译与明确 unsupported 边界，不宣称已有 Named Pipe/ConPTY。
- 官方 n0 是产品默认 Relay。可选 self-hosted profile 保持隔离；本任务不新建 public/self-hosted
  Relay acceptance workflow。代表性双物理网络与自动发现仍属于父任务 M10。

## 功能需求

### R1. 入站远程 Session 服务

1. normal `zterm/1` connection 上已认证的业务 stream 必须按首 frame 精确分为 unary
   Session control 或 duplex terminal attachment；unknown kind、trailing unary bytes、malformed、
   oversize 和 stalled first frame 只关闭对应 stream，不关闭 connection、Session 或 PTY。
2. remote list/create/rename/close/operation-lease/attach/takeover/input/resize/
   `SnapshotApplied`/sync/detach 调用唯一 `SessionService`。remote principal 固定为
   `{DeviceId, accepted authorization generation}`；请求中的 target 必须精确等于当前宿主 identity。
3. 每个会读取或改变宿主 Session/PTY 的 remote 调用，在副作用提交点取得匹配 generation 的
   `AuthorizedCommitContext`，并持有到同步 SessionService/PTY 操作完成。等待中的 revoke writer
   不能被新请求越过；revoke 返回后旧 generation、排队帧和在途旧 controller 都不能再提交。
4. remote adapter 只拥有 framing、principal、deadline、target 与 domain/proto 转换；registry、
   replay、controller、resource、snapshot 和 PTY 语义仍只有 SessionService 一个 owner。

### R2. 出站远程 control RPC 与 target 解析

1. CLI 只连接本机 same-UID daemon。daemon 根据 `local` 或经过本地设备目录解析出的完整
   DeviceId 选择 local adapter 或 `ConnectionBroker`；CLI 不读取 SQLite/identity.key，不绑定
   Endpoint，不执行 DNS/Relay 查询。
2. remote unary RPC 持有一个 `ConnectionDemand`，在 promoted normal connection 上打开独立
   service stream。read-only RPC 可在 transport ambiguity 后安全重试；mutation 只能在同一
   absolute deadline 内以 byte-identical frame、相同 daemon-issued lease 和 OperationId 重试一次。
   `SessionOperationLeaseRequest` 是 stateful control；post-write ambiguity 返回原 typed transport/
   protocol failure，不打开第二条 remote stream，避免分配两个 replay lease。
   完整 typed response（包括 outcome unknown）是终态，不能换新 lease 重跑同一逻辑操作。
3. selector 接受精确 alias 或完整 canonical DeviceId；短 ID 不用于 mutation，避免碰撞。
   `session` selector 接受稳定 SessionId 或精确名称。方向不允许控制时返回明确的 device/
   authorization error，而不是把 outbound-known 与 inbound-authorized 混为一谈。
4. 多个本地 CLI/多个 control RPC 对同一 remote 复用 broker 的同一 primary connection；
   一个慢/失败 stream 不阻塞其他 Session 或 local daemon IPC。

### R3. 可重连的远端 attachment bridge

1. 一个仍存活的本地 CLI view 对一个目标 Session 保持一个 daemon-owned desired attachment。
   bridge 在 remote connection 丢失时保持 local IPC，进入明确 reconnecting 状态，持有同一个
   `ConnectionDemand` 触发已有 250 ms 到 10 s bounded-jitter 重连；CLI 进程已退出时不复活 view。
2. 首次默认 connect 可原子 create-and-attach `main`。首次成功后 bridge 固定稳定 SessionId；
   后续 transport 重连只能 attach 该 ID，不能在宿主 daemon 重启/Session 消失后悄悄创建一个
   同名替代 Session 或伪装旧任务仍存活。
3. 每次 remote stream 重建都会获得新的 remote AttachmentId。local view 身份保持稳定，bridge
   必须隔离/映射 remote attachment identity，不能让旧 stream 的 input、resize、ack 或 takeover
   命中新 attachment。
4. 断线和 snapshot 同步期间普通 input/paste 不排队、不稍后重放；CLI 继续 drain stdin并处理
   本地 detach。最新 viewport 可在新 snapshot 原子应用并 ack 后再发送。revision gap、queue
   overflow、model reset 或 baseline 不匹配一律请求/接受权威 full snapshot，不能猜测补齐。
5. normal network loss显示 reconnecting；revoked、unauthorized、protocol incompatible、SessionEnded、
   LeaseLost/taken over 使用不同终态。connection/path/direct/Relay 变化本身不结束 Session。

### R4. 公开命令面与安全交互

公开命令至少包括：

```text
zterm setup
zterm status [--json]
zterm doctor [--json]
zterm pair create [--ttl 10m]
zterm pair accept [--stdin] [--name <alias>]
zterm device list [--json]
zterm device rename <device> <alias>
zterm device revoke <device> [--yes]
zterm connect <device|local> [--session main] [--takeover]
zterm session list <device|local> [--json]
zterm session new <device|local> <name> [--cwd <host-path>]
zterm session attach <device|local> <session> [--takeover]
zterm session rename <device|local> <session> <new-name>
zterm session close <device|local> <session> [--yes]
zterm daemon status|stop|restart
zterm logs
zterm reset --identity [--yes] [--force]
```

1. `connect` 默认 attach `main`，不存在时原子创建；`session new` 成功后立即 attach 新 Session。
   `session attach` 与 `connect --session` 使用相同交互路径。
2. pair ticket 在最窄 owner 中 zeroize。`pair create` 只向用户 stdout 输出一次 ticket；
   `pair accept` 不接受位置参数/flag ticket。没有 `--stdin` 时非 TTY 输入直接失败且不读取。
3. device list 的 human/JSON 都清楚区分“本机可连接对端”和“对端可控制本机”，并显示安全的
   generation/online/stream/attachment 摘要，不输出 route cache、direct IP 或 secret。
   rename 只改 outbound alias；revoke 只撤销 inbound authorization。
4. close、revoke、daemon stop/restart 和 identity reset 显示精确目标与影响。交互模式确认；
   脚本必须显式 `--yes`/既有 `--force`。identity reset 在确认后停止 daemon/PTY，受管理地删除
   本机 identity/config/database 与配对状态，不发送 `RevokeSelf`、不自动 setup；下一次 setup
   生成新 EndpointId并要求重新配对。
5. 需要 daemon 的 pair/device/connect/session 命令在已有 setup 下使用 singleflight launcher
   按需启动一次；status/doctor/logs/daemon status/stop、help/version 和解析错误永不 autospawn。

### R5. 交互终端客户端

1. 交互 attach 要求 stdin/stdout 是 TTY，并在发起 attachment 前由 RAII `TerminalGuard`
   保存 termios、进入 raw mode。正常 detach、错误、Ctrl-C、SIGTERM/SIGHUP、task cancellation
   和 unwind 都恢复 termios、cursor、mouse/focus reporting、bracketed paste 与 alternate screen。
2. snapshot 只有在完整写入并 flush 到本地 TTY 后才发送精确 `SnapshotApplied`。delta 必须从
   当前 local revision 连续开始，否则请求 resync。daemon 只发送 TerminalModel 产生的受控 ANSI；
   CLI 不渲染原始未知 OSC/DCS/APC 或 terminal/input 内容到日志/错误。
3. snapshot 同步和 reconnecting 时继续 drain/丢弃普通 stdin；已同步时才转发 input。
   SIGWINCH/viewport 更新有界合并，不能形成逐 resize 无界队列。
4. `Ctrl+] .` 只 detach，不 close Session；`Ctrl+] Ctrl+]` 只发送一个 prefix byte。
   未识别/超时组合按配置处理，普通 Ctrl-C/Ctrl-Z 等在已同步状态下仍进入远端 PTY。

### R6. 资源、兼容与诊断

1. 保持已有 8 MiB frame、1 MiB control payload、per-connection/per-peer/global stream/task、
   Session/viewport/projection 和 operation replay 上限；任何新增 channel/watch/reconnect owner 都有界。
2. error code 继续只由 `DomainErrorKind` 投影。CLI 文案区分本地 daemon/setup、target/device、
   address/transport、authorization/revoke、wire/protocol、Session/lease/sync 和 outcome unknown。
3. wire v1 只做兼容字段/kind新增并保留 unknown capability bits；不改变 normal/pair ALPN、
   EndpointId、授权方向、SQLite schema 或 Session identity。
4. macOS 开发机不得执行会 bind Endpoint/UDP、DNS 或联网的测试；只运行纯状态、本地 Unix socket、
   PTY、CLI、compile/Clippy/`--no-run`。Linux CI 执行 real-Iroh loopback remote Session/多进程门禁。

## 验收标准

- [ ] 两个已配对 daemon 通过 production broker 完成 remote list/create/rename/close、默认
      `main` create-and-attach、命名 Session attach、input、resize、snapshot ack、detach 和 takeover；
      宿主使用唯一 SessionService，没有 remote registry。
- [ ] remote CLI 启动长期任务后强制断开 normal connection；本地 view显示 reconnecting且不转发/
      重放同步期输入，宿主 PTY继续。恢复后回到相同 SessionId、进程、cwd、screen 与近期历史。
- [ ] 强制产生 revision gap/慢 consumer/remote stream丢失后只通过 full snapshot恢复；ack前 input/
      resize 无副作用，snapshot/delta 无缺口、重复或错误 baseline。
- [ ] 同一控制设备同时连接两个不同 remote Session，broker observation只有一个 primary connection，
      attachment 使用独立 stream；拖慢/关闭一个不影响另一个或 control RPC。
- [ ] create/rename/close/takeover 在宿主提交后丢响应，客户端以完全相同 operation bytes重试并得到
      同一结果，没有重复 Session、重复 close 或双 controller；outcome unknown不换 lease重跑。
- [ ] revoke 与 remote list/attach/input/resize/takeover 的确定性竞态证明：旧 started commit先完成，
      writer之后的新提交全部失败；revoke返回后 connection/stream/attachment/lease失效但 Session/PTY继续。
- [ ] 另一台 CLI 远程创建/使用的 Session 可由宿主 `connect local` 接续相同 SessionId、进程、cwd、
      screen；local普通 attach不抢 remote controller，`--takeover` 才原子接管，反向亦然。
- [ ] public help含完整命令面且无 state/identity/socket override。setup前裸 `zterm`只提示 setup；
      setup后裸命令与 `connect local --session main` 等价；help/version/inspection/解析失败零副作用。
- [ ] pair create/accept/device list/rename/revoke 的 human/JSON与方向语义正确；ticket不进入 argv/env/log/
      errors/snapshots，默认 prompt不回显，piped input没有显式 `--stdin` 时拒绝。
- [ ] raw-mode、snapshot/delta renderer、SIGWINCH和默认/改键/禁用 prefix测试通过；正常、错误、panic、
      Ctrl-C、SIGTERM/SIGHUP均恢复本地 TTY，`Ctrl+] .`不关闭PTY，普通控制键仍进入Session。
- [ ] `reset --identity` 精确确认影响、拒绝无确认/有活动Session无force、停止后只删除受管理状态；
      重跑安全，后续 setup生成不同 EndpointId且旧配对不能控制新身份。
- [ ] macOS arm64/Intel、Linux x86_64/arm64 通过适用测试；Linux执行 real-Iroh loopback remote CLI门禁，
      Windows shared build/Clippy保持清洁且运行命令返回明确 unsupported。
- [ ] workspace fmt/check/Clippy `-D warnings`/tests/docs/deny、source/version/secret policy、Trellis
      validation与diff check全部通过；开发者Mac未执行任何Endpoint/UDP/DNS/network测试。

## 明确不在本任务范围

- Android/iOS App、Windows daemon/Named Pipe/ConPTY、桌面 GUI和CLI内嵌tab切换UI。
- observer/multi-view UI、多写者、per-session ACL、访客/分享链接、账号/中央撤销服务。
- `HISTORY_PAGING` RPC、无限/磁盘 transcript、alternate-screen应用内部完整历史。
- 任意create时启动命令、Codex/OpenCode/tmux/Herdr程序名特判或Agent专用状态/通知。
- PTY跨daemon崩溃、stop/restart、update或宿主重启存活；remote daemon重启后静默重建旧Session。
- M9 installer/release/update/uninstall（本任务只提供已规划的独立 `reset --identity` 命令）。
- M10双物理网络、NAT/path实验、official-n0新的公网运行证据与完整发布验收。
- 新Relay部署、public/self-hosted Relay acceptance workflow或改变已固定的official-n0 production profile。
