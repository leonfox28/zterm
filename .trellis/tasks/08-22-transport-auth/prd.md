# Iroh Transport 与设备认证

## 目标

完成 Phase 1 M5–M6：让每用户 daemon 绑定其长期 Iroh 身份，作为桌面端唯一的
Endpoint 和 connection broker；用户可以通过一次性文本票据建立 SSH-like 单向设备
授权，并在本机查看、命名和撤销设备。该任务交付一个经过认证、受资源约束、可供 M7
远端 Session 协议直接复用的 transport/auth foundation。

用户价值是：无需账号、业务控制平面或云端授权服务，两台已 setup 的 macOS/Linux
设备即可安全互认；relay、DNS/Pkarr 或网络路径只能帮助建连，不能授予终端权限。

## 已确认事实

- 父任务把 M5–M6 映射为一个独立的 `transport-auth` child；M4 已完成 daemon-lifetime
  Session、same-UID 本地 attach 与 controller lease，M7 才接入远端 Session RPC。
- Phase 1 产品默认使用固定 Iroh 1.0.3 的官方 N0 production Relay/QAD 与 production
  DNS/Pkarr；显式 self-hosted profile 是一张独立、无 QAD 的 Relay-only map，不能与默认
  map 混合（`.trellis/spec/backend/relay-deployment.md`）。
- 当前 `transport.rs` 已能构造 official/self-hosted endpoint builder，但只注册
  `zterm/1`；daemon lifecycle 尚未 bind Endpoint（`crates/daemon/src/transport.rs:14`、
  `crates/daemon/src/lifecycle.rs:193`）。
- SQLite v1 已包含 `device_auth` tombstone/generation 和 `known_devices` route cache，且
  `StateStore` 已有基础 authorize/revoke/upsert 方法；运行时 `StoreActor` 目前只开放
  metadata 查询（`crates/daemon/src/store.rs:140`、`crates/daemon/src/store.rs:281`）。
- 现有 `pairing.proto` 只是 M2 占位消息，尚未表达完整 ticket、challenge/proof、结果与
  device management；wire kind registry 只有 100/101 两个占位 kind
  （`proto/zterm/v1/pairing.proto:7`、`proto/zterm/v1/wire.proto:18`）。
- 父设计已确定 M8 的最终 CLI：`zterm pair create/accept` 与
  `zterm device list/rename/revoke`。本任务只交付 daemon 侧 same-UID local IPC 契约和
  daemon-internal/test client adapter；TTY prompt、human/JSON 呈现、argv/stdin 与交互确认
  仍由 M8 统一实现（父 `implement.md:352`、`design.md:444`）。
- 配对授权方向、卸载边界与撤销行为已由用户在父规划中确认，不在本任务重新决策。

## 功能需求

### R1. Daemon-owned Iroh Endpoint

1. 每用户 daemon 从已经提交的 `identity.key` 加载唯一 SecretKey，绑定一个 Iroh
   Endpoint；CLI 不读取私钥、不另建 Endpoint，也不为每个命令重复绑定网络资源。
2. Endpoint 同时注册正常协议 `zterm/1` 和配对协议 `zterm-pair/1`。禁止 0-RTT/
   0.5-RTT 承载认证、配对或有副作用的业务请求。
3. official profile 精确保持 Iroh 1.0.3 production map、QAD、production DNS/Pkarr 和
   relay-only publication；self-hosted profile 只含用户显式配置的一条 HTTPS Relay，
   QAD 关闭且不隐式回退到 official/staging Relay。
4. local readiness、same-UID Session 与 daemon stop 不等待 home Relay、DNS/Pkarr 或
   Internet online；Endpoint 初始化/失败作为独立的 network readiness 状态可诊断。网络
   子系统失败不得阻止本机 Session 使用，也不得留下虚假的 network-ready 状态。
   stop/fatal cleanup 有界关闭所有 connection 与 Endpoint，但不能改变身份、授权或已知
   设备状态。
5. status/doctor 暴露有效 profile、Endpoint/home Relay、publish/lookup 状态和当前 path
   摘要；路径标签只用于诊断，不能参与 session、authorization 或重放正确性。

### R2. Connection broker 与 stream admission

1. `ConnectionRegistry` 以 remote EndpointId 为 key；同一设备对的并发本地调用共享一个
   singleflight dial，并最终指定一条 primary connection。不同 EndpointId 永不共享
   authenticated transport。
2. inbound/outbound 重复竞态用双方都能计算的 EndpointId/initiator 与随机 attempt ID
   排序收敛；loser 只关闭自身，不能误杀 winner，也不能终止任何 Session/PTY。
3. `zterm/1` connection 必须先完成一次 version/capability/device handshake；宿主在读取
   任意业务请求前按 Iroh-authenticated EndpointId 查询 `device_auth=authorized`，并取得
   `(endpoint_id, generation, cancellation)` AuthLease。每条新 stream 和每个敏感提交点
   再验证 generation。
4. 短 control RPC 和后续 terminal attachment 使用独立双向 QUIC stream。保留未来
   `DEVICE_EVENTS` capability/kind 的兼容边界，但本任务不实现长期 event stream、terminal
   stream 或 M7 Session RPC adapter。
5. connection、每 connection stream、全局/每 EndpointId 未认证握手、并发 RPC、首帧
   deadline、frame bytes 与排队 bytes 都有明确非零上限。畸形、超时或恶意 stream 只回收
   自身，不阻塞其他 connection、SessionActor、PTY reader 或 local IPC listener。
6. 成功 authenticated zterm handshake 后才更新 `last_seen` 与 versioned home Relay route
   cache；临时 direct IP 不持久化。新鲜签名 lookup 优先，DNS/Pkarr 不可用时可使用有效
   ticket/cache Relay route；远端 route hint 只用于本次拨号，不插入或改变本机 configured
   home Relay map。均不可用时给出寻址错误，不旋转身份、不删除配对。
7. connection 断开、direct/relay 切换或 broker loser cleanup 不进入 Session close 路径；
   本任务提供 M7 可调用的受认证 stream/broker API，但不承诺最终 remote reconnect UX。

### R3. 一次性文本配对

1. `PairTicketV1` 是版本化 protobuf，经固定文本前缀 + base64url（无 padding）编码；包含
   host EndpointId、host display name、当时可用的 home Relay hints、128-bit offer ID、
   256-bit随机 secret、到期时间和格式版本。未来二维码原样承载同一文本。
2. 默认 TTL 为 10 分钟；显式 TTL 必须在有界范围内。daemon 只在内存中保留少量 offer
   的 verifier/必要状态，重启即失效；完整 secret、ticket 与 offer 不写入 SQLite 或日志。
3. `zterm-pair/1` 使用 Iroh 已认证的双方 EndpointId，并以 HMAC-SHA256 证明 ticket secret
   持有。canonical transcript 绑定双方 EndpointId、offer ID、随机 challenge/nonce、格式与
   协议版本、到期时间，字段编码不可歧义。
4. PairingManager 将 offer 原子执行 `Ready -> Consuming -> Consumed`。过期、已消费、
   篡改、错误 secret、错误 host EndpointId、重放和并发第二消费者均失败，且不会留下
   半授权；成功响应丢失不能允许同一 ticket 授权另一个 EndpointId。
5. 配对严格是 `host -> controller` 单向授权：host 事务性 authorize controller 并递增
   generation；controller 只把 host 写入 `known_devices`/route cache。反向控制必须另用一张
   ticket；两方向 generation/revoke 独立。
6. same-UID local IPC 以受限 payload 把 ticket 交给 daemon，且服务端与测试适配器不得把
   ticket、secret 或 proof 写入日志、错误或持久存储。M8 才实现默认不回显的 TTY prompt、
   显式 `--stdin` 以及不接受 argv ticket 的最终 CLI 边界。

### R4. 本地设备管理与撤销

1. same-UID local IPC 与 daemon-internal/test adapter 提供授权方向清晰的 device list、
   alias rename 与 revoke；列表按 EndpointId 合并 `known_devices`（本机可连接对端）和
   `device_auth`（对端可控制本机）两种独立方向，并显示状态/generation、paired/last-seen、
   在线/attachment 摘要，但不泄露 secret 或完整敏感路由。
2. rename 只修改本机 `known_devices.local_alias`；revoke 只修改本机
   `device_auth` 入站授权。同一 EndpointId 可同时存在两个方向，撤销入站权限不得删除
   出站 address-book 记录。alias 有界、唯一，保留 selector `local` 不可使用；M8 负责把
   这些明确方向与影响投影为 CLI 文案和交互确认。
3. 每个 EndpointId 使用 AuthorizationGate：业务提交持 read permit，revoke 持 write
   permit。revoke 顺序固定为：SQLite FULL transaction 写 revoked tombstone 并递增
   generation；发布新内存 generation/拒绝新操作；取消并关闭该设备全部新旧 connection/
   stream；通过已有 SessionService 移除它的 attachment/controller lease。
4. 数据库提交失败不得关闭连接或假装成功；提交成功后，即使响应、连接或 daemon 随后
   失败，重启后旧授权也不能复活。提交前已经完成的 PTY write 无法撤销；提交后任何旧
   generation 或排队请求都不能再到达副作用提交点。
5. revoke 永不 close Session、给 PTY 发信号或影响其他已授权设备；重新授权同一
   EndpointId 必须重新消费一张新 ticket，并取得更高 generation。

### R5. 跨层兼容、安全与可观察性

1. `zterm-core` 只拥有 transport-neutral ticket/auth/connection domain 值、验证规则与资源
   defaults；Iroh、prost、SQLite、CLI 和 OS 依赖留在对应 adapter crate。
2. `proto/zterm/v1/*.proto` 是 wire source of truth；新增 kind 在唯一 registry 中分配。
   v1 只做兼容字段新增，unknown capability bits 保留；不兼容语义使用新 ALPN major。
3. 复用现有 `varint length + WireFrame`、8 MiB frame 和 1 MiB control payload 边界；
   ticket/handshake/device 字段另有更小的产品上限，并在分配或持久化前验证。
4. Runtime SecretKey、pair secret、完整 ticket、HMAC、terminal bytes/input/cwd 不进入
   tracing、SQLite 审计表或测试快照；允许记录脱敏 EndpointId、授权方向、generation、
   结果类别和 path 元数据。跨语言 golden vector 只使用明确标注的固定非生产凭据。第一阶段
   不新增持久 audit/event 表。
5. macOS/Linux 提供真实两 daemon、多进程/多 stream 与撤销竞态证据；Windows 保持公共
   core/proto/daemon unsupported boundary 可编译，不宣称已有网络 daemon/Named Pipe。

## 验收标准

- [x] daemon 使用已提交身份绑定一个含 `zterm/1` 与 `zterm-pair/1` 的 Endpoint；official
      与 self-hosted profile 的有效 Relay/QAD/DNS/Pkarr map、staging隔离、relay-only publication
      和有界 shutdown 均有自动化证据。
- [x] 同一 device pair 的并发 dial、多 local client 和多个预留业务 stream 最终只使用一条
      primary connection；inbound/outbound duplicate race 确定性收敛且 loser 不影响 winner。
- [x] connection/stream/handshake/frame/queue 上限和首帧 deadline 被逐一越界验证；恶意或
      stalled peer 不阻塞正常 peer、local IPC、SessionService 或 PTY drain。
- [x] A 创建 ticket、B 安全导入后只产生 `A authorizes B`；B 被保存为 A 的 authorized
      device，A 只成为 B 的 known device。没有反向 ticket 时 A 不获得控制 B 的权限。
- [x] ticket 编解码与 canonical transcript golden vectors 可供其他语言独立复现；过期、
      篡改、错误 EndpointId/secret、重放、并发消费与成功响应丢失都不能让 host 产生第二次
      授权或半提交。controller 本地提交失败必须返回 outcome unknown 并可通过 normal
      confirmation 修复，不能谎报成功；runtime ticket/secret 不落盘、不进入日志/错误，且
      本任务不提供可把 ticket 放进 argv 的 public 入口。
- [x] 未授权/revoked EndpointId 即使能连公开 Relay 也在任何业务 frame 前被拒绝；普通
      connection 和每个敏感 stream/RPC 均绑定当前 authorization generation。
- [x] revoke 提交成功返回后，旧 connection、所有 stream、排队 RPC、竞态 reconnect 和
      controller lease 都失效；daemon restart 后仍 revoked。同一 Session/PTY 及其他设备继续。
- [x] DNS/Pkarr 失败时，有效 ticket 或已验证 cache Relay 可寻址；无任何路由时明确失败且
      不改变身份、授权、known device 或 live Session。direct/relay path 变化只更新诊断。
- [x] same-UID local IPC 与真实 daemon-internal/test adapter 覆盖 pair create/accept、device
      list/rename/revoke、方向投影、alias 与错误边界；不提前新增 M8 public CLI command tree、
      TTY prompt 或 remote terminal UI。
- [x] format、Clippy、workspace tests、dependency/secret checks及适用的 macOS arm64/Intel、
      Linux x86_64/arm64 CI 通过；Windows shared crates 保持编译边界。

## 不在本任务范围

- M7 远端 Session list/create/attach/rename/close/takeover、terminal stream、snapshot/delta 与
  reconnect 后 attachment 恢复。
- M8 public pair/device/session/connect CLI command tree、TTY/stdin/argv 安全入口、human/JSON
  呈现与确认，以及最终 raw-mode terminal renderer、`connect` UX 和控制前缀。
- Android/iOS/GUI、Windows Endpoint daemon/Named Pipe/ConPTY、observer、多写者、
  `DEVICE_EVENTS` 或 history paging 实现。
- per-session ACL、访客/分享/低信任角色、账号/业务 API/中央撤销服务、卸载前
  `RevokeSelf`、持久 pairing offer 或 audit/event 表。
- 改写/扩展 relay 数据平面、自动 fallback 到 staging/optional self-hosted Relay、
  QAD-only 新服务、生产 SLA 或新基础设施 profile。

## 技术研究结论

- ticket 固定为 `zterm-pair-v1:` + base64url-no-pad protobuf；认证使用独立、固定大端长度前缀
  canonical bytes 和 HMAC-SHA256 golden vectors。
- Iroh 1.0.3 支持 daemon-owned bind/accept/path/close，以及不修改 configured RelayMap 的
  动态远端 Relay URL 拨号；fresh signed lookup 由 broker 显式先行。
- per-EndpointId broker actor、deterministic candidate key、fair AuthorizationGate、sole
  StoreActor 与 Session remote-principal detach hook分别拥有连接/授权/持久化/attachment边界。
- 初始资源上限和确定性 barrier/fault-injection harness 已在 `design.md`/`implement.md` 固定；
  详细证据位于本任务 `research/`。
