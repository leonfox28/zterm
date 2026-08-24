# M7-M8 实施计划：远程 Session 与公开 CLI

## 实施约束

- [x] 开始实现前运行 `trellis-before-dev`，重新注入 core/proto/platform/daemon/CLI 的适用
      spec；本规划通过审批后才运行 `task.py start`。
- [ ] 只实施父任务 M7-M8。不得顺带实现 mobile/GUI、observer、多写者、history paging、M9
      installer/update/uninstall 或 M10 双物理网络实验。
- [ ] 复用唯一 `SessionService`、`ConnectionBroker`、`AuthorizationRegistry`、StoreActor、frame
      codec、local daemon launcher 和 TerminalModel；不得建立 remote registry、第二套 replay、第二套
      terminal parser 或 CLI-owned Iroh Endpoint。
- [ ] 开发者 macOS 上不执行任何会创建 Iroh Endpoint、bind UDP、触发 DNS/公网 lookup 的测试。
      real-Iroh targets 只编译/列出；实际执行由 Linux CI 负责。
- [ ] 每个阶段先跑最小 focused gate，再跑比例化 package gate。发现契约偏差时先修正设计/spec，
      不以超时、sleep、忽略测试或放宽断言掩盖。
- [ ] 完成后由独立 `trellis-check` 审核；只有实现、focused/full gate 和 hosted CI 都有证据时才更新
      parent M7-M8 checklist、完成和归档任务。

## Step 0：基线、依赖图与安全边界冻结

- [x] 记录 HEAD、dirty worktree 与当前 hosted CI 状态，确认规划文件之外没有本任务未解释的改动。
- [x] 运行 source/version/secret policy、fmt、workspace check/Clippy/tests/docs/deny 的适用安全基线；
      真实网络 targets 在 macOS 只 `--no-run`。
- [x] 以测试锁定当前边界：local M4 unary/attachment 全绿；normal-ALPN service stream 仍返回精确
      `ServiceNotImplemented`；pair/device/revoke、broker single-primary 与 daemon local-only 行为不变。
- [x] 建立 kind/DTO/owner/调用图，逐项映射 PRD 的 list/create/rename/close/lease/attach/input/resize/
      ack/sync/detach/takeover；任何没有当前消费者的消息或 capability 不进入 v1。

Gate：现有 focused tests 与适用 package gates 全绿；若已有失败或 dirty source 无法归属，停止并先
说明，不能把基线失败混入 M7-M8。

## Step 1：core/proto 的兼容契约

- [x] 为 `DeviceId` 增加唯一 canonical 完整文本表示与严格解析：固定 64 位 lowercase hex；human
      输出可另做缩写，但 mutation target 永远使用 alias 精确命中或完整 ID。
- [ ] 复核并仅补齐 remote Session 所需 target、resume checkpoint、已知 revision、transport-state/
      terminal event 和 capability 字段；保留 wire major、normal/pair ALPN 和既有 numeric kind。
- [ ] 将 local desired-view 与每次 remote stream 的 AttachmentId 明确分层；不得让 wire peer提交
      local attachment identity。
- [ ] 为所有新字段完成 domain conversion、大小/枚举/zero/default 校验、unknown-field/capability
      compatibility、frame classification 与 redacted Debug；ticket、terminal bytes、cwd、route/direct
      address 不得出现在 Debug/error fixture。
- [ ] 保持既有 frame/control/viewport/projection bounds；resume checkpoint 只允许一个有界、零
      scrollback visible-grid baseline，不引入 per-revision ring 或磁盘 transcript。

Gate：

```sh
cargo test -p zterm-core --all-features
cargo test -p zterm-proto --all-features
cargo clippy -p zterm-core -p zterm-proto --all-targets --all-features -- -D warnings
cargo doc -p zterm-core -p zterm-proto --no-deps
```

Stop condition：若新增字段要求 breaking wire major、修改 SQLite schema 或复制 Session truth，停止并
回到设计审核，而不是扩大本任务。

本轮 schema audit 已确认 remote target、Session control、terminal sync/end/lease kinds 与 unknown
capability retention 均已存在；attach-time resume、transport-state、AttachmentId 映射和对应字段只在
Step 5 的生产 consumer 同步落地，不预建无消费者协议面。SessionId canonical text同样随公开 selector
consumer 实现。

## Step 2：抽取唯一 SessionWireServer

- [x] 从现有 local session unary/duplex adapter 抽取 crate-private `SessionWireServer`；它只依赖
      generic bounded AsyncRead/AsyncWrite、validated request context、SessionService 与 deadline，
      不知道 Unix path、Iroh Endpoint、alias、route 或 CLI。
- [x] local context保持 current same-UID principal、strict unary EOF、一个 decoder分类与同次 read
      leftovers；重构前后现有 local response bytes、typed error、final drain 和 detach 行为一致。
- [x] remote context固定 `{DeviceId, accepted generation}`，并验证 request target 精确等于 host
      identity。每个读取/提交 Session/PTY 的动作只在准确同步副作用窗口通过
      `AuthorizedCommitContext::run` 执行；网络读写/等待 revision 不持 permit。
- [x] unknown/malformed/oversize/stalled/trailing frame 只关闭对应 stream；adapter drop最多 detach
      attachment，绝不 close Session、signal PTY 或改变 controller。
- [x] 保留 daemon-issued operation lease、OperationId、exact-result replay 和 outcome-unknown 语义；
      local/remote adapter 不创建自己的 replay registry。

Gate：现有 `local_ipc`、`local_session_ipc`、Session lifecycle/controller/limits/recovery tests 全绿，
并增加纯 duplex shared-server parity tests；不运行任何网络。

Local-parity slice evidence：独立 checker 修复了 attachment 内 operation-lease 分配未进入 bounded
blocking worker 的一处偏差；随后 1 个 pure duplex unit、22 个 focused case、1 个 resync harness、
daemon all-target check/Clippy、fmt 与 diff check 全绿，未执行 Iroh/Endpoint/UDP/DNS 测试。

Remote slice evidence：独立 checker 将共享 decoder 的误导性 local 文案改为 transport-neutral，补齐
self/zero-generation regression，并把两个 biased accept loop 的 completed-task 回收移到新 accept 之前；
daemon lib 97 pass/3 Linux real-Iroh ignored，focused authorization/stream/lifecycle/rebind tests、daemon
all-target check/Clippy、docs、fmt、diff check 与全部 integration `--no-run` 全绿。未执行 Endpoint、UDP、
DNS、Relay、Internet 或真实网络测试。

## Step 3：broker 入站 remote service handler

- [x] 为 production `NetworkStartup`/`ConnectionBroker` 注入一个 retained、可 shutdown 的
      `RemoteServiceHandler`，沿用现有 normal-ALPN 首 frame admission、stream/task/per-peer/global
      bounds，不给 Session 层 Endpoint/Connection 所有权。
- [x] handler只接收 broker 已验证的 remote DeviceId、accepted authorization generation、
      `AuthenticatedBiStream` 与 absolute deadline，再调用 `SessionWireServer`；pair ALPN 永不进入。
- [x] 在入站 registration 与每个实际副作用点复核 generation；revoke writer fairness、generation
      rollover、stream cancellation 和 shutdown 后 admission 都 fail closed。
- [x] malformed/stalled/slow service stream、一个 Session handler panic或deadline不得关闭 primary
      connection、其他 Session stream、local IPC、Session或PTY。
- [x] lifecycle shutdown先停止新 service admission并有界回收 handler，再关闭 broker/Endpoint；
      fatal/retryable listener分支保持已有 truthful ownership。

Gate：socket-free handler/fake-stream/admission/revoke race tests、daemon all-target check/Clippy 全绿；
既有 real-Iroh integration在 macOS只编译。

## Step 4：出站 remote unary RPC 与 target resolver

- [x] 在 local daemon内实现 target resolver：保留字 `local` 走同一 local service；其他 selector由
      同一 StoreActor/DeviceDirectory 解析精确 alias或完整 canonical DeviceId，并按 directional
      authorization检查是否可 outbound connect。
- [x] `RemoteUnaryClient` 每次 operation持有 `ConnectionDemand`，通过 broker `open_bi(Service)`
      发送严格一个 frame并读取严格一个 response；CLI从不获得 Endpoint、key、route或socket path。
- [x] read-only ambiguity可在同一 absolute deadline安全重试；mutation仅复用完全相同的 encoded
      request、OperationId和daemon-issued lease最多重试一次。operation-lease allocation是stateful
      control，post-write failure只发送一次并保留typed transport/protocol failure。typed response/
      outcome unknown为终态，不自动换lease重新执行。
- [x] 扩展 local IPC request的target-aware control路径，并保持旧local-only调用兼容；慢remote RPC
      不占用local listener、StoreActor或其他Session的全局锁。
- [x] device/session selector的not found、ambiguous、direction denied、transport、authorization、
      protocol和Session domain error保持可区分且不泄漏peer授权状态。

Gate：pure fake-broker byte-identical retry、selector/direction、response-loss/typed terminal tests，以及
local IPC existing/focused tests全绿。

Step 4 final-check evidence：daemon pure units 119 pass/3 Linux real-Iroh ignored；core/proto all-feature
tests、Unix `local_ipc` 4 cases与frozen-target persistence regression全绿。`RemoteUnaryClient`测试证明一个
`ConnectionDemand`下最多两次byte-identical service-stream send，且target/request/lease/OperationId不变；
outer Unix mutation regressions对malformed/truncated、wrong kind/ID和invalid typed payload均记录exactly
one send，stateful lease的outer Unix envelope与daemon remote service stream均只发送一次并保留typed
failure，read-only outer仍记录一次byte-identical retry。workspace all-target check、Clippy
`-D warnings`、docs、fmt、source/version/secret policy、task validation与diff check全绿；全部daemon integration（含
real-Iroh targets）仅以`--no-run`编译，未执行Endpoint、UDP、DNS、Relay、Internet或真实网络测试。

## Step 5：可重连 RemoteAttachmentBridge

- [x] 每个活跃local CLI view创建一个daemon-owned bridge；其整个desired-view生命期持有同一
      `ConnectionDemand`，但每次reconnect创建新service stream与remote AttachmentId。
- [x] 首次 `connect` 可原子确保默认 `main` 存在并attach；首次成功后锁定SessionId。重连只attach
      此ID，host daemon restart/SessionEnded时绝不静默创建同名替代。
- [x] 显式实现 `Preparing -> Synchronizing -> Active -> Reconnecting -> Terminal/Detached` 状态机；
      旧stream epoch的input/resize/ack/takeover/event全部作废，local view ID始终稳定。
- [x] disconnect时local IPC继续存在并显示bounded reconnect状态；普通stdin继续drain但丢弃，绝不
      queue/replay。只coalesce一个latest viewport，待完整snapshot写入/flush/ack后发送。
- [x] 使用一个有界resume checkpoint请求连续delta；baseline/gap/overflow/reset不匹配则接受权威
      full snapshot。只有local CLI仍活着时重连；drop view立即释放demand、tasks、channels与permit。
- [x] 区分temporary network loss与revoked/unauthorized/protocol incompatible/SessionEnded/LeaseLost；
      terminal状态不无限重试，detach不close Session。

Gate：纯fake-clock/fake-stream状态机tests覆盖ID隔离、backoff/cancel、sync input drop、resize coalesce、
gap/resync、revoke/lease/session终态、两view共享一个task-private target owner；真实single-primary由既有
broker gate与Step 9 Linux real-Iroh gate负责；local Unix duplex tests全绿。

Step 5 final-check evidence：15个`remote_attachment` pure/fake-stream cases证明one-demand/fresh-epoch ID、
freeze SessionId、状态顺序、同步期input drop、latest viewport、marker+same-attachment snapshot fallback、
post-Active half-open `SessionOccupied` 的250 ms paused-time retry与first-ever occupied终态、bounded
demand/open/write/read、correlated lease/takeover completion、terminal epoch pending-control drain、
ordinary/fatal ServiceError分类及typed local fatal error flush；`session_wire` authenticated pure duplex/PTY
regression证明只有authenticated clean EOF移动一个exact visible checkpoint，transport I/O、explicit
detach与protocol failure均丢弃并在下次返回full snapshot。host resume/overlapping-live-view unit、bounded
server attachment write/flush/control queue、local target router/transport-state event、`local_ipc` 4 cases、
`local_session_ipc` 7 cases与`terminal_recovery` 1 case全绿；此前独立full gate为daemon lib 139 pass/3
Linux real-Iroh ignored，本follow-up另有`remote_attachment` 15/15、daemon lib check/Clippy全绿；
core/proto all-feature tests、workspace all-target check/Clippy `-D warnings`、docs、fmt、
source/version/relay/secret policy、offline deny、task validation与diff check全绿。全部daemon integration
（含real-Iroh targets）仅`--no-run`编译；macOS未执行Endpoint、UDP bind、DNS、Relay、Internet或真实网络测试。

## Step 6：公开 pair/device/session 命令后端

- [x] 扩展现有 `LocalRuntime` 为高层、typed、可测试接口；公开CLI仍只调用same-UID local IPC和
      singleflight daemon launcher，不暴露`LocalClient`、UserPaths或secret owner。
- [x] 公开 PRD 中的 pair/device/connect/session/daemon/reset 命令，并锁定：裸 `zterm`等价
      `connect local --session main`；`zterm --help`始终显示帮助；setup前裸调用只提示setup。
- [x] `pair accept`默认从no-echo TTY prompt读取，自动化仅显式`--stdin`；禁止ticket位置参数/flag/
      env，所有短期String/bytes最窄zeroize，stdout只在create成功时输出一次ticket。
- [x] human/JSON device输出清楚区分outbound-known与inbound-authorized，隐藏route/direct address；
      rename只改outbound alias，revoke只改inbound authorization并要求交互确认或`--yes`。
- [x] session命令统一解析exact ID/name；new后attach、connect默认main、普通attach不抢controller，
      `--takeover`才原子接管。close显示精确目标并要求确认。
- [x] 实现`reset --identity`的独立安全边界：预检影响、活动Session需`--force`、有界stop、取得
      lifecycle lock、exact managed UserPaths/no-follow校验、只删受管state、不发送RevokeSelf、不在
      同一命令setup；失败可安全重跑。
- [x] help/version/parse error/status/doctor/logs/daemon status/stop不autospawn；其余需要daemon的命令
      仅在已setup后按需singleflight启动。

Gate：CLI command-side-effects、help/snapshot、pairing secrecy、device direction、confirmation、bare
entrypoint、local self-attach、identity-reset fault-injection tests全绿；生产argv无state/identity/socket
override。

Step 6 final-check evidence：公开clap只消费高层`LocalRuntime`，生产argv无state/identity/socket/ticket
override；bare与`connect local --session main`在local-only daemon harness中复用同一SessionId，普通attach
不抢controller，create→attach与close/revoke confirmation均冻结exact ID。pair ticket默认no-echo TTY、
自动化只允许显式`--stdin`，短期输入zeroize，Debug/error不回显，stdout exact write+flush，且success/error/
panic均恢复echo。identity reset使用一个absolute deadline覆盖stop/readiness/socket/daemon.lock，取得
lifecycle lock后再复核；fixed inventory、unknown/type/owner/mode/symlink拒绝前零unlink、lifecycle lock最后
删除、注入partial deletion后缺config/identity仍可重试均有直接测试。invalid cwd错误已改为typed path-free
诊断。独立checker的完整CLI package、command-side-effects、local-only autospawn/self-attach、daemon
operations/session cwd、platform removal tests，CLI+daemon all-target check/Clippy `-D warnings`、fmt与diff
check全绿；real-Iroh targets仅`--no-run`编译，未执行Endpoint、UDP、DNS、Relay、Internet或真实网络。
Windows本机cross-check在进入项目源码前被缺MSVC SDK headers/`ml64.exe`阻断，保留hosted Windows验收。

## Step 7：raw terminal UI、renderer与信号恢复

- [x] Unix上实现RAII `TerminalGuard`：attach前验证stdin/stdout TTY并保存termios，进入raw mode；
      drop恢复termios、cursor、alternate screen、mouse/focus和bracketed-paste状态。
- [x] 统一snapshot/delta renderer：完整snapshot写入并flush后才发送精确`SnapshotApplied`；delta
      revision不连续立即resync。只渲染TerminalModel生成的受控ANSI，错误/日志不含终端内容。
- [x] 实现固定容量stdin reader/channel与纯byte prefix parser：默认`Ctrl+] .` detach，
      `Ctrl+] Ctrl+]`发送一个raw prefix；改键和disabled通过本次调用配置，不扩大持久config schema。
- [x] Synchronizing/Reconnecting继续drain并丢弃普通输入；Active才发送。SIGWINCH只保留latest
      viewport；同步完成后发送一次。
- [x] SIGINT/SIGTERM/SIGHUP、task cancellation、server EOF、typed error和unwind都走同一幂等恢复；
      恢复TTY后再以normal mode打印安全诊断。Windows shared CLI保持无dead-code并明确unsupported。

Gate：`raw_mode_restore`、`control_prefix`、renderer/snapshot/delta、signal/cancel/panic、resize coalesce、
local self-attach与bare entrypoint tests全绿；使用task-private PTY，不接触Iroh。

Step 7 final-check evidence：Unix raw UI在任何attach副作用前验证stdin/stdout TTY；`TerminalGuard`保存
exact termios，raw模式与zterm-owned alternate screen成对，成功/错误/task abort/panic及隔离进程中的
SIGINT/SIGTERM/SIGHUP均先恢复mouse/focus/paste/application cursor+keypad、cursor/screen与termios，再在
normal mode输出固定诊断。termios读写重试`EINTR`，只有display与attributes同时恢复后才封口，失败的
显式恢复仍由Drop重试且恢复错误优先。renderer只消费typed TerminalModel ANSI，snapshot完整write+flush
后才ack，revision gap请求full sync，嵌套screen selector被拒绝。stdin使用CLOEXEC fd、固定channel与纯byte
prefix；每次进入Active先discard receiver、wake/join旧reader，再flush、换epoch并启动新reader，两个
无sleep PTY barrier循环直接覆盖`poll`到blocking `read`窗口且只交付fence后输入。create与
`create_main`一旦可能提交，prefix/EOF/signal只记录取消并继续取得exact SessionId、
`CreatedSessionAttach`或`OperationOutcomeUnknown`；首个snapshot的总deadline覆盖任意pre-snapshot state，
paused-time watchdog证明stalled state不会绕过边界。最终CLI 28 pass/3隔离helper ignored，daemon相关
create/deadline/state focused 5 pass；CLI+daemon all-target/all-feature check、Clippy `-D warnings`、
compile-only `--no-run`、fmt与diff check全绿。未执行Endpoint、UDP、DNS、Relay、Internet或真实网络测试。

## Step 8：确定性并发、安全与资源回归

- [x] 用barrier而非sleep证明revoke与list/attach/input/resize/takeover顺序：已started同步commit可完成，
      writer之后的新提交全部拒绝；revoke返回后旧generation、stream、attachment与lease失效，Session/
      PTY继续。
- [x] 注入create/rename/close/takeover提交后丢响应，验证byte-identical重试与exact replay；typed
      OutcomeUnknown不换lease、无重复Session/close/controller。
- [x] 两个attachment与control RPC共享同一remote primary，独立stream无head-of-line；一个慢/恶意
      stream只消耗自身bounded资源。
- [x] 覆盖Session/stream/task/frame/channel/viewport/projection/replay上限、deadline/cancel/drop/panic/
      poison、revision overflow与shutdown；所有owner均可观察回收，无无限reconnect或input queue。
- [x] secret/log snapshot扫描覆盖ticket/proof/key/terminal/cwd/direct IP/route；public JSON保持稳定安全。
- [x] 宿主local attach可接续remote创建的同一SessionId/进程/cwd/screen；普通attach不抢lease，
      双向`--takeover`保持原子。

Gate：新增remote-session/reconnect/revoke/stream-isolation/resource targets与全部既有M3-M6安全测试全绿；
macOS不执行任何real Endpoint target。

Step 8 final-check evidence：真实`AuthorizedCommitContext`/StoreActor/SessionService revoke matrix与
writer-first-poll barrier证明durable→registry→close→detach顺序；real Session wire的
create/rename/close与takeover response-loss保持byte-identical重试与exact replay。两个production
attachment bridge与一个unary client共享一个task-private target owner，容量1 duplex的真实
`poll_write -> Pending`不阻塞第二个Session wire stream或list；真single-primary所有权由既有
broker gates与Step 9 Linux real-Iroh gate证明，此pure fake不冒充broker证据。真实PTY用例
证明remote→local→remote双向接管保留同一SessionId/进程/cwd/screen，普通反向attach
也不抢controller，另一principal/Session持续可用。两个exact gate分别连续20/20与
10/10通过；`remote_session` 18/18、`remote_attachment` 18/18、`controller_lease` 4/4，
broker RAII/panic/malformed/oversized/stalled、stream/resource/revoke gates与daemon all-feature
check/Clippy、fmt、source-policy、secret-scan、task validate、diff check全绿。未执行Endpoint、UDP、
DNS、Relay、Internet或任何真实网络路径。

## Step 9：Linux real-Iroh transport 门禁与 M10 移交

- [x] 保留唯一较小的Linux-only `two_daemon_transport` fixture；两个owner隔离identity/store/auth，
      复用一个Endpoint承载pair与normal ALPN，且不新增生产state/identity/socket override argv。
- [x] real target在macOS标记ignore且fixture在任何bind前fail closed；开发者Mac只运行`--no-run`/
      `--list`，Linux hosted runner实际执行该target。
- [x] 记录精确commit、test名称与run URL，并限制证据范围为transport owner/授权/route persistence。
- [x] public CLI/remote Session 的正式OS多进程验收移交M10，由M9签名Release的installed binaries执行；
      不再建立第二套仅编译或task-private的daemon-like Session harness。

Gate：Linux保留的real-Iroh loopback transport test实际通过；该loopback证据不冒充remote Session、
official-n0公网、self-hosted relay或M10双NAT/双物理网络发现证据。

Step 9 cleanup evidence（2026-08-24）：删除了没有hosted Linux job或真实run URL的
remote Session daemon-like lib-test harness及其专用response-loss注入。保留唯一较小的
`two_daemon_transport` fixture，持久配置为`OfficialN0`、运行时为relay-disabled IPv4 loopback和
task-only direct route；它只覆盖pair/normal ALPN、Endpoint/primary复用、方向授权与direct-route不落盘，
不冒充remote Session、official-n0 Relay、公网或public CLI多进程证据。remote Session的重试、
response-loss、reconnect、no-HOL、revoke与cleanup继续由pure/Unix IPC测试覆盖。未来若补真实remote
Session或pairing acceptance，必须与hosted Linux job及run URL一起建立，并复用这一fixture，不能再
维护第二套仅编译的daemon-like owner。

Hosted evidence（2026-08-24）：commit `d3cfc5697c4b6a5dcd10f3bf70689e29b3c797f8` 的
[GitHub Actions run 32725142928](https://github.com/leonfox28/zterm/actions/runs/32725142928)
全部成功。Linux x86_64 job 实际运行
`two_daemon_owners_reuse_endpoint_for_pair_and_normal_confirmation` 并通过；Linux arm64、macOS arm64/
Intel、Windows shared/unsupported、dependency policy 与 official Relay bundle jobs 同时通过。该
run 没有执行已删除的remote Session harness，因此不能被引用为public CLI/remote Session证据。

## Step 10：文档、spec与最终门禁

- [x] 更新backend specs与用户文档，准确说明command、local/remote target、default main、reconnect、
      takeover、directional trust、ticket input、identity reset和daemon-lifetime Session边界。
- [x] 文档保持official n0为production默认，不新增public/self-hosted Relay acceptance workflow；
      M10 discovery/NAT/path与M9发行事项继续未完成。
- [x] 更新parent M7-M8 checklist只勾选有直接实现与测试证据的条目，记录Linux run与Windows hosted
      shared compile证据；不把macOS compile-only写成network runtime通过。
- [x] 运行独立`trellis-check`，修复spec drift、cross-layer重复、cfg dead-code、secret surface与
      flaky/非确定性测试后重跑完整门禁。

Step 10 local documentation evidence（2026-08-24）：新增`docs/remote-cli.md`，并同步`README.md`、
`docs/core-local-daemon.md`、`docs/persistent-sessions.md`与`docs/development.md`；backend executable
contracts只更新`local-daemon-ipc.md`、`session-service.md`和`transport-auth.md`，分别固化public
CLI/target/secret ownership、local/remote同一controller规则与Step 9 hermetic evidence边界。逐项核对
root及全部subcommand clap help后，唯一具体mismatch是`daemon stop/restart --force`仍写“future
milestones”；已改为会结束当前active Sessions/PTYS，并由`command_side_effects`子进程help断言固定。
文档只称`OfficialN0`为production默认；Step 9 exact target明确只使用`RelayMode::Disabled`、IPv4
loopback和task-only direct route，不能充当official-n0 Relay、公网、自建Relay、DNS/Pkarr、M10或
public CLI多进程证据。

父任务仅勾选现有pure/Unix/PTY及source gate直接证明的M7-M8条目，并修正Ctrl+] prefix契约；已记录
Linux retained transport runtime、exact run URL与hosted Windows shared compile。public CLI/remote
Session OS多进程、M9发行与M10 network lab仍未勾选并明确移交，不把transport test扩大解释。

独立final checker（2026-08-24）修复两项跨层问题：peer-authored `ServiceError.message` 不再进入本地
`DaemonError`或attachment frame，只保留correlated request ID/kind、zeroize原文本并重编码稳定
content-free detail；多进程PTY gate不再把resize同步周期与input/detach串联，而由connect child证明
eventual interactive echo→默认detach，bare child独立证明SIGWINCH viewport/revision、
SIGTERM cancellation与termios恢复。PTY gate串行20/20、8路×5共40/40，精确进程检查零orphan。
完整workspace tests、all-target check/Clippy `-D warnings`、docs、offline cargo-deny、fmt、source-policy、
workspace-version、Relay static、secret scan、task validate（27 implement + 27 check，无截断warning）与
diff check全绿。真实Iroh target在macOS保持ignored/compile-only；未执行Endpoint、UDP bind、DNS、Relay、
Internet、TCP listener或其他网络路径。

用户批准的行为不变清理（2026-08-24）删除两套没有Linux run URL的daemon-like real-Iroh harness、
其专用response-discard seam、CLI导出的Active marker及renderer分支、重复`revoke_races` target与一条
重复writer unit。保留`two_daemon_transport`唯一loopback fixture，以及`authorization`、
`local_device_ipc`和`session_wire`唯一durable revoke矩阵；service调度observer仅在test构建存在。
简化后的真实`run_terminal` PTY gate连续10/10，本轮focused daemon/CLI gate全绿。此清理不改
reconnect resume、identity reset或任何产品路径；完整最终gate结果以本段之后的最新执行为准。

清理后最终本机安全gate（2026-08-24）：workspace all-target/all-feature check与Clippy `-D warnings`、
workspace tests（daemon lib 172 pass、CLI lib 31 pass/3 isolated ignored及全部integration/doc tests）、
workspace docs、offline cargo-deny、fmt、source-policy、workspace-version、Relay static、secret scan、
task validate（27 implement + 27 check）与diff check全部通过；`cargo deny`仅报告既有允许的duplicate
dependency warnings。简化后的PTY multiprocess gate另连续10/10。macOS上的real-Iroh
`two_daemon_transport` case保持ignored；本轮没有执行Endpoint、UDP bind、DNS、Relay、Internet或其他
网络路径。

Final gate：

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
sh tests/relay/static.sh
sh tests/secret-scan.sh
python3 .trellis/scripts/task.py validate .trellis/tasks/08-23-remote-cli
git diff --check
```

在开发者macOS执行完整gate时，任何包含真实Endpoint的target必须以项目现有ignore/fixture策略跳过；
Linux CI另行实际执行Step 9 exact targets。hosted Windows负责shared core/proto/daemon/CLI compile与
unsupported tests，本机缺MSVC SDK的cross-check不能替代或阻塞该证据。

## 完成条件

- [x] M7-M8实现范围都有直接、可复现证据；正式安装后的public CLI/remote Session验收明确移交M10，
      没有借用M9/M10或未执行的公网测试冒充当前证据。
- [x] 唯一SessionService/ConnectionBroker/AuthorizationRegistry与secret ownership未被复制或绕过。
- [x] 独立checker、完整本机安全gate和hosted matrix全绿；Linux retained real-Iroh transport gate实际
      执行通过，证据范围已准确记录。
- [x] parent M7-M8已同步实现状态与发布验收边界；child已记录commit、CI run与剩余M9-M10 owner，
      可以在本次规划/evidence commit后归档。

## 执行复盘与继续条件

本次超过合理时长的原因、外部provider计费边界、代理数量限制、验证预算与停止条件已持久化到
[execution-retrospective.md](./research/execution-retrospective.md)。任何压缩后继续本任务的会话必须先读该
文件；用户批准的唯一行为不变清理已完成，本机不再新增实现、审计、测试矩阵或网络harness，下一步
仅允许按既定分组commit并执行finish-work。长期可复用规则位于
`.trellis/spec/guides/evidence-driven-simplicity.md`，且已在本任务implement/check manifest中注入。
