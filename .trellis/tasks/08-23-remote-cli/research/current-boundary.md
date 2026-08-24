# M7-M8 当前代码边界与实施接缝

> 2026-08-24 execution note: before spawning more workers or widening evidence,
> read [execution-retrospective.md](./execution-retrospective.md). This task has
> completed the one user-authorized, behavior-preserving cleanup pass and has
> reached its local stop condition; remaining work is commit/finish plus the
> explicitly hosted evidence listed there.

## 结论

本任务不是从零实现远程终端。M4已经交付宿主终端真相和same-UID duplex路径，M5-M6已经
交付网络、配对和授权真相。最小正确实现是把两个已冻结的owner用一个remote wire adapter和
一个daemon-owned reconnect bridge连接，再把已有hidden IPC client通过安全CLI公开。

```text
CLI raw terminal
  -> same-UID local IPC
  -> local daemon RemoteAttachmentBridge / RemoteUnaryClient
  -> existing ConnectionDemand + one promoted Iroh connection
  -> remote SessionWireServer
  -> existing SessionService / SessionActor / PTY / TerminalModel
```

local target继续是：

```text
CLI -> same-UID local IPC -> the exact same SessionWireServer -> SessionService
```

## 已存在的owner

### M4 Session owner

- `SessionService`是daemon内唯一Session registry/replay/resource/controller入口。
- `SessionAttachment`已经提供snapshot ack、merged delta/full resync、input、resize、takeover、
  lifecycle watch与final drain。
- `LocalAttachmentClient`已经证明真实Unix duplex framing和严格attachment状态，但它是test/internal
  adapter，不是raw TTY UI。
- local IPC已经用一个`FrameDecoder`处理首frame leftovers、strict unary EOF、bounded control queue
  与latest-only revision/lifecycle watch。
- remote principal已经存在为`AttachmentPrincipal::RemoteEndpoint { device_id, auth_generation }`；
  revoke已经能按DeviceId detach匹配principal而不结束Session/PTY。

### M5-M6 transport/auth owner

- production daemon拥有唯一Endpoint、`ConnectionBroker`、`AuthorizationRegistry`、StoreActor、
  DeviceDirectory与PairingService。
- `ConnectionDemand`是remote connectivity的RAII consumer；同一DeviceId共享PeerSlot、dial worker与
  primary connection。`open_bi(StreamPurpose::Service, deadline)`返回受限
  `AuthenticatedBiStream`和remote receiver generation。
- inbound normal connection在Hello前授权，candidate注册后再次精确generation检查；revoke关闭
  candidate/stream。
- 当前`handle_service_stream`只验证M7 kind并返回`service_not_implemented`，没有Session副作用。
- `AuthorizationRegistry::acquire_commit`返回`AuthorizedCommitContext`，其`run`把read permit带入
  blocking effect；这是remote Session副作用唯一正确入口。
- hidden `LocalPairingClient`和`LocalDeviceClient`已经完成same-UID、strict EOF、byte-identical retry、
  ticket zeroize和方向化DeviceSummary，只缺M8 CLI UX。

## 必要重构

1. 从`local_ipc`抽取crate-private、transport-generic的Session wire server：共享request decode、
   target/principal validation、SessionService调用、terminal reader/writer和snapshot projection。
   UnixStream与Iroh Send/RecvStream只是不同I/O adapter，不能复制第二套语义。
2. 给ConnectionBroker增加一次性安装的normal service handler。handler只收到fully authenticated
   remote identity/generation和owned stream；它不持Endpoint/profile/PeerSlot。composition在bind前完成，
   避免broker↔DaemonService强引用环。
3. local unary dispatcher对remote target使用broker打开service stream；对local target继续直接调用
   shared wire server。remote mutation保存原始payload并在一次ambiguous retry中重用同一frame bytes。
4. local duplex dispatcher对remote target建立`RemoteAttachmentBridge`。它保持local socket和
   `ConnectionDemand`，把每次remote AttachmentId映射到稳定local view，明确报告reconnecting，
   并在新snapshot ack前丢弃input。
5. `LocalRuntime`提供不暴露UserPaths/identity/SQLite的高层pair/device/session/attachment入口；CLI
   仍是薄解析、确认、TTY guard与渲染层。

## 不能采用的捷径

- 不能让CLI直接使用Iroh、读取identity.key/SQLite或自行缓存remote connection。
- 不能在remote adapter创建第二个Session registry、OperationWindow、terminal parser或frame codec。
- 不能只在stream开始检查一次generation然后让input/resize绕过`AuthorizedCommitContext`。
- 不能在network断开时close Session、向PTY发signal或把输入排队到重连后重放。
- 不能用alias/session name作为authorization；授权只来自TLS DeviceId和本机generation。
- 不能在开发者macOS运行real Endpoint测试。Linux CI拥有real-Iroh loopback执行；M10拥有双网络实验。

## 需要验证的技术风险

- generic AsyncRead/AsyncWrite抽取必须保持local首frame leftovers、strict EOF和final-output-before-ended
  现有回归全部绿色。
- remote stream重建时remote AttachmentId变化；映射错误会让旧ack/input命中新controller。
- response-loss重试若decode/re-encode而改变unknown fields/bytes，会破坏operation replay证据；应保留
  原始payload/envelope。
- raw terminal Drop只能处理可捕获退出；SIGKILL/进程崩溃无法执行恢复。CLI应使用Tokio Unix signal
  select处理SIGINT/SIGTERM/SIGHUP，并依赖用户shell/terminal在不可捕获退出后的标准恢复手段，不做
  虚假保证。
- identity reset是不可恢复操作，必须在exact UserPaths、held lifecycle lock、daemon确认停止和
  symlink/owner validation之后删除，且需要明确`--yes`/`--force`边界。
