# 多 session 的 Iroh 连接复用

## 结论

zterm 应采用“每个本地设备身份到每个远端设备身份一条活动 Iroh 连接，多 session 使用独立 QUIC stream”的模型。连接是可重建的传输容器，session 是 daemon 持有的持久资源，attachment 是两者之间一次短生命周期的绑定。

```text
控制设备的 daemon / App
  └─ Iroh Connection（一次寻址、打洞、认证、路径选择）
       ├─ control RPC streams
       ├─ generic session event stream
       ├─ attachment stream: main
       ├─ attachment stream: build
       └─ attachment stream: review

远端 daemon
  ├─ session main
  ├─ session build
  └─ session review
```

一个控制设备连接多个宿主时，每个宿主各有一条连接。不同已授权控制设备连接同一宿主时，也各自拥有连接；不能跨不同设备身份共享已经认证的 transport。

## Iroh 0.96.1 证据

本机 Cargo registry 中 `iroh-0.96.1/src/endpoint/connection.rs` 将 `Connection` 明确定义为 QUIC connection，并公开：

- `open_bi` / `accept_bi`：双向 stream；
- `open_uni` / `accept_uni`：单向 stream；
- `set_max_concurrent_bi_streams` / `set_max_concurrent_uni_streams`：并发 stream 上限；
- 文档说明 stream 通常创建成本低且即时，除非受到 flow control 限制。

因此无需为不同 session 重复建立 Iroh connection，也不应在单个字节流中重新实现一套容易产生队头阻塞的全局 multiplex framing。

## Zedra 证据

当前参考仓库使用 Iroh `0.96` 与 `irpc-iroh 0.12`：

- `zedra-session/src/connect.rs` 建立一个 `iroh::endpoint::Connection`，再用 `IrohRemoteConnection::new(conn)` 创建整个 RPC client。
- `zedra-host/src/rpc_daemon.rs` 的 connection handler 在循环中调用 `conn.accept_bi()`，每个请求使用一条双向 stream，并为每条消息 spawn 独立 dispatch task。
- 源码注释明确要求单个 stream 的解码失败不能关闭 connection，否则会破坏同一 QUIC connection 上其他 in-flight RPC。

这证明“一条 Iroh connection 承载多个并发 RPC/stream”已经是参考项目验证过的基础模式。zterm 可以借鉴 transport 形状，但 session、权威 VT、snapshot 和多 attachment 语义仍需按自身需求设计。

## 桌面 CLI 如何真正共享连接

同一终端模拟器中的多个 CLI 是多个操作系统进程；如果每个进程直接创建 Iroh endpoint，它们仍会形成多条连接。zterm 已确定每个 OS 用户运行一个本地 daemon，因此桌面端应让 daemon 同时充当控制侧连接 broker：

1. daemon 独占当前用户的设备私钥和 Iroh endpoint；
2. 按远端 device ID 缓存活动 connection；
3. CLI/GUI 通过权限受限的用户级本地 IPC 请求 list/create/attach/close；
4. daemon 在已有 connection 上为每个本地视图打开 attachment stream；
5. 最后一个本地视图退出后可以关闭空闲 transport，但不能影响远端 session。

Android/iOS 没有独立 daemon，单个 App 进程中的 connection manager 承担相同职责。App 被系统杀死时 transport 消失，远端 session 不受影响；App 重启后只建立一条新 connection，再恢复需要的 attachment。

## Stream 与会话协议边界

- 连接完成设备级认证后，每个新 stream 仍携带协议种类、版本、session ID、attachment ID 与请求能力；宿主对每个操作再次检查该设备的授权和 session controller lease。
- 长生命周期 terminal attachment 可以使用一条双向 stream 承载输入、resize、ack、snapshot 和增量；短生命周期 list/create/close 等控制 RPC 使用各自的双向 stream。具体 framing 在 `design.md` 中确定。
- controller lease 属于 `(session_id, attachment_id)`，不能属于整个 connection，否则一个设备无法在同一连接上同时控制不同 session。
- attachment 或 connection 关闭都不能进入 PTY 终止路径。

## 隔离与背压

QUIC stream 提供独立的可靠有序字节流，但所有 stream 仍共享连接级拥塞控制和底层带宽。zterm 不能因为使用多 stream 就假设资源隔离自动完成：

- 每个 attachment 使用独立、有界发送队列；
- daemon 的 PTY reader 永不等待某个客户端队列；
- 落后客户端丢弃中间增量并请求最新 snapshot；
- 控制 RPC 与输入/resize 不排在无界 terminal output 后面；
- 限制每连接 stream、attachment 和排队字节数；
- 测试一个高输出 session 时另一个交互 session 及控制 RPC 的延迟。

## 后台 tab 的建议

GUI 或移动端无需为列表中的每个 session 永久打开 terminal stream。建议默认：

- 一个轻量 connection-level event stream 更新 session 创建/关闭、controller 状态、根进程状态和“有新输出”的通用 revision，不解析 Agent 内容；
- 当前前台 tab 使用完整 attachment stream；
- 后台 tab 暂停终端增量，保留客户端最后画面，并在重新前台时用权威 snapshot + watermark 无缝恢复；
- 桌面端未来可以把少量最近 tab 保持 warm 作为可选优化，不能成为协议正确性的前提。

这种策略利用了宿主权威 VT 的核心优势，显著降低 Android 的电量、流量、内存和后台生命周期压力。代价是切回后台 tab 时需要一次 snapshot，切换延迟略高于所有 tab 永久在线订阅。

切换冷 tab 时不应显示空白页，也不应重新执行 Iroh 寻址、打洞、TLS/设备认证或 relay 选择。建议交互顺序为：

1. 客户端立即显示该 session 上次保存于内存的渲染画面，并标记为 `syncing`；该缓存只用于视觉连续性，不是权威状态。
2. connection manager 在现有 Iroh connection 上打开 attachment stream，携带 session ID、客户端视口尺寸和已知 revision。
3. daemon 先返回当前 viewport 的权威 VT snapshot 与 watermark，再从 watermark 后继续增量。
4. 客户端原子替换缓存画面并清除 `syncing`，此后才启用键盘、粘贴和鼠标输入。同步期间不缓存用户输入，避免用户根据旧提示符输入后被发送到已经变化的审批框或 Shell。

初始 snapshot 不应无条件携带整个内存 scrollback；其关键路径只包含当前主/备用屏幕、光标、mode、样式及恢复渲染所需的小范围上下文。更早的有界 scrollback 在用户向上滚动时分页读取，否则一个数 MiB 的历史会把普通 tab 切换变成大传输。

在连接已经建立的前提下，该冷切换主要承担一次应用往返和一个当前 viewport snapshot 的传输成本，具体可见延迟取决于 direct/relay 路径、移动网络 RTT、屏幕尺寸和编码大小。协议仍应允许客户端按资源预算保留少量 warm tab：桌面端可保留最近使用的若干 tab，Android 在 Wi-Fi/充电状态也可预热相邻 tab；这只能是体验优化，不能成为正确恢复的前提。
