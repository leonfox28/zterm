# M7-M8 技术设计：远程 Session 与公开 CLI

## 1. 边界与原则

本任务连接两个已经完成的子系统，而不改变它们的真相所有权：

- `SessionService`仍唯一拥有Session、PTY、TerminalModel、revision、attachment、controller、
  resource reservation和operation replay；
- `ConnectionBroker`仍唯一拥有每remote primary Iroh connection、dial/reconnect、candidate、route、
  stream admission和transport observation；
- `AuthorizationRegistry`/StoreActor仍唯一拥有入站授权generation和durable revoke顺序；
- local daemon仍唯一拥有identity.key、Endpoint与remote connection pool；
- CLI只拥有命令解析、用户确认、raw TTY、local renderer和一个same-UID IPC view。

任何adapter drop最多结束一个attachment或stream，不能close Session或signal PTY。

## 2. 组件与数据流

```text
                           host daemon
                     +---------------------+
remote normal stream | RemoteServiceHandler|
-------------------->|                     |
                     |  SessionWireServer  |----> SessionService/SessionActor
local Unix stream    |                     |             | PTY + TerminalModel
-------------------->| LocalSessionAdapter |
                     +---------------------+

controller machine

zterm CLI
  | same-UID Unix IPC
  v
local daemon
  |- local target --------------> local SessionWireServer
  `- remote target
       |- RemoteUnaryClient ----> ConnectionDemand/open_bi(Service)
       `- RemoteAttachmentBridge -> one desired Session attachment
                                      across broker reconnects
```

`SessionWireServer`是新的crate-private共享边界，建议放在
`crates/daemon/src/session_wire.rs`。它不知道Unix socket path、device alias、route、CLI或
Endpoint；它只知道已验证的request context、一个generic AsyncRead/AsyncWrite stream、
`SessionService`和remote模式下的`AuthorizationRegistry`。

## 3. Shared SessionWireServer

### 3.1 Request context

```rust
enum SessionRequestContext {
    LocalSameUid {
        own_device_id: DeviceId,
        local_view_id: AttachmentId,
    },
    RemoteAuthenticated {
        own_device_id: DeviceId,
        remote_device_id: DeviceId,
        accepted_generation: AuthGeneration,
        authorization: AuthorizationRegistry,
    },
}
```

context生成稳定`AttachmentPrincipal`。local request要求`target.local=true`；remote request要求
`target.device == own_device_id`。target只用于routing/confused-deputy检查，不构成authorization。

remote context通过统一helper运行每个服务动作：

```text
authorization.acquire_commit(remote, accepted_generation)
  -> AuthorizedCommitContext::run(|| SessionService effect)
```

list、lease分配、create/rename/close、prepare_attach、snapshot ack、sync、input、resize、takeover
都走该helper。socket/QUIC写和等待revision不持authorization permit；revoke通过broker cancel和
`detach_remote_principal`结束attachment。这样permit只覆盖准确副作用窗口，不让慢网络阻塞writer，
又不会出现“stream开始时授权、实际input时已经revoke”的TOCTOU。

### 3.2 Stream分类与framing

一个generic first-frame reader继续使用唯一`zterm_proto::FrameDecoder`并返回decoder leftovers：

- unary kinds：list、operation lease、create、rename、close；要求一个frame后EOF，响应后finish；
- terminal kind：`TerminalAttachRequest`；保留同一read中已解出的后续frame，进入duplex loop；
- attachment内只允许snapshot ack、sync、input、resize、detach、operation lease和takeover；
- 其他kind/major/frame/payload错误写一个typed error（若仍可写）并只结束该stream。

local listener继续在peer-UID验证后调用它；broker只在Iroh TLS和normal Hello/Welcome认证后调用它。
local lifecycle/pair/device kinds仍由`DaemonService`原dispatcher处理。

### 3.3 Terminal server loop

现有local attachment reader/writer逻辑抽取为generic halves：

- reader验证绑定的SessionId/AttachmentId和context后才调用SessionAttachment；
- writer使用fixed-capacity control queue与latest-onlyrevision/lifecycle watch；
- final update仍必须先于`SessionEnded`；revision watch提前关闭时等待lifecycle authority；
- Drop guard调用`attachment.detach()`；network/EOF/error没有PTY close authority；
- remote explicit detach删除resume checkpoint；transport EOF可留下一个有界resume checkpoint。

本次重构先用全部现有`local_session_ipc`、`terminal_recovery`、session concurrency测试证明
local行为字节/语义不回归，再启用remote handler。

## 4. Broker入站service handler

新增与`PairConnectionHandler`同形但专用于normal service stream的一次性callback，例如：

```rust
trait RemoteServiceHandler: Send + Sync + 'static {
    fn handle_service_stream(
        &self,
        stream: InboundAuthenticatedStream,
        deadline: Instant,
    ) -> RemoteServiceHandlerFuture;
}
```

`InboundAuthenticatedStream`只暴露owned SendStream/RecvStream、remote DeviceId和本机接收时确认的
authorization generation；不暴露Endpoint、PeerSlot、route或profile。broker仍在调用前拥有：

1. inbound authorization pre-Hello检查；
2. candidate注册后的generation recheck；
3. promoted connection与per-connection/global handler permits；
4. first-frame deadline/connection cancel/metrics；
5. revoke、duplicate、shutdown时abort并回收handler。

handler通过`NetworkStartup::with_service_handler`在bind前安装。composition只捕获
`SessionWireServer`（SessionService + AuthorizationRegistry + own DeviceId），不捕获
`DaemonService`或broker，因此没有强引用环。未安装时保持当前typed
`service_not_implemented`行为，方便isolated tests。

## 5. 出站RemoteUnaryClient

local unary frame在strict EOF后按`TargetSelector`分流：

- `local`：直接交给SessionWireServer local context；
- `device`：确认不是own identity并让broker从durable known-device row取得
  `ConnectionDemand`，打开`StreamPurpose::Service`，发送原始frame envelope，finish write，严格读取
  一个response与EOF。

remote request不decode/re-encode其payload作为重试源。adapter从`DecodedFrame`保留原始kind、
request_id、deadline和payload，一次构造完整bytes；read-only与operation-ID mutation在ambiguous
transport failure时都可在同一absolute deadline打开新stream再发送完全相同bytes一次。remote
expected typed response原样投影到local response；完整且correlated的已知`ServiceError`只保留
`DomainErrorKind`与request correlation，丢弃不可信peer message并以稳定content-free detail重编码。
完整`operation_outcome_unknown`不再retry。
`SessionOperationLeaseRequest`单独归类为stateful control：一旦写出后结果不明，返回原typed
transport/protocol failure，不打开第二条remote stream，也不投影为mutation outcome unknown。

重试owner只允许一层：daemon `RemoteUnaryClient`持有一个`ConnectionDemand`；SessionList与逻辑
mutation在同一deadline内最多发送两次完全相同的Iroh frame，lease allocation只发送一次。
`LocalClient`把remote mutation的same-UID outer envelope写出任何
字节后，首个缺EOF、malformed、wrong ID/kind或invalid typed payload立即投影为
`operation_outcome_unknown`，不得重连local socket或重放outer envelope；只有read-only outer request
保留一次byte-identical retry。完整且correlated的已知`ServiceError`与通过共享validator的expected
typed response都是终态。这样端到端最多是1 outer × 2 inner，而不是2 × 2。

CLI侧逻辑`LocalClient`继续lazy缓存目标宿主签发的operation lease。lease cache必须按target
DeviceId分区；target变化、typed outcome unknown或daemon incarnation变化只清理对应target cache，
不能让local lease用于remote或跨remote复用。

## 6. RemoteAttachmentBridge与resume

### 6.1 状态机

```text
LocalViewCreated
  -> Connecting
  -> AwaitingRemoteSnapshot
  -> AwaitingLocalSnapshotApplied
  -> Active
       | transport loss
       v
     Reconnecting --bounded broker backoff--> AwaitingRemoteSnapshot/Delta
       | revoked/incompatible/session missing/local EOF
       v
     Terminal
```

一个bridge从local attach首frame开始持有：

- stable local view/attachment ID；
- target DeviceId、稳定Session selector和首次成功后的SessionId；
-一个`ConnectionDemand`；
-当前remote stream与remote AttachmentId；
-最后local已应用Revision、当前viewport和一个random resume view ID；
-fixed-capacity control channel/latest-only viewport；
-cancellation与一个absolute per-attempt deadline，整体生命周期由local socket决定。

初次`connect main`可以`create_main=true`。取得首个snapshot后selector冻结为SessionId；重连只发送
该ID。若host daemon重启或Session结束，`session_not_found`/`SessionEnded`是终态，不能创建同名替代。

### 6.2 AttachmentId隔离

每条remote stream使用host生成的新AttachmentId。bridge不把它当作local identity：

- server→local的snapshot/sync/lease-lost/ended中的remote ID改写为stable local ID；
- local→server的ack/input/resize/detach/takeover先验证stable local ID，再改写为当前remote ID；
- stream epoch与remote ID一起检查；旧epoch frame即使晚到也被丢弃，不能命中新controller；
- operation lease属于remote host daemon并在bridge内按remote target维护。

### 6.3 Revision resume

`TerminalAttachRequest`以兼容新增字段携带random `resume_view_id`和optional
`known_revision`。SessionActor最多保留当前/最近remote controller的一个zero-scrollback
resume checkpoint，key绑定remote principal generation、SessionId和resume view ID：

- transport EOF保存checkpoint并立即释放controller lease；
-显式detach、Session end、takeover by another principal、generation变化或容量替换删除它；
-重连known revision与checkpoint精确相等时可生成一个merged delta；不相等/已淘汰/resize/model reset
  直接full snapshot；
-checkpoint不是per-revision queue，不复制2,000行host history，不写磁盘，也不扩展observer能力。

这保留M4“latest-only、无逐revision backlog”的不变量，同时满足短断线delta恢复；任何不确定性都
回退到权威snapshot。

### 6.4 Reconnect期间输入

新增local-only attachment transport-state消息（Connecting/Reconnecting/Synchronizing/Active）供CLI
显示，不在remote协议中冒充Session事件。bridge在非Active状态仍读取local IPC以避免内核缓冲：

- input/paste立即丢弃，不入队；
- detach立即取消bridge；
- resize只覆盖latest viewport；
-snapshot ack只接受当前stream epoch的精确revision；
-takeover必须在当前snapshot已ack后提交。

## 7. CLI runtime与target解析

`LocalRuntime`新增高层入口，不公开`UserPaths`、socket或daemon secret：

```text
ensure_configured_daemon()
pair_create / pair_accept
device_list / rename / revoke
resolve_target(alias | canonical full DeviceId | local)
session_list / create / rename / close
attach(target, selector, create_main, takeover, viewport)
reset_identity(...)
```

需要daemon的命令先执行“已setup验证→singleflight ensure→IPC”；没有setup返回`not_setup`和固定
`zterm setup`提示。inspection/help/version/parse error不调用ensure。

DeviceId提供一个core-owned canonical lowercase 64-hex文本adapter和严格full-length parser。
CLI显示full ID（可另显示安全短suffix），mutation不接受short prefix。alias精确、大小写敏感；
只有`outbound_known=true`的设备可作为remote connect target。inbound-only row仍可用full ID revoke。

Session selector优先解析明确full 32-hex SessionId，否则使用`SessionName`。名称与ID歧义通过完整长度
和domain validator确定，不接受模糊prefix。

## 8. 公开CLI与secret边界

clap命令按PRD固定。pair TTL parser只接受checked `s`/`m`/`h`并在core
`MIN_PAIR_TTL_SECONDS..=MAX_PAIR_TTL_SECONDS`内；默认0让daemon选择10m。

`pair accept`：

-默认要求stdin TTY，用termios guard临时关闭echo，bounded读取一行，恢复echo后才继续；
-`--stdin`读取bounded输入并要求ticket后只有whitespace/EOF；没有flag的pipe不读取就失败；
-ticket立即进入`PairTicketText`/`Zeroizing` owner；错误只含typed类别；
-不存在ticket位置参数、`--ticket`、env或state-path测试override。

device/session list的human/JSON来自同一typed view。device方向使用明确列/字段：
`outbound_known`, `inbound_status`, `generation`, `online`, `streams`, `attachments`。revoke确认文案明确
只撤销“对端控制本机”，不会删除“本机连接对端”记录或结束其Session。

破坏性操作统一confirmation helper。interactive显示exact full target/name/SessionId和影响；
noninteractive缺`--yes`失败。daemon stop/restart保留既有`--force`语义。`session new`先执行mutation，
再按returned SessionId attach；attach失败不rollback已经成功创建的Session，而是如实显示SessionId。

`connect`与`session attach`支持`--escape ctrl-]`（默认）、其他单一ASCII control byte或`none`；
这是一条per-invocation配置，不修改config schema v1。

## 9. TerminalGuard、renderer与input

### 9.1 TerminalGuard

Unix `TerminalGuard`使用`nix::sys::termios`保存stdin termios并`cfmakeraw`，保存是否进入本地
alternate screen。进入attachment前创建；Drop按best-effort顺序：

1. disable mouse/focus reporting与bracketed paste；
2. reset SGR、show cursor、leave zterm-owned alternate screen；
3. flush stdout；
4. restore exact saved termios。

Tokio启用`signal` feature并select可捕获的SIGINT/SIGTERM/SIGHUP；这些路径先取消attachment，再drop
guard。raw mode关闭ISIG，所以键盘Ctrl-C/Ctrl-Z作为bytes发送remote PTY；外部signal仍终止本地view。
panic unwind也由Drop恢复。SIGKILL/abort不可捕获，文档不作虚假保证。

### 9.2 Renderer

renderer只接收proto validated TerminalModel output：

-full snapshot先原子写recent history + screen state并flush，再更新local revision并ack；
-delta要求`from_revision == local_revision`，写完flush后更新到`to_revision`；否则request sync；
-SyncRequired进入synchronizing并禁止input，下一snapshot替换状态；
-Reconnecting写简短本地状态，下一full snapshot覆盖；
-LeaseLost/SessionEnded/typed terminal error恢复TTY后在normal mode打印原因。

### 9.3 Input/prefix/resize

stdin reader与server event loop使用fixed-size buffer/channel。prefix parser是纯byte状态机：

-prefix + `.` => local detach；
-prefix + prefix => 输出一个prefix byte；
-其他组合/timeout按配置原样发送；
-disabled时所有bytes透明发送。

非Active状态reader继续drain但丢弃普通bytes。SIGWINCH只更新latest viewport；Active时发送，
Synchronizing/Reconnecting时等snapshot ack后发送一次最新值。

## 10. Identity reset

`zterm reset --identity`不是数据库局部清空，而是本机身份销毁边界：

1. side-effect-free读取setup/daemon/session影响；
2.显示EndpointId、活动Session数量、所有本机配对失效且不广播RevokeSelf；
3. interactive确认或要求`--yes`，有活动Session另要求`--force`；
4. bounded stop并等待owned socket/daemon lock释放；
5.取得`lifecycle.lock`，再次确认daemon stopped；
6.从exact effective-user`UserPaths`验证root ownership/type/no symlink，只删除受管理state root，
  使用目录fd/no-follow或逐项验证，绝不接受用户路径参数；
7. sync parent并返回“run zterm setup”。不在同一命令生成新identity。

中途失败返回精确错误且可安全重跑；binary和安装metadata若位于state root之外不受影响。真正uninstall
及binary删除仍属于M9。

## 11. 错误、安全与资源

- remote peer只看到generic unauthorized，不区分unknown/revoked；local CLI可显示自己的typed诊断。
- terminal/input/cwd/ticket/proof/direct IP/route cache不进入tracing、error、JSON或snapshot fixture。
-每个local view一个bridge task；每remote连接、stream、handler、frame、control queue均复用现有上限。
-没有无限reconnect task：只有local view持有`ConnectionDemand`时重连；view drop释放demand和所有permits。
-stream write/read/deadline、signal shutdown、handler join与identity reset全部有absolute deadline。
-remote resume checkpoint最多一个controller-sized visible-grid projection，不增加host scrollback或磁盘状态。

## 12. 兼容、测试与延后项

- proto v1只新增字段/kind；numeric registry、Debug redaction和unknown-field fixtures同步更新。
- local IPC重构先跑现有全部M3/M4 tests，证明字节/错误/stop/final-drain行为不回归。
- pure/fake transport tests覆盖handler、generation、retry、bridge state/ID rewrite/backoff，无socket可优先。
- macOS运行Unix socket、PTY、CLI与纯状态测试；real Iroh只`--no-run`/Clippy。
- Linux CI运行两个production daemon/broker/remote Session/多个CLI的loopback real-Iroh gate；该证据
  不冒充official-n0公网或双NAT实验。
- Windows只编译shared代码并返回UnsupportedPlatform；不加入Unix-onlyprivate dead code。
- history paging、observer、mobile cold-tab event subscription、GUI、M9发行和M10网络lab保持延后。

## 13. 关键取舍

- 选择daemon bridge而不是CLI直连Iroh：保证多个CLI共享一条connection、secret不离开daemon，
  并让未来GUI复用local IPC；代价是需要显式AttachmentId映射和local reconnect state。
- 选择一个resume checkpoint而不是per-revision ring：满足短断线delta优化且保持M4 latest-only有界模型；
  cache失配时多传一次snapshot但永远正确。
- 选择首次成功后锁定SessionId：不会在remote daemon restart后把新`main`伪装为旧任务；代价是用户需
  明确重新执行connect创建新main。
- 选择per-invocation escape配置而不迁移config schema：本任务可验证改键/禁用且不扩大持久状态；
  后续若需要全局偏好可作为兼容config字段增加。
