# Iroh Transport 与设备认证设计

## 1. 范围与设计结论

本任务实现父任务 Phase 1 的 M5–M6。交付边界是一个由 per-user daemon 独占的 Iroh
transport/auth subsystem，供现有 same-UID local IPC 调用，并为 M7 remote Session adapter
提供受认证 stream API。

本任务不会新增 public `zterm pair ...`/`zterm device ...` command tree；M8 才实现 TTY、
stdin/argv、人类/JSON 输出与 destructive confirmation。现有 `status`/`doctor` 已是公开诊断面，
本任务会扩展其 typed observation。

以下父任务决策不再开放：

- pairing 是 SSH-like 单向授权；A 创建 ticket、B 接受只产生 `A authorizes B`；
- pairing 本身就是双方显式意图，不再要求 A 本机二次确认；
- pairing 授予当前 OS 用户下的完整 host terminal trust，不做 per-session ACL/guest role；
- 没有账号、中央 revoke、`RevokeSelf` 或卸载协调；每台 host 分别 revoke；
- Session/PTY lifetime 独立于 connection、path、stream、attachment 与 authorization。

## 2. 不变量与唯一 owner

| 不变量 | 唯一 owner | 其他层只能做什么 |
| --- | --- | --- |
| 长期设备身份 | `DeviceIdentity` + daemon-owned `Endpoint` | CLI 只看 public DeviceId |
| configured infrastructure profile | `InfrastructureProfile` | route hint 只作为 dial candidate，不改 map |
| 每 remote primary connection | `ConnectionBroker` | local/M7 adapter 只申请 demand/stream |
| 入站 authorization truth/generation | `StoreActor` + `AuthorizationRegistry` | wire/CLI 只投影状态 |
| 出站 known-device alias/route | `DeviceDirectory` 经 `StoreActor` | M8 以后解析 selector |
| 一次性 offer | `PairingManager` | IPC 只创建/接受，不复制状态机 |
| Session/attachment/controller | `SessionService` | transport 只带 principal 调用/按 principal detach |
| frame 编解码/kind | `zterm-proto` | local/QUIC adapter 复用同一 codec |

总体组合：

```text
same-UID LocalClient
        │ Unix peer UID + existing frame codec
        ▼
DaemonService ─────── SessionService ───── PTY/TerminalDriver
   │                        ▲
   │ pair/device/status     │ remote principal + bounded detach
   ▼                        │
PairingService ───── AuthorizationRegistry
   │                        │
   ├──── DeviceDirectory ───┤
   │                        ▼
   └──── ConnectionBroker ─ StoreActor ─ SQLite v1
                 │
                 ▼
        daemon-owned Iroh Endpoint
        zterm/1 + zterm-pair/1
```

任何 local listener recovery、network reconnect 或 revoke 都不能创建第二个 store/session/
connection registry。

## 3. Core domain 与资源契约

### 3.1 新增 transport-neutral 值

`zterm-core` 新增：

- `DeviceAlias`：1–128 UTF-8 bytes，无首尾空白和 Unicode control，精确值唯一；`local`
  是保留值。显式 alias 冲突返回 typed error；未显式 alias 时使用 remote name，冲突或保留
  时追加稳定的 short EndpointId，并在 128-byte 边界内截断前缀。
- `PairOfferId([u8; 16])`、`PairNonce([u8; 32])`、redacted/zeroizing `PairSecret`；
- `PairTicketFields`：format version、host DeviceId/name、relay hints、offer ID、expiry；secret
  与公开字段分离，避免普通 Debug 投影 bearer value；
- `RelayHint`：validated HTTPS URL string，最多 2048 bytes；core 不依赖 Iroh URL type；
- `ConnectionAttemptId([u8; 16])` 与
  `ConnectionCandidateKey { initiator: DeviceId, attempt: ConnectionAttemptId }`，按 bytes
  lexicographic 排序；
- `AuthorizationSnapshot { status, generation }`、`AuthGeneration` checked-next helper；
- `TransportLimits`，不把 network 数值继续塞进 Session 的 `ResourceLimits`。

Core 仍不得依赖 prost、Iroh、SQLite、CLI 或 OS API。Canonical ticket/transcript builder 和
HMAC domain separation 可依赖 exact `ring`/`zeroize`，protobuf/base64 适配留在 proto/daemon。

### 3.2 默认资源上限

初始 production defaults 对应当前 desktop daemon、8-session 上限和 M7 consumer：

| 资源 | 上限 |
| --- | ---: |
| active authenticated remote connections | 32 |
| pending outbound dials | 16 global、1/EndpointId singleflight |
| accepted-but-not-authenticated normal connections | 16 global、2/EndpointId |
| pairing handshakes | 8 global、1/EndpointId |
| bidirectional streams | 32/connection |
| concurrent stream handlers | 16/connection、128 global |
| broker open-stream request queue | 32/connection，队列不持有 payload bytes |
| StoreActor command queue | 64，sole bounded sync channel |
| live pair offers / local ticket replay cells | 16 |
| ticket text | 16 KiB |
| relay hints | 4，每条 2048 bytes |
| hello/pair frame | 16 KiB |
| one pairing handshake total bytes | 64 KiB |
| first application frame deadline | 5 s |
| pairing total deadline | 15 s |
| fresh address lookup budget | 2 s |
| one connect attempt budget | 10 s |

全局 frame 仍是 8 MiB、control payload 仍是 1 MiB；上表是更紧的产品层 admission。所有
计数用 checked arithmetic，零值配置拒绝，任何 exhaustion 返回 `resource_exhausted`，不能
wrap、无界排队或用 sleep 掩盖。

### 3.3 Typed errors

在现有 `DomainErrorKind` 最小新增：

- `AddressUnavailable`、`TransportUnavailable`；
- `Unauthorized`、`AuthorizationRevoked`；
- `PairTicketInvalid`、`PairTicketExpired`、`PairTicketConsumed`、
  `PairOutcomeUnknown`；
- `InvalidDeviceAlias`、`DeviceAliasConflict`、`DeviceNotFound`。

remote peer 只能收到泛化的 unauthorized/pairing-rejected/overloaded/incompatible close reason，
避免区分 unknown 与 revoked、missing 与 consumed。更细类别只在 same-UID 本地观察面使用。

## 4. Protobuf、ALPN 与 framing

### 4.1 ALPN

- `zterm/1`：normal authenticated connection；
- `zterm-pair/1`：短生命周期 pairing；
- 禁止 pairing/auth/mutation 使用 `into_0rtt`；只等待完整 1-RTT TLS authentication；
- pair connection 永不升级或插入 normal `ConnectionRegistry`。Pair 完成后用 normal ALPN
  做 authorization confirmation。

### 4.2 Kind 分配

保留已有 1–11、200+、300+，把占位 100/101 替换为完整协议：

| kind | message |
| ---: | --- |
| 12 / 13 | `LocalPairCreateRequest/Response` |
| 14 / 15 | `LocalPairAcceptRequest/Response` |
| 16 / 17 | `LocalDeviceListRequest/Response` |
| 18 / 19 | `LocalDeviceRenameRequest/Response` |
| 20 / 21 | `LocalDeviceRevokeRequest/Response` |
| 100 | `PairBegin` |
| 101 | `PairChallenge` |
| 102 | `PairProof` |
| 103 | `PairAccepted` |
| 104 | `ConnectionHello` |
| 105 | `ConnectionWelcome` |

现有 `ServiceErrorResponse` 可承载 generic typed failure；不为每种失败新增 kind。未来 M7
QUIC stream 直接以既有 200+/300+ service frame 分类，不新增“stream inside stream” envelope。

### 4.3 Message shape

- `PairTicketV1`：`format_version`、host DeviceId/name、repeated relay URL、offer ID、secret、
  `expires_at_unix`；它只作为文本 payload，不在 pair connection 上重发 raw ticket。
- `PairBegin`：offer ID、controller display name、controller nonce、pair protocol version；
- `PairChallenge`：host nonce、selected version、ticket expiry；
- `PairProof`：controller proof；
- `PairAccepted`：authorization generation、host confirmation proof、host diagnostic version；
- `ConnectionHello`：wire range、capability bits、attempt ID、initiator display/build/platform；
- `ConnectionWelcome`：selected wire version、capabilities、responder display/build/platform、
  receiver-side accepted authorization generation。

`initiator_endpoint_id` 不接受 wire 自报：双方从 Iroh connection side + authenticated
EndpointId 构造 candidate key，wire 只携带 initiator 生成的 attempt ID。

Local pair create/accept 带一个 128-bit client-generated `ephemeral_operation_id` 与语义
fingerprint。PairingService 用 bounded operation cell join/replay同一 mutation；同 ID 不同
payload 返回 outcome unknown。Device rename 是 exact set、revoke 是 idempotent tombstone，
因此 byte-identical local transport retry由数据库语义直接吸收，不建立第二套持久 replay 表。

### 4.4 Compatibility

- protobuf 是 source of truth；所有 bytes/ID/string/list 在 allocation/persistence 前转换为
  validated domain value；
- v1 unknown optional fields/capability bits 保留兼容；不兼容 transcript/wire 语义使用新
  format/ALPN major；
- golden fixtures包含 ticket text、canonical bytes、proof/confirmation 与 proto round-trip，
  不能依赖 Rust enum layout 或 prost 的字段输出顺序。

## 5. Daemon lifecycle 与 network readiness

### 5.1 Startup

`run_daemon` 保持现有 trust 顺序：state directories → daemon lock → committed store/setup →
owned Unix listener。随后：

1. 启动 sole `StoreActor`，读取所有 auth/known-device snapshot；
2. 构造 `AuthorizationRegistry`、`DeviceDirectory`、`PairingService` 与 network observation；
3. 构造唯一 Tokio runtime，同时启动 network supervisor 与 local listener；
4. local listener 一旦可服务 store/session 即返回 `DaemonReadiness`；network observation 初始
   为 `initializing`，不等待 Relay/DNS/Pkarr/Internet；
5. supervisor 从 committed identity/config build 并 bind Endpoint，成功后启动 ALPN accept、
   broker/path tasks；失败记录 redacted typed category、状态 `degraded`，按 250 ms→10 s capped
   exponential backoff + jitter 重试，不旋转 identity 或停止 local service。

Identity/config/store trust failure仍是 daemon hard start failure；纯 Endpoint bind/online failure
是 network degradation。`setup` 的 readiness probe继续只证明 local daemon，不谎称 remote
network online。

### 5.2 Listener recovery 与 shutdown

Network supervisor、StoreActor、SessionService 都位于 existing owned-listener recovery loop
之外。fatal local accept 后若 Session cleanup 未完成，exact-token rebind期间它们继续由同一
daemon owner持有。

最终 stop 顺序：

1. local stop mutation完成 SessionService bounded shutdown；
2. 成功 response flush + socket shutdown 后 listener 才退出；
3. network supervisor拒绝新 dial/accept，取消 pair/stream handler，关闭 peer connections；
4. `Endpoint::close().await` 在同一 absolute deadline 内完成；
5. StoreActor shutdown，最后 exact-token 删除 owned Unix socket并释放 daemon lock。

若 Session ownership仍未释放，沿用现有 rebind/retry契约，不提前关闭 service/store或报告
successful stop。Network outage、connection loss 或 broker cleanup从不调用 daemon/session stop。

## 6. Infrastructure 与地址解析

### 6.1 Effective profile

`InfrastructureProfile` 增加 `ZTERM_PAIR_ALPN`，其余保持 active spec：

- official：精确 Iroh 1.0.3 production default四 Relay + QAD 7842、production Pkarr/DNS；
- self-hosted：精确一条显式 HTTPS Relay、`RelayConfig::new(url, None)`、无 QAD；
- 两者都使用 relay-only publication 与 portmapper；
- 不调用环境感知的 `n0_dns()` shortcut，不允许 staging 或隐式 public/self-host混合。

### 6.2 Dial candidates

`RouteResolver::candidates(remote, transient_ticket_routes)`：

1. 在 2 s 内显式调用 `endpoint.address_lookup().resolve(remote)`，逐项验证 remote ID，提取
   signed relay-only result；
2. fresh result 可用时先拨 fresh；失败或无结果才尝试 version=1 SQLite cache；pair accept
   还可最后使用 ticket route；
3. 每个 candidate是只含目标 EndpointId + Relay URL 的 `EndpointAddr`；direct addresses只可
   来自 Iroh当前 connection/path，永不写 cache；
4. Iroh 1.0.3 可对未在本机 RelayMap 的 URL 动态创建 dial relay actor，因此禁止调用
   `Endpoint::insert_relay`。远端 route 不改变本机 home Relay、publication或profile summary；
5. 只有 Iroh remote ID与目标相等且 normal/pair application handshake成功，route才算 verified。

`RelayRouteCacheV1` protobuf只含最多四条 normalized Relay URL；unknown cache version返回
diagnostic并忽略，不迁移、不删除 known device。Normal handshake成功后原子更新
`last_seen`/verified routes；寻址失败不修改 identity、auth、known device或Session。

## 7. ConnectionBroker

### 7.1 API 与 ownership

对下游提供最小 API：

```rust
ConnectionBroker::demand(remote, deadline) -> Result<ConnectionDemand, DaemonError>
ConnectionDemand::open_bi(purpose, deadline)
    -> Result<AuthenticatedBiStream, DaemonError>
ConnectionBroker::close_remote(remote, reason)
ConnectionBroker::observe() -> watch::Receiver<NetworkObservation>
```

`ConnectionDemand` 是当前 consumer 的 RAII demand；同一 EndpointId 的 demand通过一个
`PeerSlot` singleflight dial/共享 primary。不同 EndpointId拥有不同 slot、semaphore和取消
状态。M7 local views以后持有 demand；本任务 pair normal-confirmation与测试 adapter是当前
consumer。没有 demand 时不运行无限 reconnect loop。

### 7.2 Duplicate arbitration

每次 dial在注册 pending candidate前生成 128-bit attempt ID。Candidate key：

```text
(authenticated initiator EndpointId bytes, attempt ID bytes)
```

双方都按 lexicographic minimum选择 designated primary：

- outbound/inbound connection取得authenticated remote ID与Hello attempt后先注册provisional；
  只有完成Welcome并被promote的candidate才能开放business stream；
- connection side验证 initiator，不能相信 peer自报 EndpointId；
- registry不持全局锁等待 dial/handshake；per-peer actor串行 register/promote/close；
- 新 candidate若胜出，先原子发布新 primary，再以 retryable duplicate code关闭旧 loser；
- loser close只唤醒该 connection的 stream/demand重试，不close Session、PTY或其他 peer；
- duplicate loser在同一PeerSlot仍有pending/primary candidate时不启动新dial；只有slot真正无
  candidate且仍有demand才进入正常backoff，避免同时拨号形成自激重连；
- per-peer outgoing singleflight避免正常路径同侧自造多个 attempt。

纯 core winner reducer和有barrier的双 Endpoint integration test分别证明排序与真实竞态；
不能依赖 wall-clock sleep猜测双方已收敛。

### 7.3 Normal handshake 与单向权限

入站 `zterm/1` connection取得 Iroh `remote_id()` 后，在读取任意 application frame前查询
本机 `device_auth`。Unknown/revoked立即generic close；authorized则记录当前 generation，
再在5秒内读取唯一Hello stream并协商版本/capability。

出站connection只要求目标存在于 `known_devices` 或来自 pair acceptance transient target；
它由远端host决定本机是否被授权。这样 A authorizes B 时，B可以拨A，而A不因共享QUIC
自动获得控制B的权限。

QUIC connection在transport上可双向开stream，但每个receiver在接受每条service stream时
仍按自己的 `device_auth(remote_id)` 校验，并在敏感提交点取得 expected-generation permit。
因此 mutually-authorized peers可共享一条primary；one-way peers只能在授权方向调用服务。

### 7.4 Stream isolation

- connection actor只负责accept/open、semaphore、AuthContext、path/close observation；
- 每条accepted stream独立task和first-frame deadline，读取同一 `FrameDecoder`；
- M5–M6 对已知 M7 kind验证auth后返回 `service_not_implemented`，不创建临时 Session adapter；
- malformed/oversize/stalled stream只reset自身；protocol-major/authorization/revoke才close
  connection；
- bounded semaphore在读payload前取得，超限拒绝；没有按frame/terminal revision的无界
  mpsc；
- `path_events()`只更新 `{direct|relay|unknown, relay_url?, selected}` typed observation，默认
  status不显示direct IP。

Reconnect只在存在 demand且错误可重试时运行，250 ms指数退避至10秒并加jitter；revoked、
incompatible和明确取消不重试。Path migration在同一connection内不触发reconnect/reattach。

## 8. PairTicket 与配对协议

### 8.1 Ticket text 与 canonicalization

固定文本：

```text
zterm-pair-v1:<base64url-no-pad(protobuf PairTicketV1)>
```

默认 TTL 10 min，允许 1–60 min。Host创建时同时保存wall-clock expiry与monotonic
`Instant` deadline；任一到期即过期，时钟回拨不能延长offer。创建ticket要求Endpoint已bind
且当前至少有一条home Relay hint；没有route时返回 `address_unavailable`，不创建offer，
但local Session/readiness继续可用。

Canonical encoding不使用protobuf output bytes：

```text
domain "zterm-pair-ticket-v1\0"
u32be format_version
32 bytes host_endpoint_id
u16be host_name_len + host_name UTF-8
u8 relay_count + each(u16be url_len + normalized URL UTF-8)
16 bytes offer_id
u64be expires_at_unix
```

Host 用 Iroh `RelayUrl::to_string()` 产生 ticket URL；controller验证它是有界HTTPS Relay URL，
但canonicalization使用ticket中原始UTF-8 bytes，不做可能跨语言不一致的二次normalize。
Relay URL顺序保留、重复拒绝。然后：

```text
ticket_digest = SHA256(canonical_ticket_without_secret)
offer_key = HMAC-SHA256(pair_secret,
  "zterm-pair-offer-key-v1\0" || canonical_ticket_without_secret)
```

PairingManager auth state只保存offer key、digest、expiry和state。为处理本地 create response
丢失，最多16项的operation replay cell可单独保留`Zeroizing<String>`完整encoded ticket；
consume/expiry立即清除。没有secret/ticket进入SQLite、tracing、error、status或snapshot。

### 8.2 Pair state machine

```text
Ready
  ├─ invalid proof / timeout ───────────────► Ready (若未过期)
  ├─ valid proof + CAS ─► Consuming
  │                         ├─ DB fail ─────► Ready (若未过期)
  │                         └─ DB commit ───► Consumed(controller, generation)
  └─ expiry ────────────────────────────────► Expired
```

Consumed tombstone保留到ticket expiry，verifier/raw replay result清除；同一或不同EndpointId都
不能再次触发authorize。Manager全局最多16个offer，清理以monotonic deadline为准。

### 8.3 Transcript 与 handshake

Controller先验证ticket语义/host ID/route，在pair ALPN上发Begin。Host发送32-byte nonce。
Transcript固定为：

```text
domain "zterm-pair-transcript-v1\0"
32 ticket_digest
32 host_endpoint_id
32 controller_endpoint_id
16 offer_id
32 controller_nonce
32 host_nonce
u16 controller_name_len + controller_name
u32be ticket_format_version
u32be pair_protocol_version
u64be expires_at_unix
```

Controller proof：

```text
HMAC(offer_key, "zterm-pair-controller-proof-v1\0" || transcript)
```

Host constant-time verify后才CAS到Consuming，再经StoreActor authorize/checked generation。
Commit成功后的confirmation：

```text
HMAC(offer_key, "zterm-pair-host-accepted-v1\0" || transcript || u64be generation)
```

Controller验证confirmation。Pair protocol最多64 KiB，first frame 5 s、总15 s；invalid字段、
offer状态或proof只发generic pairing-rejected。完整ticket/secret/proof不进入reason string。

### 8.4 Local create/accept 与丢响应恢复

`LocalPairCreate`：

1. validate TTL/operation fingerprint与network route；
2. generate ID/secret，构造ticket/derived verifier；
3. 原子插入offer + exact replay result；
4. strict unary返回ticket。相同operation ID重试返回byte-identical结果，不创建第二offer。

`LocalPairAccept`：

1. 在分配/日志前按16 KiB限制decode并zeroizeowned buffers；validate alias并通过
   `DeviceDirectory`保留alias；
2. pair ALPN完成host authorize；
3. 无论PairAccepted正常收到还是response ambiguous，都用ticket target/route尝试normal
   `zterm/1` confirmation；只有normal handshake证明host确已授权当前controller后才在本机
   transaction写`known_devices`/verified route并返回成功；
4. 后续同一ticket的新accept若收到generic pairing rejection，也只可尝试normal confirmation；
   normal auth成功表示此前已为当前EndpointId提交，可修复本地known device，失败绝不能
   重新开放Consumed offer或推断host的内部offer状态；
5. host未提交则ticket仍Ready可重试；host已提交但network/local store暂时失败则返回
   `pair_outcome_unknown`，保留远端authorization事实，用户可用同一ticket重新accept并通过
   normal confirmation修复，不能发明远端rollback/RevokeSelf；
6. operation cell保证local response丢失后的byte-identical retry加入同一个执行/结果。

Alias在network operation前由统一directory reservation持有，device rename也使用同一owner，
避免两个并发accept都让远端commit后才发现本地alias冲突。Daemon crash后的SQLite unique
constraint仍是最终owner；repair允许用户选择新alias。

## 9. StoreActor 与设备目录

### 9.1 保持 schema v1

不新增table/user_version。扩展StoreActor为cloneable `StoreHandle`，actor owner独占join handle
并在network task结束后shutdown。command改为容量64的`sync_channel`，每条command携带absolute
deadline/started gate；full queue只能在bounded blocking worker中等待或明确overload，不能在
Tokio current-thread阻塞或无界积累。所有SQLite操作仍在其线程；Tokio调用通过
`spawn_blocking`等待。

新增typed commands：

- list/get all `device_auth` 与 `known_devices`；
- authorize、idempotent revoke、last_seen；
- upsert known device/verified route、rename alias、alias availability；
- merged device projection所需字段。

Generation从`saturating_add`改为checked add并限制SQLite i64范围。Authorize总是从当前值
加1；revoke Authorized时加1并写tombstone，already Revoked返回现有generation且不再加；
missing revoke返回device_not_found。所有状态变化在Immediate transaction下运行，连接已
配置`PRAGMA synchronous=FULL`。

### 9.2 Directional projection

`DeviceDirectory::list()`按EndpointId合并：

- outbound：known alias、remote name、route verification；
- inbound：authorized/revoked、generation、paired/revoked/last_seen；
- live：primary/path/stream count与remote attachment count。

Rename只接受精确DeviceId并修改outbound alias；没有known row返回device_not_found。Revoke只
修改inbound auth；没有auth row返回device_not_found。M8以后可以用alias/short ID解析到精确
ID，但本任务IPC不接受模糊selector，避免误撤销。

## 10. AuthorizationGate 与立即 revoke

### 10.1 Registry/permit

启动时从SQLite预载每个EndpointId snapshot。每项拥有`Arc<tokio::sync::RwLock<...>>`与
`watch::Sender<AuthorizationSnapshot>`；外层短锁只查找/创建entry，不跨await。

- connection admission：读取snapshot、记录expected generation并订阅watch，不长期持read；
- stream admission：再次确认authorized/current；
- sensitive commit：取得`OwnedRwLockReadGuard`，校验expected generation，把guard移动到
  `spawn_blocking` closure，直到实际SessionService/PTY/store副作用返回；
- authorize/revoke：取得owned write guard。Tokio fair writer排队后阻止新reader越过，且
  writer会等待已经开始的commit结束。

M7必须通过`AuthorizedCommitContext::run(...)`进入SessionService，不能自行比较一个裸u64。
本任务用synthetic commit与remote-principal Session hook固定该API。

### 10.2 Revoke order

```text
acquire endpoint write permit
  → StoreActor FULL-synchronous transaction writes revoked + generation+1
  → publish in-memory revoked generation / wake cancellation watchers
  → ConnectionBroker closes all current connections/streams for endpoint
  → SessionService detaches all RemoteEndpoint(endpoint, any generation)
  → release write permit and return exact impact
```

DB失败时write guard释放，内存/watch/connection/attachment完全不变。DB成功后即使local
response丢失或daemon崩溃，重启预载tombstone，旧generation不能复活。Commit前已完成的PTY
write不可撤销；commit后旧permit不可能再到达副作用。

Revoke关闭当前transport以终止in-flight remote access，但不删除`known_devices`。如果本机
仍被该remote授权，未来本机可重新拨它执行outbound control；该新connection上remote发来的
service stream仍会被本机revoked gate拒绝。方向不能混淆。

### 10.3 Session principal hook

把`prepare_attach`改为显式接收`AttachmentPrincipal`，`ActorAttachment`保存principal；
takeover验证调用principal与prepared attachment owner一致。新增bounded
`detach_remote_principal_until(device_id, deadline)`：枚举live/provisional SessionActor，发送
actor command删除matching attachments并释放matching controller lease，收集impact/error。

它不得：close Session、interrupt child、给PTY发signal、删除TerminalModel、影响local/其他
remote attachment。没有M7 remote adapter时，通过直接remote principal integration test证明。

## 11. Same-UID IPC、status 与 doctor

复用现有Unix peer credential、strict unary EOF和唯一FrameDecoder：

- local pair create/accept/device list/rename/revoke新增到`DaemonService`；
- pair accept/normal confirmation是async-native branch；同步Store/Session work继续
  `spawn_blocking`，不能把network future塞进blocking thread或再建runtime；
- refactor first-frame dispatch为borrow metadata后move唯一frame，不能沿用会复制敏感payload的
  generic `frame.clone()`；sensitive request/reply bytes用zeroizing owner并在write/decode后清理，
  仍复用同一个FrameDecoder；
- 每个local request沿用一个absolute deadline。started pairing operation放入bounded cell继续
  到terminal result；waiter timeout只丢waiter，retry加入原cell；
- `LocalPairingClient`/`LocalDeviceClient`只作为daemon-internal/test-facing real socket adapter，
  不导出public CLI commands。

扩展现有typed `DaemonStatus`：network state、Endpoint bind/home Relay、publish/lookup摘要、
active primary/path counts。`doctor` running时只读IPC observation；stopped时只检查committed
config/state，不bind Endpoint、不query DNS、不spawn daemon。Human/JSON继续由同一typed status
投影，路径不作为authorization/session/replay truth。

## 12. Secret、日志与内存

- `DeviceIdentity`新增daemon-internal secret ownership API，但CLI crate无依赖/访问；
- pair secret/ticket/proof使用redacted type，owned decode/encode buffers尽快zeroize；
- tracing禁止使用`?request`/`?ticket`/raw proto Debug；只记录short EndpointId、方向、
  generation、result category、ALPN和redacted path kind；
- terminal bytes/input/cwd与direct IP不进入auth/transport日志；
- SQLite只保留device auth、alias/name、relay-only verified cache；没有offer、secret、proof、
  transcript、session、attachment或audit/event table；
- tests用明确标注的固定非生产secret发布cross-language golden；任何runtime随机ticket不得
  进入snapshot/failure output。

## 13. 验证设计

### 13.1 Pure/domain/proto

- `pairing_vectors`：ticket bounds、prefix/base64、canonical bytes、HMAC/confirmation golden、
  expiry、tamper、wrong ID/secret、zeroizing/redacted formatting；
- `compatibility`/pair wire：kind唯一、proto round-trip、unknown fields/capability、ID/string/list
  bounds、unknown cache version；
- pure duplicate reducer、DeviceAlias/default alias、checked generation、TransportLimits。

### 13.2 Store/auth/session

- persistence tests证明schema仍v1/无新表、authorize/re-authorize/revoke generation、idempotent
  revoke、transaction failure无状态变化、known/inbound方向合并与alias reservation；
- authorization tests证明unknown/revoked在business frame前拒绝、stream/commit generation
  recheck、writer fairness；
- revoke barrier test显式观察old commit已进入、revoke等待、commit完成、DB commit、watch
  cancel和queued/new commit拒绝，不能用短sleep推断；
- remote principal attachment/takeover/revoke只detach目标，Session/PTY/其他principal继续。

### 13.3 Real Iroh/local IPC

- task-private两daemon/两identity使用localhost direct candidate跑normal/pair ALPN、singleflight、
  multiple independent streams、response loss recovery；常规CI不依赖public Internet；
- duplicate test用barrier同时双向dial，双方最终报告同一candidate key，loser close不影响winner；
- stream limit/stall/malformed test证明一peer/stream不阻塞另一peer、local status或PTY drain；
- pair test覆盖expiry/tamper/replay/concurrent consumers/DB failure/accepted response drop；A授权B
  后A不能控制B，反向ticket后才可以；
- local device IPC通过真实same-UID socket覆盖所有新kind、strict EOF、deadline与secret日志
  sentinel；public CLI help保持没有pair/device command；
- network lifecycle在DNS/Relay全部不可用时仍通过local readiness/Session，Endpoint状态degraded
  且stop有界；identity不旋转。

### 13.4 Relay/path evidence

- 扩展`iroh_profile_gate`精确断言两个ALPN、official map/QAD/production lookup与self-hosted隔离；
- 继承已验收的 Foundation Case C 作为 official n0 运行时证据：两端在bind前阻断非DNS UDP，
  仍通过官方 WSS/TCP Relay 完成三条端到端加密双向stream；不在本任务重复外网实验；
- Linux CI在`RelayMode::Disabled`的loopback Endpoint上运行当前`connection_broker`、
  `two_daemon_transport`与两进程production PairingService gate，证明本任务新增的broker、ALPN、
  pairing与authorization组合使用真实Iroh；这些测试不冒充public n0运行时证据；
- route/path pure tests验证fresh/cache/ticket顺序、远端Relay candidate不修改configured map，且
  direct/relay observation不改变Session/auth/generation/replay状态；
- 可选`relay.zenithconsulting.cn`及其他self-hosted部署不构成M5-M6验收门槛。真实双网络自动
  address discovery仍按Foundation结论留给父任务M10。

### 13.5 Platform/gates

- macOS arm64/Intel与Linux x86_64/arm64运行core/proto/daemon与真实Unix IPC tests；
- Windows运行shared core/proto/daemon lib compile/tests，所有Unix private import/field/helper完整
  cfg-gate；不声明Endpoint daemon/Named Pipe已支持；
- source-policy、fmt、Clippy `-D warnings`、workspace tests/docs、cargo-deny、secret sentinel通过。

## 14. 明确不采用

- 不用账号、JWT、长期pair certificate、第二套公钥或central control plane；
- 不把route/Relay/DNS/ticket当授权，不持久direct IP；
- 不为远端复用local Unix socket，不让local self target self-dial Iroh；
- 不为M7预建Session RPC registry、event stream、terminal stream或reconnect UI；
- 不把public pair/device CLI拆到本任务“顺手实现”；
- 不用connection lifetime read permit（会让revoke等待自己关闭的connection而死锁）；
- 不在SQLite commit前先close connection，不在DB失败后假装revoked；
- 不通过修改configured RelayMap解决跨profile route，不隐式fallback staging/public/self-host；
- 不用无界task/mpsc、wall-clock sleep或retry掩盖duplicate/revoke竞态。

## 15. Downstream handoff

M7只需：

1. 持有`ConnectionDemand`并在每条remote stream取得`AuthorizedCommitContext`；
2. 把既有200+/300+ protobuf转换到同一个SessionService；
3. `prepare_attach(RemoteEndpoint { id, generation }, ...)`；
4. connection loss只drop attachment，reconnect后重新attach/snapshot。

M8只需：

1. 把本任务same-UID pair/device IPC接到公开command tree；
2. ticket默认hidden TTY、自动化显式stdin且永不argv；
3. 把directional DeviceProjection渲染为明确human/JSON与revoke confirmation；
4. 不读取identity、SQLite或自行bind Endpoint。
