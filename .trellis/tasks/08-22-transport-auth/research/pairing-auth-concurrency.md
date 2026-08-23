# Pairing、authorization 与 revoke 并发模型

## 密码学与依赖选择

Cargo.lock 已包含 `ring 0.17.14`、`base64 0.22.1` 与 `zeroize 1.9.0`。实现可把这三个精确
版本提升为 direct workspace dependency，使用：

- `ring::rand::SystemRandom` 生成 128-bit offer ID、256-bit ticket secret 和 256-bit nonce；
- `ring::hmac::HMAC_SHA256` 生成/constant-time verify proof；
- `base64::URL_SAFE_NO_PAD` 编解码 protobuf ticket；
- `zeroize::Zeroizing` 与显式 buffer zeroize 缩短 ticket/secret/proof 的内存生命周期。

不需要新增 `rand`、`hmac`、JWT、PAKE、账号或证书体系。Iroh TLS 已证明双方长期
EndpointId；ticket secret 只证明一次性宿主意图。

## Ticket 与 canonical bytes

建议固定：

- 文本前缀 `zterm-pair-v1:`；总文本上限 16 KiB；base64url 无 padding；
- `format_version=1`、32-byte host EndpointId、1–128 UTF-8 byte host name、最多 4 条
  HTTPS Relay URL（每条最多 2048 bytes、无重复）、16-byte offer ID、32-byte secret、
  `expires_at_unix`；
- 默认 TTL 10 分钟，可选 1–60 分钟；最多 16 个 live offer；host 同时用 `Instant`
  deadline 防止系统时钟回拨延长 offer。

protobuf 只负责兼容 envelope，不作为 canonical transcript。`canonical_ticket_v1` 使用固定
domain tag、big-endian integer、fixed-width ID 和 length-prefixed UTF-8/URL 列表。Host用
Iroh `RelayUrl::to_string()`发出URL；controller验证HTTPS/长度但canonicalization保留ticket中
原始UTF-8 bytes，不做跨语言可能不同的二次normalize。authorization state只保存：

```text
ticket_digest = SHA256(canonical_ticket_v1_without_secret)
offer_key = HMAC-SHA256(pair_secret,
    "zterm-pair-offer-key-v1\0" || canonical_ticket_v1_without_secret)
```

secret-bearing Rust type 不实现可泄密 Debug/Display；错误只返回类别。

`LocalPairCreate` 是会产生 bearer capability 的 mutation。为了让同一 operation ID 在本地
response 丢失后返回 byte-identical ticket，PairingManager 另保留一个最多 16 项、随
consume/expiry 清除的 `Zeroizing<String>` replay result；它是唯一允许短期保留完整 ticket
的内存位置。该值不进入 SQLite、日志、错误、status 或 snapshot。

## Pair ALPN 状态机

1. controller 解码/验证 ticket，用 ticket route 连接 ticket host 的 `zterm-pair/1`；TLS
   remote EndpointId 必须精确等于 ticket host。
2. controller 发送 `PairBegin(offer_id, controller_name, controller_nonce)`；host 回应随机
   `PairChallenge(host_nonce, negotiated_version)`。
3. transcript 固定绑定 ticket digest、双方 TLS EndpointId、offer ID、双方 nonce、controller
   name、format/protocol version 和 expiry。controller 发送 domain-separated HMAC proof。
4. host constant-time verify 后才执行内存 CAS `Ready -> Consuming(controller_id,
   transcript_digest)`；并发第二消费者失败。随后由 StoreActor transaction authorize 并
   checked increment generation。
5. DB 失败则在未过期时恢复 Ready；DB commit 成功才转 `Consumed`，清除 verifier，并发送
   generation-bound host confirmation HMAC。Consumed tombstone 保留到 expiry，不能授权第二
   EndpointId。
6. controller 验证 confirmation 后写本机 `known_devices`。若 accepted response 丢失，它以
   同一 ticket route 尝试普通 `zterm/1`：host 已提交则 handshake 成功并补写 known device；
   host 未提交则明确失败/结果未知。恢复不能重新开放 offer。

invalid proof 在 CAS 前失败，不消费票据；valid concurrent proof 只有 CAS winner 能触碰
SQLite。Pair ALPN 有独立 global/per-endpoint semaphore、first-frame/total deadline 与 64 KiB
总字节预算，避免 unauthenticated peer 占住 daemon。

## 单向授权与设备投影

- `device_auth` 是入站权限：remote 是否能控制本机；pair host 写 controller。
- `known_devices` 是出站地址簿：本机是否知道如何连接 remote；pair controller 写 host。
- list 按 EndpointId 合并两个方向，但不把二者折叠成一个“paired=true”真相。
- rename 只改 `known_devices.local_alias`；revoke 只改 `device_auth`。同一 EndpointId 可以
  同时存在两个方向，revoke 不删除 route/alias，也不暗示远端同步撤销。
- alias 由 core 值对象验证 1–128 UTF-8 bytes、无首尾空白/控制字符、唯一且不能为 `local`。
  未显式提供时优先 remote name；冲突/保留值时追加稳定 short EndpointId。

## AuthorizationGate

daemon 启动时从 StoreActor 预载全部 authorization snapshot。每个 EndpointId 有一个
fair Tokio owned `RwLock` 和 state watch：

- connection/stream admission 读取 `(status, generation)` 并订阅 cancellation；
- 真正副作用提交取得 expected-generation owned read permit，并把 permit 移入
  `spawn_blocking` closure，直到 SessionService/PTY side effect 返回；
- authorize/revoke 取得 write permit，所以等待已开始的 commit，且 writer 排队后阻止新
  commit 越过它。

revoke 固定顺序：write permit → StoreActor FULL-synchronous transaction 写 tombstone 并
checked increment → 更新内存状态/watch → 关闭该 EndpointId 全部 connection/stream →
`SessionService` 按 remote principal 移除 attachment/controller → release。DB 失败时内存、
connection 与 attachment 不变；already-revoked 返回当前 generation，不重复递增。

`SessionService::prepare_attach` 必须接收/保存 principal，takeover 校验 attachment owner；
新增 detach-remote-principal command 只回收 attachment/controller lease，永不 close Session、
发送 PTY signal 或影响其他 principal。

## Connection duplicate key

每个 dialer 生成 128-bit attempt ID；connection key 是
`(initiator_endpoint_id_bytes, attempt_id_bytes)`。Iroh connection side 验证 initiator，双方按
同一 lexicographic order 选择 primary。per-Endpoint singleflight 防止正常路径同侧多拨；
短暂 inbound/outbound 或 stale reconnect 重复只关闭 loser，不传播 Session close。Pair ALPN
不进入该 registry。

对一向授权，只有被授权 controller 主动拨 host；互相授权且同时有 demand 时两个 normal
connection 才会竞态，deterministic key 让双方收敛。每个入站 service stream 仍按接收方
本地 `device_auth` 检查，因此共享一条 QUIC connection 不会把单向授权意外变双向。
