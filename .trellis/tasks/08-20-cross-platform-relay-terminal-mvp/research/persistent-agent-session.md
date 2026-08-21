# 持久 AI Agent 会话模型

## 产品定位

zterm 的核心对象不是一次网络连接，而是宿主上的持久 PTY 会话。Codex、OpenCode 等交互式 AI Agent 可能运行数小时，期间控制端会经历锁屏、休眠、移动网络切换、进程退出和 relay/direct 路径变化。上述事件都不能等价于关闭远程 Shell。

这一模型更接近由产品内建并远程化的 tmux 会话，而不是传统的“连接存在则 Shell 存在”的 SSH 默认体验：

```text
宿主 daemon
  └─ 持久会话（PTY + Shell + AI Agent）
       ├─ 控制连接 A：attach → disconnect
       └─ 控制连接 B：稍后 reattach
```

## 必要语义

- **会话与连接分离**：网络 transport 断开只删除 attach 关系，不关闭 PTY master。
- **稳定会话身份**：会话具有独立 ID 和用户可读名称，不能用某条 QUIC stream 充当会话身份。
- **有界重放**：宿主记录单调递增的输出序号和有界 backlog；客户端携带最后确认序号重新接入。
- **终端恢复**：重连时至少交付近期输出并同步当前尺寸；对全屏 TUI 还需在设计阶段研究快照、重绘或可恢复终端状态，不能假设任意一段原始字节都足以重建屏幕。
- **明确终止**：进程自然退出或经过授权的显式 close 才能回收会话；transport 错误不能进入终止路径。
- **本地资源策略**：输出和会话元数据仅保存在宿主本地，并设置容量、空闲和回收边界。

## 架构影响

- daemon 可以直接持有 PTY，但不能让处理单条远程连接的任务拥有 PTY 生命周期；连接任务结束只能 detach，daemon 本身退出则允许终止会话。
- RPC/协议需要把 `create/list/attach/detach/resize/close` 作为不同操作，并为 attach 输出定义序号、确认和溢出行为。
- Android/iOS 进入后台不应主动关闭远程会话；回到前台时应探测连接并 reattach。
- 授权撤销、宿主退出和普通网络断开需要使用不同的结束原因，便于客户端解释会话为何结束。

## 已确认的 daemon 与更新边界

- 首版不引入独立于 daemon 的 session supervisor；这避免额外的本地 IPC、进程监管和跨版本恢复协议。
- daemon 停止、崩溃、重启、升级或宿主重启时，活动 PTY 可以结束。
- 产品不自动更新。更新由用户手动触发，检测到活动会话时默认停止并展示影响，只有明确确认或非交互式强制参数才能继续。
- 这一取舍不削弱核心承诺：只要宿主 daemon 仍在运行，控制端断开和公网路径变化就不能终止会话。

## 单控制端与未来多端

1.0 每个会话只授予一个 attachment 输入和 resize 权限。新设备普通 attach 不影响现有控制端；显式 takeover 原子地转移控制权，随后以确定的结束原因 detach 原控制端。PTY 和会话身份不随控制权变化。

为了后续支持多个客户端同时在线，1.0 的内部模型仍需区分：

- `Session`：PTY、进程、输出序号和有界 backlog 的所有者。
- `Attachment`：某个已认证设备对会话的一次订阅，生命周期短于会话。
- `ControllerLease`：当前输入和 resize 权限的所有者，1.0 基数最多为一。

输出序号和重放游标必须属于 session/subscriber 关系，不能属于唯一网络连接。这样未来可将输出 fan-out 给多个 attachment，而不改写 PTY 层。1.0 不需要真正实现多订阅者 UI，但也不能用单个固定 `client` 字段封死模型。

用户已确认未来的默认多端语义为单写者、多观察者：所有 attachment 可以消费会话输出，只有 `ControllerLease` 持有者可以发送输入和 resize；控制权可以原子转移。多个客户端同时写入仅可能作为未来显式协作模式出现，不能成为默认协议语义。

## 2.0 前后的能力边界

2.0 之前，核心只认识设备、授权、会话和 PTY 字节流。它不根据进程名称或终端输出推断 Codex、OpenCode 等工具的状态，也不发送 Agent 专用通知。

为了避免未来扩展破坏现有客户端，设计阶段需要保留以下边界，但第一阶段不实现空的插件框架：

- 协议具有显式版本和可选能力协商，未知可选事件可被旧客户端安全忽略。
- 原始终端流与未来的结构化 Agent 事件分离，任何适配失败都回退为普通终端。
- Agent 观察或适配发生在会话层旁路，不进入 Iroh transport、设备认证或 PTY 生命周期核心。
- 客户端通知消费结构化事件，而不是每个平台分别解析不稳定的终端文本。

当 2.0 有第一个真实 Agent 集成需求时，再用其可验证行为反推适配接口，避免现在为未知厂商协议设计过度抽象。

## MVP 验证重点

第一阶段至少用一个持续输出并接受输入的测试进程，以及一个真实的交互式 AI Agent，验证：运行中断网、任务继续、产生输出、恢复连接、重放有界输出、继续输入、最后显式关闭。仅测试普通 Shell 命令不足以证明核心使用场景。

## 无客户端时的会话保留策略

参考实现都把“没有客户端”与“进程应该结束”分开：

- Herdr 官方文档明确说明本地持久会话会在客户端 detach 或终端窗口关闭后继续运行，直到 pane 被关闭、进程退出或 server 停止。
- 本机 tmux 3.7c 的 `destroy-unattached` 默认值是 `off`，即最后一个 client detach 后保留 orphaned session；用户可以显式选择改变该行为。
- Zedra 的 registry 源码存在一个按 `last_activity` 清理连接 session 的 helper 和单元测试，但当前 host 主路径中未找到对该 cleanup helper 的调用，而且该 registry session 与 zterm 需要承诺的持久 PTY 生命周期并不等价，因此不能据此采用自动 PTY 超时。

对 zterm 的建议是 1.0 不设置会话 idle TTL。AI Agent 可能在长时间计算、等待限流或等待用户审批时没有输出或输入，任何基于“最近活动”的自动回收都有误杀真实任务的风险。无人连接时仍保留 PTY 和进程，但每个会话的 VT/scrollback 内存必须有界，并在 `zterm sessions` 中显示创建时间、最后活动时间、是否有控制端和 PTY 根进程是否仍存活，方便用户显式关闭陈旧会话。Agent 等前台子进程退出后若返回仍在运行的会话 Shell，不应误判为整个 session 结束。

代价是遗忘的 Shell、复用器 attach 和子进程可能长期占用进程、文件描述符及有界内存。首版应通过可见列表、显式 close、创建数量上限与达到上限时拒绝新建来管理资源，而不是静默杀死旧会话；具体默认数量可在设计阶段依据内存预算确定。

用户已确认 1.0 不自动关闭 idle session。无人连接、没有输入或没有输出都不能触发终止；只有 PTY 根进程退出、用户显式关闭或经确认停止 daemon 才能结束会话。

## 连接、会话、attachment 与 tab

这四个概念需要独立：

```text
远端设备 / daemon
  ├─ session main   = PTY + 根 Shell + VT 状态
  ├─ session build  = PTY + 根 Shell + VT 状态
  └─ session review = PTY + 根 Shell + VT 状态

临时网络连接 ── attachment ──> 某一个 session
GUI tab / 本地终端窗口 ───────> attachment 的呈现方式
```

- **连接**是临时加密 transport，可以重建，也可以在未来用多个 stream 承载控制请求或多个 attachment。
- **session**属于远端 daemon，不属于某次连接；连接消失后 session 继续存在。
- **attachment**表示某个已认证客户端当前订阅或控制哪个 session。1.0 每个 session 最多一个 controller，但不同 session 可以各自有 controller。
- **tab**只是客户端 UI。桌面 GUI 或移动 App 可以把 session 显示为 tab；CLI 不需要自己绘制 tab bar。

因此多 session 不要求提前开发 App。第一阶段可以采用普通 CLI：

```text
zterm connect server                 # main 不存在则创建，存在则 attach
zterm session list server            # 列出远端持久 session
zterm session new server build       # 新建并 attach build
zterm connect server --session build # attach 指定 session
zterm session close server build     # 显式关闭
```

一个 CLI 进程一次只渲染一个 session。用户可以在本地终端模拟器的多个窗口或 tab 中分别运行这些命令，同时控制不同 session；从当前 session 切换时，第一阶段可以先本地 detach 后重新执行 attach 命令，不需要实现内嵌 tab UI。后续 GUI/Android/iOS 使用同一 session API 绘制原生 tab 或列表。

只提供单一全局 session 的优点是首版命令更少，并可让用户依赖 tmux/Herdr 进行复用；缺点是 zterm 自身永远无法在不依赖远端第三方工具的情况下表达并行任务，移动端和后续 GUI 还要重新扩展协议。建议第一阶段 daemon 和协议直接支持多个 session，但保留稳定的默认 `main`，让不需要多 session 的用户始终获得“每次连接都回到同一个终端”的简单体验。
