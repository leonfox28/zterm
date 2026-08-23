# Iroh Transport 与设备认证实施计划

## 0. Execution rules

- 本任务只实现父任务 M5–M6；严格停在 M7 remote Session adapter 与 M8 public pair/device
  CLI 之前。
- 用户批准本规划后才运行 `task.py start`；开始编码前重新运行 `trellis-before-dev`，读取
  task context 与适用 spec。
- 每一步先通过 focused gate，再进入下一步。失败修 owning boundary，不添加无界 retry、
  sleep、全局锁或第二套 decoder/registry 掩盖。
- 所有 daemon/identity/socket/database/relay fixture 使用 task-private TempDir/UserPaths；
  测试不得触碰真实 `~/.zterm`，普通 CI 不依赖 public Internet。
- SecretKey/ticket/secret/proof 不写进 command output、tracing、panic、snapshot 或 fixture
  failure；测试错误只打印 redacted sentinel/category。
- 保留用户已有 worktree 改动；每步检查 diff，不重写不相关文件。

## Step 0. Baseline、spec 与依赖

### Work

- [x] 记录 clean task baseline：git status、source policy、workspace version、fmt、Clippy、
  workspace tests/docs、cargo-deny。
- [x] 用 `trellis-update-spec` 新增并索引 active backend transport/auth contract；同步扩展
  core-wire、effective-user-state、local-daemon-ipc、session-service 中真正改变的签名/边界，
  不把未实现愿望写成 current contract。
- [x] Workspace 提升 Cargo.lock 已有的 exact direct dependencies：
  `ring = =0.17.14`、`base64 = =0.22.1`、`zeroize = =1.9.0`；按实际 owner只加入
  core/proto/daemon crate。
- [x] 不新增 rand/hmac/JWT/gRPC/connection pool/async SQLite/runtime/CLI prompt dependency。
- [x] 先运行现有 `iroh_profile_gate`，记录 official/self-hosted baseline；不在依赖阶段改
  production constants、staging 隔离或 identity bytes。

### Gate

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
python3 .trellis/scripts/task.py validate .trellis/tasks/08-22-transport-auth
```

Stop if a dependency changes MSRV/license/provider behavior, duplicates a pinned transitive
primitive without a current consumer, or requires exposing secret material outside daemon/core.

## Step 1. Core ticket、alias、authorization 与 limits

### Work

- [x] Add `DeviceAlias`, PairOfferId/PairNonce/redacted PairSecret, RelayHint,
  PairTicketFields, ConnectionAttemptId/CandidateKey, AuthGeneration/Snapshot 与
  TransportLimits in transport-neutral core modules。
- [x] Implement exact validation：UTF-8 byte bounds、reserved `local`、HTTPS Relay hints、
  counts/sizes/TTL、checked generation、nonzero resource limits。
- [x] Implement deterministic default alias projection with UTF-8-safe truncation + short
  EndpointId suffix；explicit conflict仍由DeviceDirectory/SQLite判定。
- [x] Implement canonical ticket/transcript bytes、SHA-256、domain-separated HMAC proof/
  confirmation与constant-time verify；secret-bearing types redacted and zeroized。
- [x] Extend DomainErrorKind stable code round-trip with only design.md listed categories。
- [x] Implement pure deterministic candidate winner reducer；connection runtime只消费该owner。

### Tests

- [x] `pairing_vectors` golden fixtures可由非Rust实现复现；覆盖prefix-independent canonical
  bytes、wrong length/order/duplicate route、tamper、wrong ID/secret、expiry/version。
- [x] Alias empty/too-long/whitespace/control/reserved/conflict-input/default suffix/case semantics。
- [x] AuthGeneration i64/u64 ceiling、idempotent representation、TransportLimits零值/边界。
- [x] Candidate order在initiator/attempt相等前缀、反序注册和随机corpus中确定。
- [x] Debug/Display/redacted error corpus不包含固定secret sentinel。

### Gate

```sh
cargo fmt --all -- --check
cargo clippy -p zterm-core --all-targets --all-features -- -D warnings
cargo test -p zterm-core --all-features
cargo test -p zterm-core --test pairing_vectors
cargo doc -p zterm-core --no-deps
```

Stop if core needs prost/Iroh/SQLite/CLI/OS APIs or if protobuf serialization order becomes the
canonical authentication input.

## Step 2. Product protobuf、kind registry 与 ticket text adapter

### Work

- [x] Replace placeholder pairing messages with PairTicketV1/Begin/Challenge/Proof/Accepted；
  add transport Hello/Welcome and local pair/device messages per design kind table。
- [x] Keep one `WireKind` numeric registry and one generated v1 module surface；reject duplicate/
  missing kinds and retain future capability bits。
- [x] Add domain↔proto validators for every ID/name/alias/route/version/generation before allocation
  or persistence。
- [x] Implement `zterm-pair-v1:` + base64url-no-pad adapter with 16 KiB pre-decode limit and
  zeroized decoded/secret buffers；never implement ticket formatting in CLI/daemon twice。
- [x] Add RelayRouteCacheV1 bytes/version adapter containing Relay URLs only；unknown versions are
  ignored with typed diagnostic。
- [x] Retain existing frame/control bounds and incremental decoder；add 16 KiB pair/hello and
  64 KiB handshake accounting above it。

### Tests

- [x] Golden ticket text/protobuf round-trip matches Step 1 canonical vectors。
- [x] Unknown optional fields and unknown capability bits survive supported round-trip；unknown
  kind/version fails at documented boundary。
- [x] Prefix/padding/alphabet/truncation/oversize/invalid UTF-8/ID/URL/count/generation corpus。
- [x] Kind 1–21、100–105、200+、300+ unique and centrally mapped。

### Gate

```sh
cargo fmt --all -- --check
cargo clippy -p zterm-proto --all-targets --all-features -- -D warnings
cargo test -p zterm-proto --all-features
cargo test -p zterm-proto --test compatibility
cargo doc -p zterm-proto --no-deps
```

Stop if a second frame decoder appears, raw prost structs enter core, or a secret-bearing generated
message is logged/formatted by a service path.

## Step 3. StoreActor、DeviceDirectory 与 AuthorizationRegistry

### Work

- [x] Keep SQLite `user_version=1` and exact table inventory；add typed StateStore reads/updates for
  all device_auth/known_devices fields without schema/table expansion。
- [x] Replace saturating generation with checked i64-compatible increment；authorize always advances,
  first revoke advances/writes tombstone, repeated revoke returns current generation unchanged。
- [x] Make StoreActor expose cloneable StoreHandle while sole owner retains thread join/shutdown；
  replace unbounded mpsc with capacity-64 sync channel + deadline/started gate，and add
  list/get/auth/revoke/last-seen/known-route/rename commands。
- [x] Run all network-runtime StoreHandle waits through `spawn_blocking` with one absolute deadline；
  never share rusqlite Connection or hold a Tokio/global registry lock across SQLite wait。
- [x] Add DeviceDirectory directional merge, exact-ID rename/revoke lookup, default/explicit alias
  validation, in-memory alias reservations shared by pair accept and rename。
- [x] Preload AuthorizationRegistry from store；per EndpointId fair owned RwLock + watch，short outer
  map lock，expected-generation commit guard与generic remote error projection。

### Tests

- [x] Existing migration/schema inventory remains byte/semantic compatible and excluded tables absent。
- [x] authorize→reauthorize→revoke→revoke→reauthorize exact generation/status/timestamps。
- [x] Generation exhaustion and injected transaction failure do not wrap/partially mutate。
- [x] Inbound/outbound rows merge without conflation；rename never changes auth，revoke never deletes
  known route/alias。
- [x] Concurrent alias reservation/rename produces one winner before network work；SQLite unique is
  final crash-safe owner。
- [x] StoreActor concurrent callers remain responsive and shutdown joins once。

### Gate

```sh
cargo test -p zterm-daemon --test persistence
cargo test -p zterm-daemon --test authorization
cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings
cargo doc -p zterm-daemon --no-deps
```

Stop if schema version/table count changes, a DB error closes live transport, or runtime code opens a
second SQLite connection while StoreActor is live.

## Step 4. Session principal ownership 与 revoke detach hook

### Work

- [x] Change SessionService `prepare_attach(_until)` to require AttachmentPrincipal and carry it in
  SessionCommand/ActorAttachment/PreparedAttachment ownership。
- [x] Validate takeover principal matches prepared attachment owner；preserve existing replay principal
  and response-loss continuation semantics。
- [x] Add bounded `detach_remote_principal_until` that concurrently notifies every owned SessionActor,
  removes matching remote attachments/controller lease, and reports exact impact/errors。
- [x] Keep local same-UID attach call sites explicit；no adapter may infer local/remote from target name。
- [x] Never invoke actor/session close、driver interrupt、PTY signal、model removal or resource release
  for principal detach。

### Tests

- [x] Remote principal detach removes active/prepared takeover/controller across multiple sessions；
  local and another remote principal continue。
- [x] Detached principal stale input/resize/takeover fails lease checks；SessionId、child PID、PTY output
  and registry entry remain。
- [x] Concurrent natural exit/explicit detach/revoke is idempotent and does not remove a replacement。
- [x] Existing M4 local_session_ipc/controller/session concurrency corpus stays green。

### Gate

```sh
cargo test -p zterm-daemon --test controller_lease
cargo test -p zterm-daemon --test session_lifecycle
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-daemon session
cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings
```

Stop on any revoke/principal path that ends a Session/PTY or weakens local same-UID controller rules.

## Step 5. Endpoint supervisor、profile 与 lifecycle composition

### Work

- [x] Add `ZTERM_PAIR_ALPN` to exact profile builder/summary；keep official production and explicit
  self-hosted map unmodified。
- [x] Expose daemon-internal DeviceIdentity secret ownership without changing redacted Debug or CLI
  dependency boundary。
- [x] Compose sole Endpoint/network supervisor into existing daemon Tokio runtime outside owned-listener
  recovery loop；local readiness remains independent from network online。
- [x] Implement network state watch：initializing/bound/degraded/online/stopping，home Relay、publish/
  lookup摘要；Endpoint bind failure retries 250 ms→10 s+jitter with stable identity。
- [x] Add ALPN accept router and bounded global/per-endpoint pre-auth permits；pair/normal paths separate。
- [x] Implement final network quiesce/connection close/Endpoint close under lifecycle absolute deadline；
  preserve response-flush and exact-token socket removal order。

### Tests

- [x] Profile summary/builder exact four official QAD relays + production lookups + two ALPNs；self-hosted
  exactly one no-QAD relay；staging env cannot alter either。
- [ ] Fully offline DNS/Relay/Internet and injected Endpoint bind failure still serve local readiness,
  status、Session create/attach/stop，with truthful degraded observation and unchanged identity。
- [ ] Fatal local listener rebind retains the same network/store/session owner；final stop closes
  Endpoint once and removes only exact owned socket。
- [x] No inspection command binds Endpoint、queries network or spawns daemon when stopped。

### Gate

```sh
cargo test -p zterm-daemon --test iroh_profile_gate
cargo test -p zterm-daemon --test network_lifecycle
cargo test -p zterm-daemon --test local_ipc
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-cli
```

Stop if daemon local readiness waits `Endpoint::online()`, profile code calls staging-sensitive shortcut,
or network cleanup short-circuits retained Session ownership.

## Step 6. RouteResolver 与 ConnectionBroker

### Work

- [x] Implement fresh signed lookup with 2 s budget and ordered cache/transient-ticket fallback；build
  relay-only EndpointAddr candidates without `insert_relay` or direct-IP persistence。Host emits
  `RelayUrl::to_string()`；ticket canonical bytes retain the exact validated URL text without a
  controller-side normalization rewrite。
- [x] Implement per-Endpoint PeerSlot、global dial budget、singleflight waiters、RAII demand、bounded
  open-stream queue and error-aware reconnect while demand exists。
- [x] Implement Hello/Welcome on normal ALPN with remote Iroh ID check、wire/capability negotiation、
  device/build/platform diagnostics and receiver-side auth generation。
- [x] Reject unknown/revoked inbound normal connection before reading application frame；outbound target
  comes only from known device or pair transient context。
- [x] Implement CandidateKey provisional/register/promote/loser close actor；business streams wait for
  promotion，duplicate loser does not redial while a peer candidate remains；never hold global map
  across dial/handshake or close Session on duplicate/connection/path loss。
- [x] Set transport/application stream limits；each accepted stream uses existing decoder, independent
  task/deadline/semaphore and current AuthContext。M7 kinds return service_not_implemented after auth。
- [x] Observe connection close/path events into typed status；persist only handshake-verified Relay route
  and last_seen，never direct IP。

### Tests

- [x] `connection_broker`: concurrent local demands/open streams one dial/primary；different peers isolate。
- [x] `duplicate_connection`: barrier-driven simultaneous inbound/outbound dial produces same winner on
  both endpoints，loser does not kill winner/demand。
- [x] `stream_limits`: per/global overflow、stalled first frame、malformed/oversize stream affect only
  offender while another peer、local status and PTY drain progress。
- [ ] `authorization`: unknown/revoked inbound rejected before Hello payload read；one-way connection does
  not permit reverse service stream。
- [x] Route lookup success/failure/cache/ticket/unknown cache version；dynamic remote relay leaves
  configured profile summary byte-for-byte unchanged。
- [x] Path direct/relay observation changes no Session/auth/generation/replay state。

### Gate

```sh
cargo test -p zterm-daemon --test connection_broker
cargo test -p zterm-daemon --test duplicate_connection
cargo test -p zterm-daemon --test stream_limits
cargo test -p zterm-daemon --test authorization
cargo test -p zterm-daemon --test path_migration
cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings
```

Stop if route fallback mutates home RelayMap, a peer can consume unbounded tasks/bytes, or connection
health becomes Session lifetime truth.

## Step 7. PairingManager、pair ALPN 与 normal confirmation

### Work

- [x] Implement bounded offer manager and local create operation cells：Ready/Consuming/Consumed/
  Expired，monotonic+wall expiry，exact Zeroizing ticket response replay，consume cleanup。
- [x] Generate SystemRandom offer/secret/nonces；validate TLS host/controller ID and run exact
  challenge/proof/confirmation transcript through ring HMAC verify。
- [x] CAS only after valid proof；StoreActor authorize inside Consuming；DB failure rolls back Ready if
  live，commit publishes Consumed before response。
- [x] Implement pair accept alias reservation、transient route、pair handshake、normal zterm/1
  confirmation and local known-device/route commit。
- [x] Implement ambiguous PairAccepted recovery and repeated local operation join；after generic
  pairing rejection a repeated accept may only use normal auth as same-controller confirmation，never
  reopen the ticket for a second EndpointId or issue remote rollback。
- [x] Enforce 8 global/1 endpoint pair permits、5 s first frame、15 s total、64 KiB transcript traffic、
  generic peer errors and secret-free tracing。

### Tests

- [x] `pairing_protocol`: A ticket/B accept yields only A-authorizes-B and B-known-A；A cannot call B。
- [x] Reverse ticket independently authorizes opposite direction/generation/revoke。
- [x] Expired/consumed/tampered/wrong host/wrong controller proof/replay/concurrent second consumer fail
  without second DB authorize。
- [x] Invalid proof leaves Ready；injected StoreActor failure leaves Ready/no authorization；generation
  exhaustion terminally fails without consume。
- [x] Drop PairAccepted response after host commit；normal confirmation repairs B known device exactly
  once。Drop local pair-create/accept response；same operation bytes replay exact result。
- [x] Pair connection never appears as normal primary and no 0-RTT API is used。
- [x] Secret sentinel absent from logs、errors、status、SQLite bytes and panic output。

### Gate

```sh
cargo test -p zterm-core --test pairing_vectors
cargo test -p zterm-daemon --test pairing_protocol
cargo test -p zterm-daemon --test authorization
cargo test -p zterm-daemon --test pairing_secrets
```

Stop if pair success can authorize two EndpointIds, controller writes reverse device_auth, or local
partial failure is hidden as success.

## Step 8. Device IPC、revoke transaction 与 diagnostics

### Work

- [x] Add async-native pair dispatch and blocking device dispatch behind the existing same-UID peer gate,
  strict unary EOF and one frame decoder；add daemon-internal/test LocalPairing/DeviceClient methods。
- [x] Remove the generic first-frame clone on sensitive requests：borrow kind/request metadata，then move
  one decoded frame；zeroize pair request/reply buffers after decode/write without adding a second codec。
- [x] Implement device list/rename/revoke with exact DeviceId、directional projection、byte-identical
  safe local retry；do not add public clap pair/device commands。
- [x] Implement AuthorizationGate revoke order：write permit → DB commit → memory/watch cancel → broker
  close remote → SessionService detach remote principal → response。
- [x] Hold expected-generation read permit through synthetic/Session side-effect commit via
  spawn_blocking；queued operations cannot overtake waiting revoke writer。
- [x] Extend typed status/doctor observation and existing human/JSON projections；redact direct IP、route
  cache and secrets，keep stopped doctor network-passive。
- [x] Keep CLI help/no-argument milestone text honest：public pair/device仍不可用，M8才暴露。

### Tests

- [ ] Real same-UID `local_device_ipc` covers kinds 12–21、strict EOF、deadline、oversize ticket、
  alias conflicts and direction fields；wrong UID仍zero response bytes before decode。
- [x] `revoke_races` barrier proves started old commit completes before DB revoke，queued/new commit fails
  after commit，all old connections/streams/controllers close，restart remains revoked。
- [x] Inject DB revoke failure：connection/stream/attachment/generation unchanged and retry succeeds。
- [x] Revoke one controller leaves Session/PTY、other authorized device、outbound known entry intact；
  repeated revoke exact idempotent response。
- [x] Existing status/doctor human and JSON share one typed projection and have no side effects。
- [x] `zterm --help` contains no pair/device/connect/session surface beyond current milestone。

### Gate

```sh
cargo test -p zterm-daemon --test local_device_ipc
cargo test -p zterm-daemon --test revoke_races
cargo test -p zterm-daemon --test local_ipc
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-cli
sh tests/core-local-daemon/cross-uid.sh
```

Stop if DB failure changes live access, revoke closes PTY/Session, same-UID IPC logs ticket, or public CLI
scope drifts into M8.

## Step 9. Real two-daemon、Relay/path 与 platform evidence

### Work

- [x] Build one task-private multi-process two-daemon harness using explicit UserPaths/identities and
  localhost direct candidates；prove daemon-owned Endpoint复用，不添加production state override argv。
  The production `PairingService` self-spawn gate compiles on macOS but remains ignored before bind；
  its real pair/normal execution passed on Linux x86_64 and arm64 in CI run
  `32608814512`。It composes the production
  PairingService/BrokerPairTransport owners through private lib-test access；it does not claim
  `run_daemon`/`NetworkStartup` lifecycle evidence。
- [ ] Reuse existing disposable self-hosted Relay/handshake fixture to prove ticket/cache remote Relay URL
  can dial across profile without configured map insertion；do not make ordinary tests depend public N0。
- [x] Reuse/extend ignored network gate only for explicit Relay/direct path evidence；record path events，
  不重跑Foundation无关部署/benchmark或用它替代deterministic concurrency tests。
- [x] Audit cfg boundaries on Windows shared build；private Unix imports/fields/helpers整体gate。
- [x] Run secret/dependency/source-policy checks against generated fixtures and logs。

### Gate

```sh
cargo test -p zterm-daemon --test two_daemon_transport
cargo test -p zterm-daemon --test path_migration
sh tests/relay/static.sh
sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn
sh tests/source-policy.sh
cargo test -p zterm-core --all-features
cargo test -p zterm-proto --all-features
cargo test -p zterm-daemon --lib --all-features
```

The disposable Relay command may be an explicit environment gate rather than every local run, but the
task cannot claim route/profile acceptance without one recorded pass. Stop if a harness touches real
user state or treats unavailable public Internet as a product-code retry requirement.

## Step 10. Full quality gate 与 handoff

### Work

- [x] Run `trellis-check` for spec compliance、cross-layer data/error flow、reuse、security、platform
  cfg、lint/type/test consistency；fix all task-owned findings。
- [x] Verify diff contains no M7 Session RPC adapter、terminal stream/reconnect UX、M8 clap commands、
  mobile/GUI/event/history/ACL/account/control-plane code。
- [x] Validate every PRD acceptance item against a named automated/manual evidence artifact；remaining
  external environment gate must be explicit, not silently skipped。
- [ ] Update parent M5–M6 checklist/progress only after all task gates pass；do not mark M7/M8。
- [ ] Run task validation and prepare finish-work/archive only after implementation approval scope is
  genuinely complete。

### Final gate

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
python3 .trellis/scripts/task.py validate .trellis/tasks/08-22-transport-auth
git diff --check
```

Required hosted evidence：macOS arm64/Intel、Linux x86_64/arm64 full applicable jobs；Windows shared
core/proto/daemon compile/tests。Only after these gates and review may `trellis-finish-work` archive the
child and record M5–M6 complete。
