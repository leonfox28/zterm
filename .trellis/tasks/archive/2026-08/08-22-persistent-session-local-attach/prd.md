# 持久 Session、PTY 与本地 attach

## 目标

完成 Phase 1 M4：让每用户 daemon 在自身生命周期内持有多个真正独立的终端
Session。用户断开本地或未来远端 attachment 后，PTY、login shell、前台程序和
权威终端状态继续运行；稍后通过 same-UID 本地 socket attach 时，回到同一个
SessionId、进程、工作目录和当前画面。

这里的“持久”只指 attachment/网络断开不结束 Session。daemon 停止、升级或宿主
重启会结束所有 PTY；1.0 不恢复已经死亡的进程。

## 已确认的产品边界

- daemon 第一次按默认入口 attach 时原子创建 `main`；以后默认入口回到仍存活的
  同名 Session。`main` 是保留名称和稳定默认入口。
- 用户可另建、列出、重命名、attach、detach、takeover 和关闭多个命名 Session；
  SessionId 在一次 daemon 生命周期内稳定，rename 不改变身份。
- 每个 Session 使用当前 OS 账户配置的交互式 login shell。默认 cwd 是账户 home，
  只允许显式且可访问的 `--cwd`，不接受任意启动命令。
- 无 attachment、无 controller、无输入或无输出都不会自动关闭 Session。前台程序
  退出回到根 Shell 也不会关闭；只有根 Shell 退出、显式 close 或 daemon stop 才结束。
- 1.0 每个 Session 只有一个 controller。普通第二次 attach 返回 occupied；显式
  takeover 才能转移控制权，旧 controller 立即失效。内部结构保留未来 observer，
  但本任务不开放 observer 产品能力。
- 本机 attach 使用 same-UID Unix socket，直接调用唯一 SessionService；不配对、
  不解析 device alias、不建立到自己 EndpointId 的 Iroh 连接。
- tmux、Herdr 以及其他复用器只是 PTY 内的普通黑盒程序；产品路径禁止按进程名分支。

## 功能需求

### R1. Session 生命周期与身份

1. daemon 内维护唯一 SessionRegistry；按 SessionId 和唯一名称访问同一对象。
2. 并发首次 attach `main` 最终只创建一个 Session。创建失败不能留下名称、资源额度
   或半初始化 actor。
   create 的 provisional name slot 与 rename uniqueness 必须在同一个原子 registry 边界；
   SessionId 碰撞检查与 resource insertion 也必须在固定 `state -> resources` lock order 下原子
   完成，所有 name/resource/actor owner 使用同一不可伪造 token compare-remove；
   spawn 后 actor 到 publication/cleanup 完成前一直是 registry-visible provisional owner。
   publication 丢失必须 close/reap child；cancel 只禁止 publish，原 Starting name 在实际 cleanup
   完成前仍不可复用；cleanup 失败保留可重试 ownership，不得隐形释放。
3. 普通名称必须有界、非空、无控制字符且大小写敏感。`main` 不能被普通 create 或
   rename 占用，也不能被 rename；显式 close `main` 后，下次默认 attach 创建新的
   `main` 和新的 SessionId。
4. root Shell 自然退出时给所有 attachment 一个有类型的结束原因，原子移除 registry
   项并释放资源；显式 close 只终止目标 Session。
5. daemon stop 在退出前显式关闭全部 Session，并报告受影响数量；daemon restart
   得到空 registry，持久身份和授权不受影响。

### R2. PTY 与权威终端状态

1. 每个 Session 只有一个 owner，持有唯一 PTY、TerminalDriver/TerminalModel、revision、
   attachment 状态、controller lease 和资源 reservation。
2. PTY reader 从创建到 root Shell 退出持续排空，与 attachment 数量无关。慢或断开的
   client 不得阻塞 PTY reader，也不得产生无界输出队列。
   每 Session 的 blocking PTY write/flush/resize/finalize 必须隔离在独立有界 ordered
   execution owner；Session A 阻塞不得占用 daemon current-thread runtime 或阻塞 Session B。
3. 所有 PTY bytes 先按顺序进入权威 TerminalModel；客户端只接收带 Revision 的最新
   snapshot 或合并 delta，不维护第二套服务端终端真相。
4. main/alternate screen、Unicode、颜色、光标、输入模式、连续 resize、DA/DSR/CPR
   回复和安全控制序列边界继续遵守 Foundation 已验证的 TerminalModel 契约。

### R3. Attach、同步与控制权

1. attach 成功后先发送权威 snapshot。服务端在收到精确匹配的
   `SnapshotApplied(revision)` 前拒绝 input/resize；同步期间的普通输入不排队。
2. revision gap、尺寸变化、未来 revision 或 delta 不再划算时发送完整 resync；客户端
   永远可以丢弃旧状态并从最新 snapshot 恢复。
3. attachment disconnect/detach 只释放该 attachment 和它持有的 lease，不给 PTY 发
   信号，不关闭 Session。
4. takeover 必须先为新 attachment 准备 snapshot，再原子递增 lease generation 并转移
   controller；旧 generation 的 input/resize 一律失败，不能发生双写。
5. create、rename、close、takeover 使用有界 OperationId 结果窗口。响应丢失后的相同
   操作返回原结果，不重复创建、改名、杀进程或转移 lease；已淘汰结果返回
   `operation_outcome_unknown`。
   global registry 只短暂注册，same ID + 完整语义 fingerprint 用 per-operation singleflight，
   不同 payload 拒绝，unrelated key 并行；executor panic/drop 必须以 outcome unknown 唤醒 waiter。
   lease 由 daemon 按 stable principal/auth generation 签发，包含随机 daemon incarnation 与单调
   ordinal。invented/high/stale/restart-mismatched lease 在任何副作用前拒绝；lost allocation 产生的
   empty lease 也必须有界回收，只回收 completed prefix，in-flight result 不被回收。ordinal 与
   sequence 穷尽显式报错，不能 wrap。
6. takeover 响应丢失后，新的已同步 pending attachment 必须能用同一 opaque operation token
   继续：controller 为空或仍由该 operation 标记时取得输入权；不得覆盖后来 operation 的 controller。

### R4. 本地 IPC adapter

1. 保留 M3 的 peer-credential same-UID gate、frame/控制载荷上限、deadline 和单实例
   daemon 契约。
2. 同一 socket 协议支持两类连接：短 unary session control RPC，以及一个 attachment
   对应一个长生命周期双向 frame stream。首 frame 决定模式，只有一个增量 decoder
   拥有边界验证。
3. unary 连接仍必须严格只有一个请求并 half-close；attachment stream 只接受该状态下
   合法的 input/resize/snapshot-applied/sync/detach/takeover 消息。
   同步 service/attachment 调用必须通过 blocking worker，并携带 absolute deadline；queued
   command deadline 到期不得开始副作用，started mutation 在 caller disconnect 后继续完成。
   LocalClient 在第一次 mutation 前 lazy 获取并缓存 daemon lease；readiness/status/list 不分配
   lease 或写 replay state。ambiguous transport 最多重试一次，复用 byte-identical request 和同一
   OperationId；typed OutcomeUnknown 不自动换 lease 重试，只让后续独立用户操作重新申请。
4. 此任务提供真实 socket 级本地 attachment client/test adapter，但不实现 M8 的 raw
   TTY UI、`Ctrl+]` detach 前缀或最终 CLI 命令树。

### R5. 有界资源与安全

1. 使用 Foundation 已固定的默认值：最多 8 个 live Session、每个 2,000 行近期历史、
   无有效初始 viewport 时 120x40、最大 viewport 240x80、全部 Session 固定 cell projection
   合计不超过 128 MiB；256 MiB 是进程 RSS 测量目标，不是假装可精确分配的硬限额。
   controller detach 后保留最后有效尺寸，不自动 resize。
2. create 和 resize 必须在 PTY/model 分配或变更前同时检查 session 数、viewport、
   scrollback 和全局 projection。失败不改变原状态或 reservation。
3. snapshot/delta 必须符合 8 MiB frame 上限；必要时只裁掉 snapshot 中最旧的完整历史
   行，绝不裁当前可见 screen、破坏 ANSI 边界或改变宿主 TerminalModel。
4. attachment 通知、actor command、连接数和 operation replay 均有界。任何慢 client
   只能合并到最新 revision 或被要求 resync，不能积累逐 revision 队列。
   checkpoint 只保留 zero-scrollback main/alternate visible grids；1.0 每 Session 最多一个
   controller 和一个 pending takeover。
5. Session 运行态、PTY、attachment、lease 和 operation replay 只在内存中；不得写入
   SQLite、日志或磁盘 transcript。
6. daemon stop 并发 interrupt 全部 Session 并在 absolute deadline 内等待 ownership release；
   任一 child/driver/actor/reservation 未释放时不得宣称 stopped，listener/socket 保持 status/
   retry 能力。shutdown 必须先向全部 live/provisional owner 发 close，再只忽略普通 ended
   SessionNotFound summary race 并上报其他 typed error；recoverable accept error 不结束 listener，
   fatal serve exit 的 cleanup 失败不得 unlink socket 或遗弃 child，而应保留 daemon lock/process 并
   compare-rebind 自己发布的 socket identity，恢复 status/stop retry。

## 验收标准

- [x] 8 路并发首次本地 attach 只创建一个 `main`；至少一个成功，所有成功响应返回同一
      SessionId，和 controller/pending 重叠的请求可返回 occupied；detach 后任务继续，重连
      取得相同 SessionId、cwd、当前 screen 和有界近期历史。
- [x] 可以同时创建 `main` 加至少两个命名 Session；list/rename/attach/detach/close 只
      影响目标对象，名称冲突、保留名和无效 cwd 不留下半成品。
- [x] 无 attachment 的 Session 仍持续排空至少 1 MiB 输出并完成长任务；前台程序退出
      后回到同一根 Shell，Session 不被自动回收。
- [x] snapshot + delta/resync 在 main/alternate、Unicode、颜色、光标、模式和连续 resize
      后重放得到与宿主相同的 TerminalState；同步确认之前 input/resize 没有副作用。
- [x] 普通第二 attach 返回 occupied；显式 takeover 后旧 attachment 收到 LeaseLost，
      陈旧 lease 无法输入或 resize，新 controller 能继续同一 PTY，且没有双写窗口。
- [x] 对 create/rename/close/takeover 注入“已提交但响应丢失”，重试返回完全相同结果，
      不产生第二次副作用；同 ID 不同 payload 拒绝；takeover 新流取得输入权；窗口淘汰、
      daemon restart/invented lease 返回 outcome unknown，panic waiter 不悬挂。
- [x] session 数、viewport 和 128 MiB projection 的边界及越界均有测试；失败前后 registry、
      PTY、TerminalModel 和 reservation 保持一致，压力测试不 OOM。
- [x] same-UID 真实 socket 完成 unary 与双向 attachment；错误 kind、超限 frame、deadline、
      trailing unary frame、attachment 非法状态只终止对应连接，daemon 与其他 Session 正常。
- [x] close 一个 Session 不影响其他 Session；daemon stop 明确结束全部 Session；自然 root
      Shell 退出、显式 close 和 daemon stop 的结束原因可区分。
- [x] tmux 与固定 Herdr 通过同一个通用本地 attachment harness 完成交互、Unicode、颜色、
      alternate screen、bracketed paste、resize、detach 后存活和 snapshot 恢复；生产源码
      不含程序名特判。
- [x] macOS arm64/Intel、Linux x86_64/arm64 的托管 CI 运行适用测试；Windows 保持公共
      domain/proto/daemon unsupported boundary 可编译，不伪装已支持 ConPTY/Named Pipe。

## 不在本任务范围

- Iroh Endpoint、远端认证、配对、QUIC stream、连接复用与 NAT/relay 路径（M5–M7）。
- 最终交互式 CLI renderer、raw mode、detach 前缀和设备命令树（M8）。
- Windows ConPTY/Named Pipe、Android/iOS/GUI、observer、多端同时控制。
- daemon/宿主重启后的进程恢复、自动启动、自动更新、任意启动命令、专有 Agent 状态识别。
- transcript 持久化、搜索、无限历史或 alternate-screen 内部应用历史抓取。
