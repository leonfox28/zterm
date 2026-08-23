# M5–M6 当前代码边界

## 结论

本任务应实现 daemon-owned Iroh Endpoint、connection broker、一次性配对、入站设备授权、
known-device 地址簿、本地设备管理 IPC 与立即撤销；不能提前实现 M7 远端 Session 协议，
也不能提前新增 M8 public pair/device CLI command tree。

## 已存在的 owner

- `crates/daemon/src/transport.rs` 已从 `presets::Minimal` 构建精确 Iroh 1.0.3 profile：
  official 使用 production default Relay/QAD，self-hosted 使用单条
  `RelayConfig::new(url, None)`；production Pkarr publisher/resolver、DNS lookup、
  relay-only publication 与 portmapper 已集中在该 adapter。当前只注册 `zterm/1`，只返回
  `Builder`，没有 bind 或 accept loop。
- `crates/daemon/src/identity.rs` 的 `DeviceIdentity` 已安全加载/创建 32-byte Iroh
  SecretKey，Debug 只显示 public DeviceId；缺少只供 daemon endpoint composition 使用的
  secret ownership transfer/clone API。
- `crates/daemon/src/lifecycle.rs:176-211` 当前依次打开 committed store、校验 setup、绑定
  same-UID Unix listener、启动 `StoreActor` 和 `DaemonService`；Iroh Endpoint 尚未进入同一
  Tokio runtime。`run_owned_daemon_listener` 的 fatal-listener recovery 会在 Session cleanup
  失败时保留 daemon lock、store、service 与 PTY owner 并 exact-token rebind，网络 owner
  也必须跨该 recovery loop 存活。
- `crates/daemon/src/store.rs` 的 schema v1 已含 `device_auth` 与 `known_devices`，不需要
  migration/new table。`StateStore` 已有 authorize/revoke/status/upsert primitive，但
  generation 使用 `saturating_add`，revoke 非幂等，且运行时 `StoreActor` 只暴露 metadata。
- `crates/daemon/src/session.rs` 已用 `AttachmentPrincipal` 区分 local/remote mutation replay，
  但 `prepare_attach` 与 `ActorAttachment` 尚未保留 principal。因此 revoke 目前无法按
  EndpointId 移除 controller/attachment；应增加 transport-independent principal ownership
  与 bounded detach hook，不能关闭 Session 或 PTY。
- `crates/daemon/src/local_ipc.rs` 只有一个 peer-UID gate、frame decoder 和 strict unary/
  duplex classifier；普通 service dispatch 经 `spawn_blocking`。Pair accept 是 async network
  operation，应在同一 decoder 后增加 async-native dispatch 分支，而不是创建第二套 IPC。
- `proto/zterm/v1/pairing.proto` 与 kind 100/101 只是占位；local kinds 1–11、session 200+、
  terminal 300+ 已占用。M5–M6 可以在 local 12–21 与 transport/pairing 100–105 范围内
  明确分配，继续由 `zterm-proto` 唯一 registry 验证。

## 必须保持的既有契约

- local readiness、status、stop 和 local Session 不依赖 Iroh online、DNS/Pkarr、Relay 或
  Internet；network readiness 必须作为独立 observation，不能阻断 self target。
- daemon/StoreActor/SessionService 各自仍是唯一 owner；CLI 不读取 key、不打开 SQLite、
  不绑定 Endpoint，也不复制 registry/decoder。
- connection、stream、path 或 revoke 不进入 Session close/PTY signal 路径。
- official 与 self-hosted profile 不混合，staging 环境变量不能改变 production constants。
- macOS/Linux 是本任务真实 runtime evidence；Windows 只保持 shared core/proto/daemon
  unsupported boundary 可编译。

## 当前任务与后续任务的接口

本任务提供：

- daemon-internal `ConnectionBroker`/authenticated stream admission API；
- same-UID local pair/device IPC 与真实 socket test adapter；
- `SessionService::prepare_attach(principal, ...)` 和按 remote principal detach hook；
- status/doctor 的 network/device projection。

M7 才把 session list/create/attach/input/resize/takeover 编到 remote QUIC streams；M8 才把
pair/device IPC 投影为 TTY prompt、stdin/argv 规则、human/JSON 与 destructive confirmation。

## 相关约束来源

- `.trellis/spec/backend/relay-deployment.md`
- `.trellis/spec/backend/core-wire-domain.md`
- `.trellis/spec/backend/effective-user-state.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/session-service.md`
- 父任务 `design.md` 第 6–9 节与 `implement.md` M5–M8
