# zterm 架构与实现审查

基线：`fde63f6`，2026-09-05，macOS arm64。主代理独立完成；产品源码未改。

## 结论

保留当前六个产品 crate 和“宿主权威模型 → semantic wire → 单一前端呈现者”的方向。
主要问题集中在客户端控制操作的等待边界，以及更新链路上重复执行的工作；当前证据不支持替换 Alacritty、Iroh 或整体重写 actor。

本审查追踪了终端主链路，检查了连接、授权、存储、重放和生命周期的关键所有者及现有测试。
它不是所有源码逐行安全审计，也不证明其他平台、真实公网或所有交互程序均正确。

## 实际所有权

| 状态/能力 | 当前所有者与证据 | 判断 |
| --- | --- | --- |
| PTY、根进程、终止权 | `crates/platform/src/pty.rs:375`、`:414`、`:438`；`crates/daemon/src/terminal_driver.rs:109` | I/O 与 child control 分锁，附件没有 kill 能力，应保留 |
| VT grid、history、revision | `crates/terminal/src/model.rs:115`；`crates/terminal/src/engine.rs` | 一个宿主模型；core/proto 没有引擎依赖 |
| Session、controller、附件 checkpoint | `crates/daemon/src/session.rs:393`、`:2597`、`:3032` | 每 Session 独立 actor，支持阻塞隔离；不应并入 socket loop |
| Endpoint 与 peer connection | `crates/daemon/src/network.rs`；`crates/daemon/src/connection_broker.rs:352`、`:1375`、`:1714` | endpoint、dial singleflight、stream admission 有明确所有者 |
| 远端字节隧道 | `crates/daemon/src/remote_tunnel.rs:1`、`:31`、`:74` | viewer daemon 不解释 Session 内容，应保留 |
| Session 客户端、resume、请求关联 | `crates/daemon/src/local_ipc.rs:908`、`:1194`、`:1677` | 在前端进程运行；源文件却混合服务端与客户端，见 F5 |
| 前端最新完整语义屏幕 | `crates/cli/src/terminal_ui/surface.rs:10` | 与物理呈现基线含义不同，不应强行合并 |
| 滚动窗口与选择 | `crates/core/src/viewport_cache.rs`；`crates/cli/src/terminal_ui/selection.rs` | 客户端局部状态，不进入 Session/transport |
| 最后成功呈现的物理画面 | `crates/cli/src/terminal_ui/ansi_presenter.rs:20` | 唯一 stdout presenter，write/flush 成功后才推进 |
| 持久元数据与授权 | `crates/daemon/src/store.rs`；`crates/daemon/src/authorization.rs:1` | SQLite 持久事实与带 generation 的内存授权快照有不同职责 |

## 已确认问题

优先级表示处理顺序：P1 为响应性缺陷，P2 为确定的冗余或维护性问题。
静态操作次数不是端到端加速比例。

### F1 · P1 · 附件控制操作无完整截止时间，租约等待缓存无总量上限

- 证据：`crates/daemon/src/local_ipc.rs:755` 的 `write_session_bytes` 对 Direct/Tunnel 都直接等待 `write_all`；`:1911` 的 `send` 没有 deadline；`:1967` 的 `next_operation_id` 无限等租约，`:1980` 附近将不匹配请求号的帧逐个加入 `deferred`，没有数量/总字节限制。路径事件在 `:1950` 的 `read_transport_frame` 也直接暂存。
- 上层路径：`crates/daemon/src/operations.rs:765` 的 `submit` 无界等待命令入队和结果；`:867` 的 driver 在命令 handler 内等待，期间不处理另一个命令或事件；`:500` 的初始 takeover 也直接等待 `begin_takeover`。
- 隔离复现：`attachment_deadline_probe.rs` 使用真实同用户 Unix socket 和当前公开 `LocalAttachmentClient`。伪服务端返回合法初始 snapshot 后，保持 socket 打开并不回复租约；6 秒时请求仍 pending，已暂存 128 个无关帧。另一个连接不再读取，900,000 字节输入在 6 秒时仍 pending，完成消息数为 0。
- 影响：停顿的同用户服务端或远端通道可能长期占据客户端命令所有者；单帧大小限制不能约束累计暂存量。普通空闲附件允许无限等事件，这与已提交控制操作必须有界是两种不同场景。
- 根因分类：已有 absolute-deadline/bounded-control 契约的局部实现遗漏，位于前端 Session client；不需要增加另一个监督器。
- 修复方向：一个绝对截止时间覆盖排队、写入和相关响应；暂存帧同时限制数量和编码字节。写超时可能已提交部分帧，必须丢弃该 transport epoch，不能继续向原流补写新帧。输入不重放，takeover 歧义保持原有 typed outcome 语义。
- 验证：将探针转换为确定性回归，覆盖静默租约、无关帧积累、阻塞写与 owner 释放；正常空闲流保持存活。

### F2 · P2 · 每次附件更新重复完整投影

- 证据：`crates/daemon/src/terminal_driver.rs:706`、`:719` 先调用 snapshot/delta，再调用 `model.checkpoint()`；`crates/terminal/src/model.rs:195`、`:204`、`:214` 三个入口各自调用 `project`。`crates/terminal/src/projection.rs:117` 遍历全部可见行列并分配投影。
- 触发：初始同步，以及每个 revision 已推进的 `sync_changed`；相同 revision 已正确提前返回，不存在空闲同步循环。
- 影响：同一次更新、同一把模型锁内遍历并构建两份完整 projection；多个附件分别重复此工作。默认最大 80×240 视口下，每次有变化的同步进行两轮各 19,200 个 cell 的投影。
- 根因分类：模型 API 将输出与后续 checkpoint 分开获取，造成确定的重复工作；不是 PTY 输入解析性能的证据。
- 修复方向：一次捕获同时产出 semantic update 和其精确 checkpoint。第一步只消除第二次投影，不加入跨附件共享缓存或 dirty-row 世代系统。
- 验证：snapshot/delta 重放相等、慢附件跨多个 revision、Main/Alternate/resize 强制替换，以及 returned checkpoint 与输出 revision 相同。

### F3 · P2 · 前端 delta 嵌套两层完整候选复制

- 证据：`crates/cli/src/terminal_ui/surface.rs:26` 克隆完整 surface 后调用 `apply_to`；`crates/core/src/terminal.rs:554` 又在内部克隆 baseline，再替换行并完整验证。
- 触发：所有 contiguous delta，包括只有 cursor/modes/metrics 变化、没有 row patch 的更新。
- 影响：进入 compositor 前已有两次完整 surface clone；`TerminalCell.contents` 是拥有所有权的字符串，非空 cell 随之深复制。live compositor 在 `crates/cli/src/terminal_ui/composition.rs:215` 附近还会复制用于 chrome/selection 的呈现行，这一份职责独立。
- 根因分类：事务性候选构造在 core 与 UI 各做一次，属于局部重复实现。
- 修复方向：core 提供一个返回完整已验证候选的入口，UI 直接接收它；`apply_to` 如仍有消费者则复用同一构造逻辑。保留 semantic candidate 与 physical committed baseline 的区别。
- 验证：错误 delta 不改旧状态；host write/flush 失败不推进 surface、viewport、selection 或 applied revision；零 row patch 更新仍推进正确 revision/modes。

### F4 · P2 · 已拥有的 protobuf 内容先深复制再消费，初始屏幕保留过久

- 证据：`crates/daemon/src/local_ipc.rs:1784`、`:1798`、`:1822` 将 snapshot/delta/history message 克隆后传入消耗所有权的转换函数，原值此后没有消费者。转换入口分别为 `crates/proto/src/lib.rs:1492`、`:1545`、`:1655`。
- 另一个保留链：`local_ipc.rs:921` 长期保留完整初始 snapshot；`crates/daemon/src/operations.rs:2107` 再复制给 `PreparedTerminalView`。driver 启动后，client 仍持有初始屏幕，生产路径只在 Debug 中读取它的 revision。
- 影响：内容转换有确定的额外深复制；每个活跃 view 留有已不参与更新的旧初始屏幕。这不是协议为了本地/远端一致性所必需的成本。
- 修复方向：三个转换直接 move；将初始 snapshot 随准备阶段移交，运行态仅保留需要的 revision/size/identity。先调整真实消费者，再删除失去用途的字段/API。
- 验证：现有 semantic/identity/history correlation 测试与 prepared-view 生命周期测试；确认无消费者依赖启动后读取初始内容。

### F5 · P2 · 源码组织混合了前端客户端和 daemon 服务端

- 证据：`local_ipc.rs:182` 是 listener，`:360` 是 ingress dispatch，`:641` 起是 transport/client，`:2343` 起是 unary client；`operations.rs:437`–`:1280` 附近又承担客户端 event driver 与 public event projection。客户端名称仍为 `LocalAttachmentClient`，实际同时服务 Local 和 Remote。
- 影响：进程边界在运行时正确，但在源码依赖和命名上不清楚；扩展一个客户端行为需在混有服务端职责的文件间跳转，容易再次引入双 Session interpreter。
- 根因分类：模块边界欠清楚，未发现需要第二个进程或 crate 的证据。
- 修复方向：同一 daemon crate 内建立明确的 `client` 模块，将 transport、Session client、view driver 收到同一边界；保留单一公共 façade，local listener 与 opaque remote tunnel 继续由 server 侧拥有。暂不新增 `zterm-client` crate。
- 验证：本地/远端相同 target-visible trace；模块移动不改变 wire、public CLI、resume ID 或 shared connection 行为。

### F6 · P2 · UI 转移散落在一个巨大的 event loop 中

- 证据：`crates/cli/src/terminal_ui.rs:229` 的 `run_view` 延续到约 `:1165`，同时维护输入 epoch、resize、transport、surface、viewport、selection、presenter。`:441`、`:535`、`:665` 附近分别处理进入同步；`:1344` 的转移函数需要大量状态引用。
- 已有失败证据：最近修复记录 `../../archive/2026-09/09-04-fix-zterm-herdr-snapshot-sync/research/herdr-break-loop.md` 说明同一事件内先改 transport state、后决定 ACK，曾把旧 delta 当成新 resize epoch 的确认。当前 `:642` 已正确捕获事件入口状态；这不是仍存在的缺陷。
- 影响：正确性依赖散落的调用顺序，后续修改容易再次混淆“事件进入时”“候选”“已呈现”三个时点。
- 修复方向：用一个有明确字段归属的 UI session owner 收拢现有状态，事件处理方法捕获入口状态后构造/提交；复用唯一 presenter。不是创建通用 reducer framework，也不是把三种有效状态合成一个值。
- 验证：保留现有 entry-state ACK、Main/Alternate、resize fence、pinned-history、selection/copy、write/flush failure 测试，并复用真实 outer-PTY 路径。

## 检查后决定保留或推迟的机制

| 项目 | 证据与决定 |
| --- | --- |
| Session actor + PTY reader/model threads | `session.rs:3032` 每 10 ms 检查 child；8 Session 对应名义最多约 800 次/秒的空闲超时轮询，不是测得 CPU。阻塞隔离、退出和 drain 已有必要契约，本轮不整体异步化 |
| 两套 command gate | `session.rs:2998` 在操作已开始时继续等待精确结果；`store.rs:1499` 超时返回 outcome unknown。外形相近但错误/完成语义不同，不机械抽成通用 actor framework |
| 满 history 后 epoch 频繁变化 | `model.rs:339` 在容量满时保守视为可能 eviction；此前研究已明确这是正确性优先的折衷。精确 eviction tracking 本轮推迟 |
| Presenter 全屏 compare/composition | 保证 chrome、selection、physical baseline 同时提交。先去掉 F2–F4 的确定重复，暂不增加第二套 damage cache |
| 多处校验 | wire 解码验证形状、client 验证 attachment/request 身份、core 验证相邻 revision、Session 验证授权/controller，检查不同不变量，应保留 |
| 六 crate 与引擎/网络选择 | 当前依赖方向正确；CLI 传递依赖 engine 是因为同一可执行文件包含 daemon。未来移动端是新的消费者，不能据此虚构今天必须拆 crate 的要求 |

## 验证与证据边界

- `cargo +1.98.0 test --workspace --all-features`：退出 0；标准 harness 合计 497 passed、0 failed、6 ignored，另有自定义 harness。
- `just check-fast`：退出 0；包含 source/dependency policy、format、Clippy、release/workflow/shell/Python 静态门和 secret-scan fixtures。
- `python3 .trellis/tasks/09-05-zterm-architecture-review/research/run_probe.py`：退出 0，确认 F1 两种 pending 行为；它是故障观察器，当前退出成功不表示缺陷已修复。
- Linux real-Iroh、cross-UID、其他架构与显式 terminal black-box 未在本次运行中获得新证据。部分 ignored 是由父测试调用的子进程入口，不能全部算作功能未测。
- 遵守当前 terminal-model 契约，不运行吞吐/CPU/RSS/候选引擎 benchmark；性能结论限于源码可证的遍历、复制和保留次数。
