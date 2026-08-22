# M4 技术设计：持久 Session、PTY 与本地 attach

## 1. 边界与设计原则

M4 把 Foundation 已验证的 PTY/VT 能力和 M3 已验证的 per-user daemon/IPC 组合成一个
可长期持有进程的 Session 内核。它不新增第二个终端模型，也不提前实现网络 transport
或最终 CLI UI。

核心原则：

1. **唯一服务、唯一状态**：local IPC 和未来 remote QUIC 只做适配，全部调用同一个
   SessionService/SessionActor。
2. **PTY 不属于连接**：连接、attachment 和 controller 都可消失，PTY 只由 SessionActor
   的显式生命周期结束。
3. **最新状态而非输出日志**：TerminalModel 是真相；attachment 只保留一个 checkpoint
   和最新 revision 通知，没有逐 revision backlog。
4. **先准入再分配**：资源检查、名称/cwd 验证和 operation replay 判定都在产生副作用前
   完成。
5. **一次边界一次验证**：protobuf/FrameDecoder、SessionName、TerminalModel projection
   和 PtyHost 各自拥有自己的不变量，adapter 不复制解析或估算逻辑。

## 2. 组件与数据流

```text
same-UID Unix socket                     future authenticated QUIC
          │                                         │ (M5+，本任务不实现)
          ▼                                         ▼
  LocalSessionAdapter ───────────────┐     RemoteSessionAdapter
                                    │
                                    ▼
                           SessionService
                                    │
                    ┌───────────────┴───────────────┐
                    ▼                               ▼
             SessionRegistry                ResourceGovernor
       name/id index + replay windows     count + cell reservations
                    │
               one actor/session
                    ▼
     SessionActor { TerminalDriver, attachments, controller lease }
                    │
       PTY reader → one TerminalModel → Revision/watch watermark
                                             │
                           latest-only sync per attachment
```

`SessionService` 是 daemon 内部的 typed API，不知道 UnixStream、QUIC、prost bytes 或 CLI
渲染。adapter 负责 principal、deadline、frame 与 domain/proto 转换；service 负责会话语义。

## 3. Domain 类型与不变量

### 3.1 SessionName 与 selector

- 新增 `SessionName` newtype，唯一验证入口：UTF-8 1–64 bytes、不得含 Unicode control、
  不得有首尾空白、比较大小写敏感，不做隐式 trim/normalization。
- `main` 是保留的 `SessionName`。只有 default attach 的 create-if-missing 路径能创建它；
  普通 create 和 rename-to/from `main` 返回 typed reserved-name error。
- `SessionSelector` 是 `Id(SessionId) | Name(SessionName)`。进入 registry 后尽快解析成
  SessionId；rename 只改 name index，不改 ID。
- SessionId 与 AttachmentId 使用操作系统 CSPRNG 生成的 128-bit bytes；SessionId candidate 的
  碰撞检查与 resource insertion 在固定 `registry state -> resources` lock order 内一次完成，绝不
  check-then-reserve 或覆盖既有 projection；不引入 UUID 文本解析或持久计数器。

### 3.2 生命周期状态

```text
Starting ──spawn success──> Running ──root exit──────> Ended(NaturalExit)
    │                          │  ├──explicit close──> Ended(ExplicitClose)
    └──failure──> no entry     │  ├──daemon stop────> Ended(DaemonStop)
                               │  └──driver failure─> Ended(DriverFailure)
                               └──detach/disconnect──> Running
```

registry 对查询只发布 `Running`，但对 ownership 同时跟踪 provisional actor。create 的名称、
资源、actor entry 共用一个 pointer-identity ownership token；actor spawn 后立即交给 CreationOwner，
再尝试普通 provisional registration，全部成功且仍持有
同一个 `Starting(CreationCell)` name slot 时才一次性替换为 live name/id index。rename 在同一短
registry lock 下检查该 slot，所以 create-vs-rename 不会产生重复 name。publication ownership
丢失或 cancel 时不得先删 Starting slot；显式 close/reap/join 新 PTY/driver 后才 compare-release
matching name/resource。若 registration/wait/join/deadline 失败，actor 留在 provisional/cleanup-only
owner registry 供 shutdown cleanup 重试并让 status 保持可诊断，不能成为隐形 child/capacity。

`SessionEndReason` 保留 exit code/signal 或 bounded error category，不记录终端内容。actor
结束时通过 registry-owned completion channel 请求 compare-and-remove，保证 close/root-exit
竞态只释放一次资源且不会删掉后来复用同名创建的 Session。

### 3.3 actor 所有权

每个 `SessionActor` 是一个独立命名 OS thread，串行消费容量 16 的 synchronous command
channel，拥有：

- SessionId 与不可变 runtime metadata；registry 持有 name/id index 和展示 metadata；
- 唯一 `TerminalDriver`（其内部仍由已有 reader/model 两线程顺序排空）；
- `AttachmentId -> AttachmentState` map；
- `ControllerLease { attachment_id, generation }`；
- 当前 `TerminalResourceProjection` reservation；
- 结束状态与一次性 completion sender。

actor 用短周期 `try_wait`/revision watermark 与 `recv_timeout` 驱动。每个 command 带 absolute
deadline 和 queued/started/expired atomic gate；queue admission 只 `try_send`，已过期的 queued
command 不开始副作用，已 started 的 mutation 即使 caller timeout/disconnect 仍完成并记录
exact result。local current-thread Tokio runtime 只用 `spawn_blocking` 调用同步 API。

TerminalDriver 把可能阻塞的 PTY writer/master 与 child control 分离；actor 可在自己的 thread
阻塞 I/O，但 daemon close thread 仍可独立 kill/reap。root 已退出或 explicit close 后 actor 等待
reader EOF、模型队列排空并 join 自己的线程；attachment drop 仍无 close authority。
actor worker、close thread、creation owner、spawned PTY 和 TerminalDriver 各有 unwind finalizer：
panic 被转换为 typed failure/outcome unknown，同时必须完成 waiter/end/interrupt 状态或保留
registry-visible ownership。TerminalDriver/SessionActor Drop 先 interrupt/abort，再把 exclusive-take
的 JoinHandle 交给 self-join-safe background reaper，绝不在 Tokio/direct caller 上 join；actor token
只有在 child/thread ownership completion signal 后才能释放。poisoned cleanup lock 恢复后仍必须
compare matching token，normal operation 的 synchronization error 不被伪装成成功。

## 4. SessionRegistry 与 SessionService

### 4.1 Registry

`SessionRegistry` 维护：

- `BTreeMap<SessionId, SessionHandle>`；
- `BTreeMap<SessionId, ProvisionalSessionHandle>`；
- `BTreeMap<SessionName, SessionId>`；
- 一个 `ResourceGovernor`；
- 以 `(AttachmentPrincipal 的稳定设备身份/auth generation, lease ordinal)` 为 key 的有界
  per-operation singleflight/result cell registry；
- actor completion receiver。

所有 index 变更由 registry 的一个短临界区完成；不得持锁等待 actor、PTY、socket 或磁盘。
并发 `attach_default_main` 使用同一临界区的 reservation/singleflight，只允许一个 creator；
其他调用等待该结果后 attach 同一 SessionId。

### 4.2 Transport-independent API

内部 API 按 domain 类型返回 typed result：

```text
list(principal)
issue_operation_lease(principal)
create(principal, operation_id, name, cwd, initial_size)
rename(principal, operation_id, session_id, new_name)
close(principal, operation_id, session_id)
prepare_attach(selector, create_main, takeover_intent, viewport)
snapshot_applied(attachment_id, revision)
next_terminal_update(attachment_id)
input(attachment_id, bytes)
resize(attachment_id, size)
takeover(principal, operation_id, session_id, attachment_id)
detach(attachment_id)
shutdown() / shutdown_until(deadline)
```

M4 的本机 same-UID principal 拥有当前用户全部 Session 能力，不再加 session ACL。未来远端
adapter 在进入该 API 前完成设备授权；service 不复制 authorization storage。

global replay mutex 只做短暂 lease/operation 注册；winner 在锁外执行，same OperationId 只有在
完整 semantic fingerprint（name、cwd 的 Option/bytes、viewport、session 等）相同才 join/replay
完整 domain success/error，不同 payload 返回 OutcomeUnknown，unrelated key 可并发。winner
panic/drop 由 completion guard 终态完成 OutcomeUnknown，所有 duplicate waiter 都会被唤醒。
每 lease 保留 128 个结果，最多 64 个活跃 lease。

lease 只能由 daemon 签发：`OperationLease { daemon_incarnation: [u8; 16], ordinal: u64 }`，其中
incarnation 是 daemon-lifetime CSPRNG，ordinal 在 stable principal/auth generation 下单调递增。
incarnation mismatch 必须先于 ordinal/floor mutation 检查；invented/high/missing/retired ordinal
都返回 OutcomeUnknown 且不执行。lost lease response 可留下 empty lease，empty/used lease 一起按
fully-completed prefix 回收并记录 exact `retired_through` floor；in-flight result 不为容纳新 lease
而丢失。ordinal 和 sequence 穷尽显式失败，不能 wrap。窗口不跨 daemon restart 或写磁盘。

readiness/status/list 不获取 lease、不写 replay state。一个逻辑 LocalClient 在首个 mutation 前
lazy 获取并缓存 lease；ambiguous transport 最多用同一 absolute deadline 重试一次 byte-identical
request/ID。完整 typed response 都是 definitive；OutcomeUnknown 只 poison 当前 cache，不把同一
mutation 自动换 lease 重跑，后续独立用户操作才可申请新 lease。M4 没有通用 fresh-process 自动
恢复；只有明确导出的 opaque retry token（当前为 takeover continuation）可跨 client object 使用。

## 5. Terminal attachment 状态机

### 5.1 状态

```text
PreparingSnapshot
        │ snapshot 已生成并登记 checkpoint
        ▼
AwaitingSnapshotApplied(revision)
        │ exact ack
        ▼
Active(controller_generation)
        ├── revision dirty ──> latest Delta | full Snapshot
        ├── takeover away ──> LeaseLost ──> Detached
        ├── root exit/close ─> SessionEnded ─> Detached
        └── EOF/Detach ──────> Detached（Session 仍 Running）
```

普通 attach：

- controller 空闲时创建 attachment、取得下一 generation，并返回 snapshot；
- controller 被占用时返回 Occupied，不创建 observer。

显式 takeover 采用准备与提交两步，但 CLI/adapter 后续可以封装为一个用户动作：

1. `prepare_attach(takeover_intent=true)` 创建只读的 pending attachment 并准备 snapshot，
   不影响旧 controller；
2. 新 attachment 精确确认 snapshot；
3. 带 OperationId 的 `takeover` 在 actor 内一次性递增 generation、切换 lease、激活新
   attachment，并把旧 attachment 状态改为 LeaseLost。
4. 如果提交响应丢失，新 socket 建立一个 fully synchronized pending attachment，并携带同一个
   opaque operation token 继续。controller 为空或仍由该 operation tag 拥有时，新 attachment
   权威接管；若 later/different operation 已安装 controller，则返回 truthful error，绝不 clobber。

这样 snapshot 准备失败不会抢走控制权，提交点又可以精确去重。旧 controller 的 input/
resize 在 actor 验证 `(attachment_id, generation)` 后才写 PTY，因此切换后没有双写窗口。

### 5.2 同步与慢客户端

每个 attachment 持有已有 `TerminalAttachment` checkpoint，并得到两个容量固定的 watch：

- `latest_revision`：只覆盖为最新值，不排队；
- `lifecycle`：Active/LeaseLost/SessionEnded 的最新终态。

checkpoint 从 latest visible ANSI 重建 zero-scrollback parser，只保留 main/alternate visible grids
（`rows * columns * 2` cells），不 clone host scrollback。1.0 每 Session 只允许一个 controller 和
一个 pending takeover；第二个 pending attach 返回 Occupied，不扩大 checkpoint 集合。

socket writer 收到 revision 变化后向 actor 请求一次 `next_terminal_update`，由 checkpoint 生成
一个合并 delta 或完整 resync。若输出在写 socket 期间继续推进，watch 仍保留更高 revision，
writer 下一轮再同步；慢 client 不会让 actor 或 PTY reader 等待。

attachment 仍在 AwaitingSnapshotApplied 时只合并 dirty revision，不向它发送 delta，也不
接受 input/resize。精确 ack 后，writer 从登记的 snapshot checkpoint 一次同步到最新 revision。

首次/full snapshot 的 checkpoint 在服务端登记，但 attachment 保持 AwaitingSnapshotApplied。
ack revision 不精确、client 报告 future revision、尺寸改变或 checkpoint 不兼容时，服务端丢弃
旧 checkpoint、发送 `TerminalSyncRequired` 加最新 snapshot，并继续拒绝 input/resize。

snapshot 编码优先保证当前 screen。若 protobuf envelope 加 screen 与 history 超过 8 MiB，
TerminalModel 的 bounded snapshot builder 只从最旧端删除完整历史行，并在 ANSI reset/行边界
重新开始；不得从任意 byte 截断，也不修改宿主 scrollback。其等价性和 encoded_len 在
TerminalModel/proto conversion 的唯一测试中验证。

## 6. 资源治理

`TerminalModel::project_resources(size, scrollback_rows)` 成为不分配 parser 的公开 checked
projection 入口；constructor/resize 和 registry 都复用它，禁止复制 cell-size 算法。

`ResourceGovernor` 在 mutex 内维护 live count 与 summed projection：

- create：先验证 name/cwd/size，再 reserve count + projection，随后 spawn；失败自动回滚；
- resize：先计算 old/new delta 并 provisional reserve，再让 actor 执行原子 resize；失败回滚，
  成功提交新 projection；
- end：completion compare-and-remove 后释放一次 reservation。

viewport 任何一维为 0 或超过 240x80 均拒绝，不自动 clamp；创建时没有有效 viewport 才用
120x40，controller detach 后保留最后有效尺寸。固定 scrollback 为 2,000 行，M4 不增加动态
配置或“超限后偷偷裁状态”的兜底。

## 7. Protobuf 与本地 IPC

### 7.1 最小 wire 调整

在现有 v1 registry 上补齐当前消费者需要的 shape：

- attach request 中保持既有 `session_id` 字段并新增互斥 `session_name` 选择与 initial viewport；
- `TerminalLeaseLost`；
- `TerminalSessionEnded` + typed end reason；
- SessionSummary 的 working directory（只来自已验证 Session metadata）。

沿用 `TerminalSnapshot` 作为 attach 成功的首个服务消息、`SessionMutateResponse` 作为
takeover 的 correlated response、`ServiceError` 作为 typed failure；不新增通用 event bus、
observer message、history paging 或每个 input 的成功 ACK。所有 MessageKind 数字、prost enum、
WireKind registry 和 round-trip matrix 同步更新。

这些 proto 尚未随产品发布。已有 message kind 和 `session_id` field number 保持不变；M4 测试
冻结第一次可执行的 DTO 形状，避免为了草稿兼容另建第二个 selector/parser。

### 7.2 首 frame 分类

same-UID peer credential 在读取任何 frame 前完成。之后同一个 FrameDecoder 读取首 frame：

- unary session/lifecycle kind：继续读取到 EOF，要求严格一个 frame，再 dispatch/reply/shutdown；
- `TerminalAttachRequest`：保留 decoder 中同批次剩余 bytes，进入 duplex attachment loop；
  client half 和 server half 分别由有界 reader/writer task 驱动，任一结束触发一次 detach。

所有可能进入 Session actor/PTy 的同步调用通过 `spawn_blocking`，并携带 frame 导出的 absolute
deadline；Tokio current-thread runtime 不 inline 等待 mailbox admission 或 blocking effect。

attachment reader 仅接受 snapshot-applied、sync-request、input、resize、detach 和针对本
attachment 的 takeover。状态不合法、ID 不匹配、过期 generation、超限或超时只关闭该
attachment stream；未知/越界 frame 仍由唯一 proto decoder 拒绝。

M3 `LocalClient` 继续只做 unary。新增 daemon-internal/test-facing `LocalAttachmentClient`，
证明真实 socket duplex 行为；M8 才把它连接到 raw TTY 和用户命令。
新增 kind 207/208 的 lease request/response；只 mutation path 使用。request ID、operation sequence
和 attachment client ID 均无 silent saturation/reuse（attachment ID 使用 CSPRNG）。recoverable
accept error 留在 listener loop 并继续服务，不以 accept 抖动释放 Session ownership。

## 8. daemon 生命周期集成

- `run_daemon` 创建一个 Arc SessionService 并注入 DaemonService/local listener；status、
  doctor 和 stop 从同一 registry 读取 live summaries，不维护影子计数。
- stop 先调用 `shutdown_until(deadline)`，先向每个 live/provisional actor 发 explicit close，再等待
  有界 finalization 并发送 SessionEnded；普通 ended SessionNotFound summary race 可省略，其他
  summary/wait/join typed error 必须在所有 owner 都收到 close 后上报。然后才构造准确
  SessionImpact，flush stop response 后停止 listener。
- 若 absolute shutdown deadline 到达时仍有 child、driver thread、actor 或 provisional resource，
  stop 返回 DeadlineExceeded，不发送 `stopping=true`，listener/socket 继续提供 status 与 retry。
  只有所有 ownership 释放后才 flush success 并退出 listener。
- recoverable accept error 不退出 serve loop。actual fatal server termination 也必须 cleanup 所有
  Session；若 cleanup 失败，`run_daemon` 保留 process/daemon lock/store/service/children，以之前绑定
  socket 的 dev+inode token compare-rebind（temporary failure backoff retry），恢复 status/stop；只有
  ownership 全释放后才 compare-unlink/退出。
- restart 复用 stop 后的既有 detached spawn。新 daemon 创建空 registry，但读取同一持久
  DeviceId/config/authorization state。
- unexpected daemon crash 不写恢复清单；这是已明确的 1.0 边界，不增加自动恢复或 rollback。

## 9. 错误与竞态矩阵

| 条件 | 结果 |
| --- | --- |
| name 已存在/保留/非法 | typed error；无 PTY、index 或 reservation |
| cwd 不存在、非目录或不可访问 | PtyHost/domain typed error；无 Session |
| 第 9 个 Session、过大 viewport、projection 超限 | ResourceExhausted；现有状态不变 |
| 普通 attach 遇到 controller | Occupied；无 attachment 副作用 |
| snapshot 未确认就 input/resize | NotSynchronized；bytes 不写 PTY |
| stale attachment/generation | LeaseLost/FailedPrecondition；不写 PTY |
| delta baseline/gap 不匹配 | SyncRequired + full snapshot |
| client EOF/写失败 | detach；Session/PTY 继续 |
| root Shell 自然退出 | final drain，SessionEnded(NaturalExit)，移除一次 |
| close 与 root exit 竞态 | actor 只提交一个 end reason；registry compare-remove/release 一次 |
| 已提交响应丢失后重试 | replay 原 result；不重复副作用 |
| 同 OperationId + 不同 payload | OperationOutcomeUnknown；不 replay unrelated success |
| restart/invented/high/retired lease 或 result 已淘汰 | OperationOutcomeUnknown；不重新执行 |
| replay/actor/close/create panic | typed failure/OutcomeUnknown；waiter 唤醒且 ownership 可清理 |
| shutdown summary 非普通 SessionNotFound error | 全部 owner 先 close，再上报 error；不 stopping |
| 一个 attachment protocol error | 只关该 stream；其他 Session/daemon 继续 |

## 10. 测试策略

每个契约保留一个权威测试，不重复静态断言已经由运行时证明的行为：

- core unit/property-style tests：SessionName、selector、lease/replay、projection preflight；
- proto tests：kind registry、DTO round-trip、字段/encoded frame bounds；
- daemon pure/concurrent tests：registry index、main singleflight、reservation、end races；
- real PTY integration：lifecycle、无 attachment drain、snapshot recovery、takeover；
- real same-UID Unix socket：unary + duplex 状态和故障隔离；
- explicit black-box gate：tmux 与固定 Herdr，共用一个 harness，不进入普通 push 的重复下载；
- hosted matrix：Unix 跑 runtime tests，Windows 只跑共享契约和 unsupported compile boundary。

不连接公网、不使用 Iroh、不修改生产 Relay、不读取用户真实 `~/.zterm`、不碰用户已有 tmux/
Herdr server；所有 fixture 使用唯一 TempDir/socket，并只清理自己的资源。
