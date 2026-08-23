# Iroh 1.0.3 runtime API 核验

## 核验范围

直接检查 workspace 锁定的 `iroh = 1.0.3` crate source，而不是依赖浮动版本文档。对应源码
位于 `$CARGO_HOME/registry/src/.../iroh-1.0.3/`。

## Endpoint 与连接 API

- `Endpoint::builder(...).bind().await` 完成本地 bind；`Endpoint::accept()` 接收入站；
  `Endpoint::connect(EndpointAddr, alpn)` 完成 1-RTT connection。不能调用
  `Connecting::into_0rtt`，因为 pairing/auth 和未来 mutation 均不可承受 replay。
- `Connection::remote_id()` 是 Iroh TLS 已认证的 remote EndpointId；`Connection::alpn()`、
  `open_bi()`、`accept_bi()`、`set_max_concurrent_bi_streams()` 可分别完成 ALPN dispatch、
  独立双向 stream 与 transport-level admission。
- `Connection::paths()`/`path_events()` 提供诊断路径；`Connection::closed()` 与
  `Connection::close()` 管 connection actor。`Endpoint::close().await` 是最终 bounded
  shutdown 的 owner；path watcher 自身不能决定 Session 或 authorization 状态。
- `Endpoint::watch_addr()` 可独立观察当前 home Relay/address；`Endpoint::online()` 只表示
  曾接触 Relay，不应成为 local daemon readiness 的前置条件。

## 地址查询与 cache 优先级

- `Endpoint::address_lookup()?.resolve(endpoint_id)` 并发合并所有 configured lookup service，
  逐项返回成功和单服务错误。broker 可以在自身 deadline 内收集第一批 relay-only result，
  因而实现“fresh signed lookup 优先”。
- `Endpoint::connect` 在传入 `EndpointAddr` 已含 Relay URL 时不会再要求 lookup。要实现明确
  的 fresh→cache/ticket fallback，broker 必须先显式 resolve，再按候选逐次 connect；不能把
  fallback 顺序交给隐式 connect 行为。
- 只有完成 Iroh EndpointId authentication 和 zterm application handshake 后，候选 Relay
  才能写入 versioned cache；direct socket address 永不持久化。

## 跨 profile route hint

核验 `src/socket/transports/relay.rs` 与 `src/socket/transports/relay/actor.rs`：

- relay sender 对 `(RelayUrl, EndpointId)` 不要求 URL 已存在于 configured RelayMap；
  `active_relay_handle_for_endpoint` 会按本次目标 URL 动态创建 relay actor。
- 动态 relay actor 只在 URL 存在于 configured map 时取得可选 auth token；无 token 的
  official/self-hosted公开 Relay route 可按 `EndpointAddr` 直接使用。
- 该动态拨号不调用 `Endpoint::insert_relay`，不会进入 home relay selection、publication
  或 profile summary。因此 official map 仍精确是 production default，self-hosted map 仍
  精确一条无 QAD Relay；ticket/cache route 只属于远端候选。

这消除了“跨 profile 配对必须混合两张 Relay map”的顾虑。实现必须用 route candidate
adapter，而不是临时修改 endpoint configured map。

## Runtime composition 结论

- Endpoint bind 本身不要求 Relay/DNS online；daemon 可先发布 local readiness，并把
  network 状态显示为 initializing/degraded/online。外网不可用不能阻止 local Session。
- accept loop、broker、pairing manager、path observation 与 local IPC 应运行在现有 daemon
  Tokio runtime。network owner 必须跨 local listener fatal-rebind loop 存活，最终 stop 在
  session ownership 释放后依次取消 streams/connections、close Endpoint、再移除 owned socket。
- Pairing 使用独立 `zterm-pair/1` ALPN 和独立并发预算，成功后不把该 connection 升级为
  `zterm/1` primary；normal confirmation 必须新建/复用 normal ALPN connection。

## 未采用的方案

- 不使用 `Endpoint::insert_relay` 解决远端 route hint：它会改变 effective RelayMap，破坏
  profile 隔离且没有必要。
- 不等待 `Endpoint::online()` 才接受 local IPC：会回归 M4 的完全离线 self-attach 契约。
- 不把 direct path/IP 写入 SQLite：它短命、可能包含局域网敏感元数据，也不是授权真相。
