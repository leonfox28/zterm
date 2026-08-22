# M4 实施计划：持久 Session、PTY 与本地 attach

## 实施约束

- [x] 开始前运行 `trellis-before-dev`，重新注入 core/proto/platform/daemon 的适用 spec。
- [x] 只实施 M4；不创建 Iroh Endpoint、remote adapter、最终 raw CLI UI、Windows ConPTY/
      Named Pipe、observer 或 Agent 特判。
- [x] 所有代码阶段先保持一个小的 focused gate，再跑比例化 workspace gate；不为同一契约
      堆叠重复 shell/static/runtime 检查。
- [x] implement 完成后必须独立 `trellis-check`，局部机械问题由 checker 修复后重跑门禁。

## Step 0：基线与边界冻结

- [x] 确认 worktree 只含本任务规划文件，记录当前 HEAD/CI 基线。
- [x] 运行 source-policy、workspace-version、fmt、workspace Clippy/tests/docs、cargo-deny。
  - 开始时运行了 focused baseline；完整 workspace/docs/deny 在最终门禁全绿，没有倒填为
    “未改代码前完整基线”。
- [x] 为 SessionService、SessionActor/resource governor、local attachment stream 分别建立 spec
      owner；更新既有 terminal/PTY spec 时只记录新增可执行契约。
- [x] 检查生产源码中当前无 SessionRegistry/attach handler，确认 M3 unary 与 Foundation
      TerminalDriver 是唯一 retained starting point。

Gate：改动前 focused baseline 与改动后完整 workspace gate 全绿；任何失败先定位，不能被
M4 改动掩盖。

## Step 1：core domain 与资源预检

- [x] 新增 SessionName、SessionSelector、SessionEndReason 和需要的 typed domain error；
      固定 `main` 保留规则与 name 长度/字符测试。
- [x] 扩展/复用 ControllerLease、OperationWindow，使状态变更 exact-result replay 可由
      daemon-issued `OperationLease { incarnation, ordinal }` + non-zero sequence 有界管理；覆盖
      executed/replayed/evicted/fixed-lease mismatch 与 exhaustion，不接受 client-invented lease。
- [x] 给 TerminalModel 增加不分配的 checked projection/resize preflight 与按完整历史行
      限制 snapshot wire payload 的唯一入口；保留 vt100 私有边界。
- [x] 证明 invalid size、revision overflow、projection overflow 和 history trimming 失败前
      不改变现有模型。

Gate：`cargo test -p zterm-core`、terminal corpus/snapshot tests、core Clippy/docs 全绿。

## Step 2：protobuf 与 frame registry

- [x] 在 common/session/terminal proto 补 SessionSelector、viewport、working directory、
      LeaseLost、SessionEnded/end reason；不添加无当前消费者的 event/history/observer 消息。
- [x] 同步 MessageKind、WireKind、kind conversion、control-payload classification 和文档。
- [x] 为所有新增 DTO 做 domain conversion、malformed/field bounds 和 frame round-trip；验证
      snapshot/delta encoded length不超过 MAX_FRAME_BYTES。
- [x] 保持 ALPN/wire major 不变；因为尚无发布客户端，只做兼容字段新增和新 kind，不复制
      自定义 framing。

Gate：`cargo test -p zterm-proto`、proto Clippy/docs、现有 M3 local IPC focused test 全绿。

## Step 3：TerminalDriver 生命周期与通知边界

- [x] 增加 latest revision watch，使模型线程可从 blocking thread 发布最新水位，且不为每个
      revision 排队。
- [x] 增加 consuming finalization：root exit/explicit close 后等待 reader EOF、queue drain
      和线程 join；不持 PTY/session mutex 等待，不让 attachment 获得 close authority。
- [x] 让 resize 使用一个 projection/preflight owner并保持 model/native PTY 一致；失败可回滚
      resource reservation，不能静默 clamp。
- [x] 保留 query reply 写回同一 PTY，回归曾经的 wait 持锁死锁测试。

Gate：terminal_drain、attachment_resync、PTY lifecycle 与新增 driver lifecycle tests 全绿。

## Step 4：SessionRegistry、actor 与 SessionService

- [x] 实现 registry id/name index、main create singleflight、ResourceGovernor 与 actor completion
      compare-remove；任何等待均在 registry lock 外。
- [x] 实现每 Session 一个有界 actor command loop，组合 PtyHost login shell、TerminalModel、
      TerminalDriver、attachments 与 controller lease。
  - transport-independent `SessionService` 保持同步 API；每 Session 由独立 OS thread 拥有
    runtime 和容量 16 的 `sync_channel`。command 用 `try_send` + absolute deadline
    queued/started/expired gate，local current-thread Tokio adapter 只通过 `spawn_blocking` 进入。
    已 started 的 mutation 在 caller disconnect/timeout 后继续并写入 exact replay cell；expired
    queued command 不开始 PTY/model/lease 副作用。
- [x] 实现 list/create/rename/close、prepare_attach/snapshot_applied/next_update/input/resize/
      detach/takeover/shutdown；所有副作用从唯一 service 入口提交。
- [x] 接入 operation replay，确保 create/rename/close/takeover 的成功和 typed error 都可精确
      replay；close replay 不要求已被移除的 Session 仍存在。
- [x] status/doctor/stop 从 registry 读真实 live summary/impact；自然退出与 close 竞态只结束
      一次，stop 关闭全部 Session。
  - close child control 与可能阻塞的 PTY writer 分离；shutdown 先并发请求所有 close，再在同一
    absolute deadline 内等待/回收。仍有 child/driver/actor/reservation 时返回 typed deadline，
    listener/socket 保持可诊断可重试，不发送 stopping=true。
- [x] operation replay 只在 global mutex 下短暂注册/查找，执行发生在 per-key cell 外；same key
      只有 fingerprint 相同才 join/replay exact result，不同 payload 拒绝，unrelated key 并发；
      panic/drop completion guard 以 OutcomeUnknown 唤醒 waiter。daemon 按 stable principal 签发
      incarnation + monotonic ordinal lease，64 active lease（含 lost-response empty lease）通过
      fully-completed prefix + retired-through floor 回收；restart/invented/high/retired lease 不执行，
      in-flight lease 不被回收，ordinal/sequence 不 wrap。
- [x] name slot 同时承担 create reservation 与 rename uniqueness；spawn 后 publication loss 显式
      close/reap/join driver；SessionId collision check/resource insertion 在 state→resources 原子边界，
      name/resource/actor 使用同一 token compare-remove。actor 在 fallible registration 前交给
      CreationOwner，cancel 后 Starting name 保留到实际 cleanup；timeout/error 留下 provisional owner。
      creation/actor/close/driver unwind finalizer 与 poison-aware cleanup 保证 waiter 终态、ownership
      可重试、reservation/name/interrupt 不悬挂且不误删 unrelated token。
- [x] shutdown 先向全部 live/provisional owner 发 close，只省略 ordinary ended SessionNotFound
      summary race，其他 typed summary/wait/join error 在 cleanup 后上报；ownership 未清空时恢复
      admission/listener 供重试，绝不返回 stopping=true。

Gate：

```sh
cargo test -p zterm-daemon --test session_lifecycle
cargo test -p zterm-daemon --test controller_lease
cargo test -p zterm-daemon --test session_limits
```

## Step 5：same-UID local duplex adapter

- [x] 重构 local listener 的首 frame 读取，使 peer gate 后由一个 FrameDecoder 分类 unary/
      attachment，并正确保留同一 read 中的剩余 frame。
- [x] unary 路径继续要求一个 frame + EOF；将 session list/create/rename/close/takeover 接到
      SessionService，不破坏 M3 lifecycle RPC。
- [x] 加入 mutation-only lease request/response：LocalClient 首次 mutation 前 lazy 获取并缓存；
      readiness/status/list 不分配；ambiguous transport 最多一次 byte-identical request/ID 重试，
      typed OutcomeUnknown 不自动换 lease 重跑，只让后续独立 operation 申请新 lease。
- [x] attachment 路径实现有界 reader/writer loop、snapshot handshake、latest-only update、
      input/resize、sync、detach、lease/session terminal event 和 deadline/cancellation。
- [x] 任一 half/协议错误只 detach 一次；socket writer 慢时 actor/PTY 不等待。
- [x] recoverable accept error 留在 listener loop；fatal serve exit 仅在 Session ownership cleanup
      成功后 unlink socket。失败时保留 process/daemon lock/store/service/child，以
      dev+inode+ctime token（防 Linux 立即复用 inode）compare-rebind 自己的 socket 并恢复
      status/stop；replacement socket 不会被删除。
- [x] 新增真实 socket test client/harness；它不进入 CLI 命令树，也不实现 raw terminal。

Gate：

```sh
cargo test -p zterm-daemon --test local_ipc
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-daemon --test terminal_recovery
```

## Step 6：端到端生命周期与黑盒兼容

- [x] 并发首次 attach main、main 显式 close 后重建、三个独立 Session、rename/name conflict、
      invalid cwd、root exit/foreground child return、targeted close/daemon stop。
- [x] 无 attachment 下持续排空至少 1 MiB，重连核对 SessionId/cwd/current state/history；
      main/alternate/Unicode/color/modes/resize 的 snapshot + delta/resync 语义等价。
  - attachment checkpoint 重建为 zero-scrollback visible-only baseline，容量固定为
    rows*columns*2；每 Session 最多一个 pending takeover，所以 1.0 最多 controller + pending
    两个 checkpoint。
- [x] 同步前输入丢弃、正常 occupied、prepared takeover、LeaseLost、stale generation 拒绝、
      no-double-write 与 response-loss replay 故障注入。
  - repeat regression 通过真实 dropped unary response + byte-identical bounded retry、新 socket same
    OperationId replay、barrier-proven same-key singleflight/unrelated-key overlap、第二 pending
    takeover 拒绝；takeover response 丢失后新 synchronized stream 用 opaque token 取得输入权且不
    clobber later controller。
- [x] 8 Session/第 9 个、最大 viewport/越界、projection boundary/overflow、resize rollback、
      slow writer/latest-only 和 RSS measurement target 回归。
- [x] tmux 使用唯一临时 socket；Herdr 使用 Foundation 固定版本/校验和/隔离目录。两者共用
      通用 attachment harness，结束后只清理本任务资源并扫描生产源码无名称特判。

Gate：

```sh
cargo test -p zterm-daemon --test session_lifecycle
cargo test -p zterm-daemon --test terminal_recovery
cargo test -p zterm-daemon --test controller_lease
cargo test -p zterm-daemon --test session_limits
sh tests/foundation/terminal-blackbox.sh --mode tmux
sh tests/foundation/terminal-blackbox.sh --mode herdr
```

若现有 black-box runner 参数不同，复用其实际显式入口，不新增第二套下载/校验脚本。

## Step 7：文档、跨平台与最终门禁

- [x] 更新 backend specs、README/docs：准确说明 daemon-lifetime persistence、main、local attach、
      resource limit、takeover 和 stop/restart 边界；不宣称远端或 Windows runtime 已完成。
- [x] 更新 parent/child checklist，只有实际运行且有证据的条目才能勾选。
- [ ] Unix hosted matrix 跑真实 PTY/socket；Windows hosted job 跑共享 core/proto/daemon compile
      和 unsupported tests，不用本机 cross-compile 代替证据。
  - CI matrix 已配置；本机 macOS arm64 全绿。Windows 本地 cross-compile 在 native
    `ring`/MSVC headers 之前失败，不能冒充 hosted Windows 证据，保持未勾选。
- [x] 普通 push 不下载 Herdr、不跑公网/网络 Gate；显式 gate 记录固定版本和清理结果。
- [x] 运行完整 final gate，检查无用户 secret、真实 transcript、用户 tmux/Herdr 资源或测试
      临时物残留。

Final gate：

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
sh tests/relay/static.sh
sh tests/secret-scan.sh
python3 .trellis/scripts/task.py validate .trellis/tasks/08-22-persistent-session-local-attach
git diff --check
```

2026-08-22 本机 macOS arm64 证据：上述 final gate 全绿；`cargo deny` 仅报告允许的 duplicate
warnings。显式 `terminal-blackbox.sh --mode tmux`（tmux 3.7c）与 `--mode herdr`（固定 Herdr
0.8.2 + SHA-256）均通过并确认清理。`local_session_ipc` 的 reconnect/natural-exit 场景在修复
revision-watch-close 竞态后连续运行 40 次通过。`cross_uid` 在非 Linux 本机按设计跳过，hosted
matrix 与独立 checker 仍保持未完成。

2026-08-22 repeat Phase 2.1 blocker pass：daemon-issued lease/restart/invented/exhaustion、
byte-identical ambiguous retry、typed OutcomeUnknown poison rotation、panic duplicate waiter、
provisional publication/unwind cleanup、takeover reconnect authority、recoverable listener accept failure、
fatal serve ownership 与 truthful shutdown summary 的 focused regressions 全绿。随后再次运行上面的
完整 final gate，全部 exit 0；`cargo deny` 仍只有允许的 duplicate warnings，`cross_uid` 在本机按
设计跳过。此 repeat 未改 consumer harness，因此未重跑已经通过的 tmux/Herdr；没有运行任何
Codex/OpenCode agent mode。

2026-08-22 final narrow ownership pass：SessionId/name/resource 使用同一 ownership token 与固定
`registry state -> resources` 锁顺序；publication timeout/error、真实 ownership mutex poison、
nonblocking actor/driver Drop + background reaper、ByteQueue late enqueue、fatal listener actual
`run_daemon` rebind，以及 detached/later-controller takeover adversarial regressions 全绿。随后运行
上述完整 final gate，全部 exit 0；`cargo deny` 仅报告允许的 duplicate warnings，`cross_uid` 在
本机按设计跳过。consumer harness 未改，因此未重跑 tmux/Herdr 或任何 agent mode。

独立 checker 随后按 actor 身份修正 shutdown 的 summary/join 去重并保留已观察 actor 直到实际
join，恢复 poisoned OperationCell 的终态写入/唤醒，并补齐 cleanup-only SessionId 冲突拒绝与普通
API poison typed-error 断言。对应 adversarial tests 与完整 final gate 均再次全绿；没有遗留设计级
偏差。hosted Unix/Windows 证据仍按要求保持未完成。

## 完成条件

- [ ] PRD 的全部 M4 验收标准有直接证据，且没有借用 M5–M8 的未实现能力。
- [x] 独立 checker 未发现设计级偏差；所有机械修复已回归。
- [ ] hosted CI 全绿后才完成/归档 child task；更新 parent M4 对应 checklist，但不提前勾选
      remote connection 或最终 CLI 条目。
