# Improve remote terminal resilience and connection UX

## Goal

让远程 zterm attachment 在调整窗口时保持可靠，能够浏览宿主已保留的有界标准终端历史，并通过一条不干扰远端程序的状态栏明确显示当前连接目标与链路状态。

## Background and Confirmed Facts

- 用户在 macOS Ghostty 中连接远端设备后，曾在调整窗口时看到
  `daemon_stopped: write local terminal message: Broken pipe (os error 32)`，随后暂时无法复现。当前内容无关 daemon 日志中没有对应 attachment/resize 失败记录，因此不能把偶发错误直接归因为 Ghostty 或远端 PTY。
- CLI 在 `crates/cli/src/terminal_ui.rs:64` 进入 zterm 自己的 alternate screen；Ghostty 的普通主屏 scrollback 因此不是远端历史的可靠浏览入口。
- resize 在 `crates/cli/src/terminal_ui.rs:297` 与 attachment events 并发处理；写入已经关闭的 same-UID attachment stream 会在 `crates/daemon/src/local_ipc.rs:1132` 暴露原始 `Broken pipe`。这证明错误表面，不足以证明哪一侧先关闭。
- daemon 已为每个 Session 保留至多 2,000 行标准 main-screen history，并在 snapshot 中携带受 frame 上限约束的近期历史；当前 CLI 只把它写入 outer alternate screen，没有历史浏览状态或 cursor（`crates/daemon/src/operations.rs:209`、`crates/cli/src/terminal_ui.rs:1502`）。
- 既有第一阶段设计已把按需 `HISTORY_PAGING` 留作后续：cursor 必须绑定 history revision/epoch，淘汰或并发变化要返回明确的 `history_changed` / `history_gap`，不能读取不一致历史（`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/design.md:415`）。
- connection broker 已按远端 Device 保存 redacted selected path kind，并可读取 selected Iroh path 的 RTT；当前 public status 只有聚合 direct/relay 计数，没有 attachment 所属 peer 的 RTT（`crates/daemon/src/connection_broker.rs:1132`、`:2428`）。

## Comparative Research

- Herdr v0.8.2 与 tmux 3.7c 都由 server/daemon 持有 pane history，并提供自己的历史浏览 UI；tmux 还明确说明 outer terminal scrollback 无法保持完整一致。
- 两者都按 child terminal mode 分流滚轮：普通 main-screen transcript 浏览宿主历史；alternate screen、mouse-reporting 或 child-owned scrolling 则把输入交给 child。Herdr 的 direct attach 还对无修饰 PageUp/PageDown 做相同分流。
- Herdr 的浏览语义是 live-at-bottom、用户进入历史后 pinned；tmux copy mode 则克隆进入时的 pane screen。zterm 已有 daemon authoritative state 与 revision/epoch cursor 规划，更适合前者。
- Herdr 和 tmux 的底部 chrome 都占用独立物理行，并用剩余高度 resize PTY，而不是覆盖 child 的最后一行。
- 完整证据与逐项比较见 [`research/herdr-tmux-terminal-history.md`](research/herdr-tmux-terminal-history.md)。

## Requirements

### Resize and attachment closure

- 连续或快速调整 Ghostty 窗口不得仅因 SIGWINCH/resize 消息而结束仍然存活的 local 或 remote attachment。
- resize 与 daemon-side attachment 关闭、remote reconnect、snapshot synchronization 或 Session end 同时发生时，CLI 必须优先呈现已知的 typed lifecycle outcome；不得把可归类的关闭竞态降级成裸 `Broken pipe`。
- 如果本地 daemon 确实停止、Session 确实结束或 attachment 确实失去 lease，仍返回现有精确错误，不通过无界重试掩盖故障。
- 为暂时不可复现的原始报告增加确定性竞态测试和最小内容无关诊断；不因一次现场现象引入第二套 reconnect owner。

### Bounded history browsing

- attachment 必须能够浏览宿主 daemon 已保留的标准 main-screen history，受现有每 Session 2,000 行上限约束；不新增磁盘 transcript 或无界客户端 backlog。
- 普通 main-screen transcript 下，mouse wheel/trackpad 与无修饰 PageUp/PageDown 必须无需预先输入快捷键即可浏览 zterm history；本轮不新增显式 history-mode 快捷键。
- 输入分流只依据 authoritative terminal modes：child 已请求 mouse reporting、alternate-scroll 或 full-screen Page keys 时把事件交给 child；否则由 zterm 浏览 history。不得按 `tmux`、Herdr、编辑器或 pager 名称特判。
- live Session 在浏览期间继续运行；位于底部时跟随 live output，用户上翻后 viewport 保持 pinned。滚动回到底部恢复 live display；普通键盘输入或 paste 先回到底部，再按正常 attachment 语义转发给远端 PTY，不形成隐藏 modal state。resize 继续更新显示几何并尽量保持 history anchor。
- history cursor 必须绑定明确 epoch/revision。旧行被淘汰或历史在分页边界失效时，在 zterm-owned history viewport 内显示明确 gap/changed 提示，不占用或扩展三字段状态栏，也不拼接不一致内容。
- 不承诺重建 arbitrary alternate-screen TUI 的内部历史；tmux、Herdr 和其他 TUI 仍走同一个通用 terminal path，没有程序名特判。

### Remote connection status bar

- remote attachment 默认在物理终端底部保留一行 zterm 状态栏；远端 PTY 看到的 viewport 相应减少一行，resize 仍保持一致。
- 状态栏只显示三个字段，固定从左到右为：所连 Device 的安全显示名/alias、当前连接模式、延迟；不增加独立 transport-state、历史位置或其他持久字段。
- 连接模式显示 `direct`、`relay` 或尚不可判定时的 `--`；延迟显示当前 selected path 的有界整数 RTT 毫秒值或 `--`。
- 整条物理行（包括字段后的空白单元格）必须有连续底色。已确认以 ANSI reverse video 交换终端主题的默认 foreground/background，不写死 RGB、不查询 host palette，也不依赖 Ghostty 私有协议；完整研究见 [`research/terminal-theme-status-colors.md`](research/terminal-theme-status-colors.md)。
- Device 字段使用 attach 时冻结的本地安全 alias，不因连接期间重命名而改变路由身份；延迟取当前 selected Iroh path 的 RTT，最多每秒刷新一次并显示四舍五入后的有界整数毫秒。
- 状态更新不得泄露 Device ID、IP、Relay URL、ticket、终端内容或其他 bearer material，也不得进入远端 PTY input/output。
- 窄窗口必须稳定截断或降级字段；状态栏重绘不得破坏远端 cursor、颜色、main/alternate screen、mouse、bracketed paste 或 terminal restoration。
- local attachment 和未来更多状态字段不在本轮默认状态栏范围内；物理终端只有一行时临时隐藏状态栏并把唯一一行留给远端 PTY，恢复到至少两行后自动重新显示。

### Compatibility and scope control

- Ghostty on macOS 是本轮现场验收终端；保留现有通用 Unix ANSI/TTY 边界，不依赖 Ghostty 私有协议。
- 保持现有单 Session controller、daemon-authoritative terminal model、bounded queues、reconnect 和 exact snapshot acknowledgement 所有权。
- `HISTORY_PAGING` 只传递 daemon-authored bounded rows/cursor；CLI 不新增第二 terminal parser，输入分流也不新增 per-program compatibility branches。

## Acceptance Criteria

- [ ] 在确定性测试中让 active resize 与 local-stream close、typed Session end、remote reconnect 和 resynchronization 分别竞争；仍存活的 attachment 不退出，已知终态显示 typed outcome，且不向用户暴露可归类的裸 `Broken pipe`。
- [ ] Ghostty 中快速连续改变窗口大小，remote shell 与 alternate-screen TUI 都保持 attachment；物理终端至少两行时远端程序观察到的尺寸等于物理行数减一，只有一行时按已定义 fallback 获得该行。
- [ ] 产生超过一屏但不超过 2,000 行的标准输出后，wheel/trackpad 和 PageUp/PageDown 可直接浏览历史；滚回底部或正常输入后显示最新 authoritative state。
- [ ] viewport pinned 时普通键盘输入与 paste 先恢复 live bottom，再精确转发一次；不得丢失、重复或因隐式 modal state 吞掉输入。
- [ ] 普通 shell history gesture 由 zterm 消费；remote child 开启 mouse reporting、alternate-scroll 或 full-screen Page-key ownership 后相同输入正确转发，nested tmux/Herdr 与常见 TUI 不被外层 history 抢占。
- [ ] 历史被淘汰或 cursor 失效时出现明确 gap/changed 提示；内存和 frame/queue 上限不变，daemon restart 后历史仍按既有约定消失。
- [ ] remote 状态栏严格按 `<device> | <direct|relay|--> | <integer ms|-->` 排列，在 initial sync、active direct、active relay、reconnecting 和 unknown path 下显示正确值，且路径迁移后会更新。
- [ ] 状态栏整行使用确认过的 reverse-video 底色；窄列宽时按显示单元安全截断，单行物理终端时临时隐藏，连续重绘下不继承 child SGR、也不把自身 style 泄漏到 child terminal。
- [ ] 窄窗口、Unicode Device alias、连续 resize、snapshot/delta、main/alternate transitions、tmux 和 Herdr 的通用回归通过；detach、错误和 signal 路径完整恢复终端。
- [ ] 相关 Rust format、Clippy、type-check、unit/integration/PTY gates 和独立 Trellis checker 通过。

## Out of Scope

- 显式 history/copy mode 与专用快捷键、全文搜索、文本选择/复制、磁盘 transcript、session recording 和任意 alternate-screen 应用内部历史。
- 状态栏颜色配置、固定 RGB/ANSI palette、图标、每字段配色、历史位置、transport phase、IP、Relay URL 或 Device ID。
- local attachment 状态栏、GUI/移动端 chrome、Ghostty 私有颜色查询和按程序名维护兼容分支。
