# Herdr 与 Zedra 的终端重连状态对比

## 调查范围

- Herdr：官方远程仓库 [`herdrdev/herdr`](https://github.com/herdrdev/herdr)，固定调查提交 `9d7b6c24c4d251a62a861f37c2c394078e083ca8`（2026-08-20）。本机 `/Users/huyuanzhe/projects/herdr` 是空目录，不作为证据。
- Zedra：本机参考仓库 `/Users/huyuanzhe/projects/zedra` 的当前源码。
- 关注点：PTY 所有权、当前屏幕恢复、scrollback、断线回放、溢出语义、磁盘持久化和全屏 alternate-screen TUI。

## 首先拆开三个概念

1. **进程存活**：客户端断开后，PTY 和 Agent 进程是否继续运行。
2. **当前终端状态恢复**：新客户端是否能准确得到当前主/备用屏幕、光标、样式和终端 mode。
3. **历史保留**：用户是否能查看之前的全部输出，以及这些内容是否写入磁盘。

进程继续运行并不自动意味着新客户端能正确重建画面；能重建当前画面也不等于保存了完整 transcript。

## Herdr

### 权威状态在宿主

Herdr 在服务端为每个 pane 创建 Ghostty VT 实例并把所有 PTY 输出喂给它，而客户端消费服务端渲染出的语义 frame 或 ANSI diff：

- [`src/pane.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/pane.rs#L2015-L2055)在 PTY 旁创建 `ghostty::Terminal`。
- [`src/server/headless.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/server/headless.rs#L4426-L4542)即使没有客户端也继续渲染权威状态；有客户端时从 terminal runtime 构造完整 `FrameData`。
- [`src/server/render_stream.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/server/render_stream.rs#L12-L110)为每个客户端维护独立的 frame baseline，可发送完整语义 frame 或服务端生成的 ANSI diff。
- 官方文档明确说 direct attach 先发送当前已渲染终端状态，再发送 live ANSI frames，并使用单写者/takeover 模型（[`persistence-remote.mdx`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/docs/next/website/src/content/docs/persistence-remote.mdx#L103-L129)）。

因此客户端离线期间无需收到每段原始输出，重新 attach 也可以从权威当前 frame 开始，不依赖从进程启动时完整重放所有 escape sequence。

### 内存 scrollback 与磁盘历史分开

- 每个 pane 的默认内存 scrollback 上限是 10,000,000 bytes，并可配置（[`src/config.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/config.rs#L52-L57)、[`src/config/model.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/config/model.rs#L955-L961)）。
- pane screen history 可以序列化成独立的 `session-history.json`，但默认关闭，因为其中可能包含 secret、token、prompt 和命令输出（[`session-state.mdx`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/docs/next/website/src/content/docs/session-state.mdx#L29-L46)、[`src/config/model.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/config/model.rs#L979-L987)）。
- 开启后保存的是服务端 terminal runtime 导出的 ANSI screen history，而不是运行进程本身（[`src/persist/snapshot.rs`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/src/persist/snapshot.rs#L384-L427)）。
- 服务端重启后旧进程已经消失；默认只恢复布局和 cwd，pane history 只是可选画面回放，不代表任务继续（[`session-state.mdx`](https://github.com/herdrdev/herdr/blob/9d7b6c24c4d251a62a861f37c2c394078e083ca8/docs/next/website/src/content/docs/session-state.mdx#L10-L46)）。

### alternate screen 限制

全屏 Agent 往往把“对话历史”保存在应用自身的 alternate-screen UI 中。离开 alternate screen 的内容不会自然进入宿主 terminal scrollback。Herdr 为已识别 Agent 增加了 idle 时模拟滚动并采集多页内容的专用逻辑，但这已经属于 Agent 集成，而非通用 VT 能力。因此通用终端只能稳定承诺当前 alternate screen 和标准 scrollback，不能承诺任意 TUI 的完整内部 transcript。

## Zedra

### 宿主保存有界原始 PTY 字节

Zedra 的 daemon 不维护完整终端 grid。它为每个 terminal 保存带序号的原始 PTY read chunk：

- `/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/session_registry.rs:452-538`：每个终端一个 `VecDeque<BacklogEntry>`，默认最多 50,000 个 chunk，同时有固定 8 MiB 字节上限；溢出时删除最旧 chunk。
- `/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/rpc_daemon.rs:2407-2429`：每次 PTY read 先进入 backlog，再尽力发送给在线客户端。
- `/Users/huyuanzhe/projects/zedra/crates/zedra-rpc/src/proto.rs:1036-1056`：`TermAttachReq` 只携带客户端 `last_seq`，`TermOutput` 只有原始 bytes 与 seq。
- `/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/rpc_daemon.rs:3522-3566`：重新 attach 时回放 `last_seq` 之后仍保留的 chunk。

客户端用 `alacritty_terminal::Term` 解析这些原始字节并在本地维护 grid/scrollback（`/Users/huyuanzhe/projects/zedra/crates/zedra-terminal/src/terminal.rs:198-260`、`277-335`）。因此同一个仍在内存中的客户端短暂断网时效果较好：它保留原有 VT 状态，只需要补上缺失 bytes。

### gap 的局限

当客户端丢失的序号已经被 8 MiB backlog 淘汰时，host 会记录 `backlog gap detected`，但当前协议没有发送 terminal snapshot 或 gap recovery 消息；客户端收到数据后只是继续喂给本地 VTE。由上述协议字段和处理路径可以推断：一个全新客户端、被系统杀死后重建的移动 App，或超长时间离线的客户端，可能从 escape sequence 中间或缺少早期 terminal mode 的状态开始，不能保证准确恢复当前全屏 TUI。该问题在 host 日志中可观察，但用户侧没有完整恢复路径。

### 磁盘只保存结构与授权

Zedra 的 `sessions.json` 只保存 authorized client keys，以及 session 的 ID、名称、工作目录和 ACL；不包含 `TermSession`、PTY backlog 或终端 grid（`/Users/huyuanzhe/projects/zedra/crates/zedra-host/src/session_registry.rs:679-702`、`751-825`、`846-864`）。daemon 重启后这些 session 元数据仍在，但终端进程、原始 backlog 和客户端 screen state 均不恢复。

## 对比

| 维度 | Herdr | Zedra |
| --- | --- | --- |
| 权威 VT 状态 | 宿主 | 客户端 |
| 在线增量 | 服务端渲染 frame/ANSI diff | 原始 PTY bytes |
| 断线缓存 | 宿主 VT + 每 pane 10 MB scrollback | 每 terminal 最多 8 MiB / 50,000 raw chunks |
| 新 attach | 当前完整 frame，再接增量 | 从 `last_seq` 回放仍保留的 raw chunks |
| 缓冲溢出 | 丢旧 scrollback，当前 screen 仍完整 | 产生 raw byte gap，当前 screen 可能无法重建 |
| 默认落盘终端内容 | 否；pane history 显式 opt-in | 否 |
| daemon 重启 | 进程终止；结构恢复，history/Agent 恢复另算 | 进程终止；只恢复 session/ACL 元数据 |
| 通用 alternate-screen transcript | 不保证；专用 Agent 逻辑另做 | 不保证 |

## 对 zterm 的建议

zterm 的核心场景是控制可能运行数小时的全屏 AI TUI，并且未来要支持多个观察端。仅复制 Zedra 的 raw backlog 会把最重要的恢复保证建立在“客户端未丢失内存状态且离线输出未超过上限”这一脆弱前提上。

建议采用 Herdr 的状态所有权原则，但保持 zterm 自己的最小产品边界：

1. daemon 在每个 PTY 旁维护权威、跨连接存活的 VT 状态，包括主/备用屏幕、光标、终端 mode 和有界 scrollback。
2. attach 先取得带输出 watermark 的完整当前 screen snapshot，再消费 watermark 之后的有序增量；snapshot 与增量切换必须无丢失、无重复。
3. 内存上限按 terminal 配置。溢出只淘汰旧 scrollback，不能破坏当前 screen；慢客户端可跳过中间 frame 并重新取得最新 snapshot，不能反向阻塞 PTY reader。
4. 1.0 默认不把终端内容写入磁盘。只持久化设备身份、授权、配置与必要的寻址缓存；active session 元数据、PTY、VT 和 scrollback 都不持久化。显式本地录制若以后需要，应作为单独能力设计保留期、权限和清理，而不是把内存恢复缓存直接落盘。
5. 1.x 的通用终端保证“准确恢复当前主/备用屏幕 + 有界标准 scrollback”，不承诺保存任意全屏 TUI 内部的完整对话历史。2.0 后再通过真实 Agent API/集成补充结构化历史和通知。

这一选择比 Zedra 的 raw replay 多出宿主 VT emulator、snapshot/delta 协议和终端兼容测试，但它直接解决长期断线、新设备 attach、移动 App 被系统杀死以及未来多观察端的共同问题。

## 嵌套使用：先连接 zterm，再进入终端复用器

研发初期的预期使用链路不限定 Herdr，也包括 tmux、GNU Screen、Zellij 等终端复用器：

```text
本地终端
  ↕
zterm 控制端
  ↕ 加密连接
zterm daemon 的权威 VT / 外层 PTY
  ↕
终端复用器的 attach 客户端
  ↕ 复用器自身的 IPC
复用器 server / 内层 PTY
  ↕
Codex、OpenCode 等进程
```

两层终端状态不冲突，因为它们管理不同层级的 PTY：复用器维护内层 pane/window；zterm daemon 维护“正在运行复用器 attach 客户端”的外层终端。复用器把内层状态渲染为 ANSI 输出，zterm 把它当作普通全屏终端程序处理。zterm 不应识别 tmux、Herdr 或任何其他复用器进程，更不应针对它们切换会话协议。

该组合有一个重要的正向效果：zterm 控制端断线时，外层 PTY 不关闭，复用器 attach 进程仍连接其 server；zterm daemon 持续读取并解析外层 PTY 输出。控制端重新连接同一个 zterm session 后，可以先从 zterm 的完整 VT snapshot 恢复当前复用器 UI，再继续输入，无需了解复用器内部 session 协议。

对 tmux 而言，zterm 控制端离线并不等于 `tmux detach`：外层 `tmux attach` 进程依然存在，会继续作为一个 tmux client 并保留最后的终端尺寸。这保证 zterm reattach 后回到原画面，但也意味着该 client 可能参与 tmux 自己的多客户端尺寸策略；这是嵌套复用器的可见语义，zterm 不应通过猜测进程类型自动改变它。用户显式执行 tmux detach、Herdr detach 或其他复用器命令时，才由对应程序结束内层 attachment。

为了让该组合可靠，zterm 必须满足以下通用终端约束：

1. 没有在线控制端时仍持续排空 PTY 并更新 VT；不能等客户端重新连接后才读取，否则高输出量可能填满 PTY 缓冲并阻塞复用器 attach 客户端。
2. daemon 作为虚拟终端端点，必须处理应用发出的终端查询并把响应写回 PTY；`TERM`/`COLORTERM` 只能声明实际实现并测试过的能力。
3. 当前控制端的 resize 必须更新 zterm 外层 PTY，再由复用器 attach 客户端传给自己的 server 和内层 PTY。无人连接时保留最后一次有效尺寸即可。
4. 第一阶段 Unix CLI 的键盘输入需要无损传入外层 PTY；alternate screen、光标、颜色、Unicode 宽度、bracketed paste 和 resize 是通用文本复用器的重点兼容面。
5. zterm 自己的本地控制通道不能简单占用复用器的常用 Ctrl 前缀。tmux 和 Herdr 默认使用 `Ctrl+B`，GNU Screen 等工具还有其他常用前缀；外层截获会直接破坏嵌套透明性。SSH 风格的行首 `~.` 冲突较少，但在全屏 TUI 中为了形成行首而先输入 Enter 可能触发当前界面，不适合作为唯一方式。更合适的候选是可配置的一次性 `Ctrl+]` 本地前缀、双前缀原样发送以及完全禁用前缀的选项；代价是单独使用 `Ctrl+]` 的远端程序需要用户双击或改键。
6. Herdr direct attach 当前在源码中仅支持 Unix raw-byte input，Windows 明确返回 unsupported；因此 Herdr 适合作为第一阶段 macOS/Linux 的复杂兼容样本，不能直接证明第三阶段 Windows 路径。tmux 则提供另一个更传统的复用器样本。

第一阶段应把 tmux 与 Herdr 作为两种代表性黑盒验收样本，同时保证实际兼容性来自同一套通用 PTY/VT 契约，而不是程序名白名单。GNU Screen、Zellij 和其他复用器应沿用相同路径。鼠标、OSC 52 剪贴板、OSC 8 链接、Kitty graphics 等增强能力可以分别协商和测试，不应影响基础文本终端、断线存活和重连恢复。
