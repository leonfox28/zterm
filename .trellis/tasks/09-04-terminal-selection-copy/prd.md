# 终端文字选中与复制

## Goal

为桌面 Zterm CLI 补齐 attachment-local 的普通拖拽文字选择与安全复制，使普通 shell 中的基本
选择体验不再因 Zterm 的物理鼠标捕获而丢失，同时保持滚动条和 nested TUI 的标准单一输入
owner 语义。已确认采用与 Herdr 同类的 Zterm-local selection，而不是要求用户 Shift-drag 或
关闭鼠标捕获。该能力应建立在现有 semantic surface/history cache 与唯一 compositor/presenter
边界上，不在 daemon、Session 或 Alacritty 共享模型中增加客户端选择状态。

## User Value

- 在 Zterm 普通 shell 的 live screen 与已缓存历史 viewport 中，可以用鼠标拖拽形成可见选区
  并把对应文本复制到实际用户桌面的剪贴板。
- Herdr、vim、PiAgent 等声明 mouse-reporting 的程序仍收到自己的普通鼠标事件，不因增加
  Zterm selection 而失去点击、拖动或滚动能力。
- 后续 Android 可以复用选择坐标、文本提取与 ownership 规则，但使用原生手势、绘制与剪贴板，
  不继承桌面 CLI 的 SGR/OSC 适配细节。

## Confirmed Facts

- CLI 在 active terminal UI 生命周期内持续向 physical outer terminal 声明 SGR any-motion
  capture（`1003 + 1006`）。普通左键事件因此到达 Zterm，而不是自动形成 outer-terminal
  selection。
- 当前 `HostInputCodec` 已完整保留 SGR button code、cell coordinates、release 与 raw bytes，
  但只识别 wheel、left press 和 motion；未建模 modifier 或 selection lifecycle。未被 gutter、
  child mouse 或 alternate-scroll 消费的普通 mouse event 当前被丢弃。
- v2 `TerminalSurfaceRow` 已提供 exact fixed-width semantic cells、`wrapped`、wide head/
  continuation 与 bounded UTF-8 contents。CLI 的 `AttachmentSurface` 和
  `ViewportCache<TerminalSurfaceRow>` 已持有当前完整可见内容，无需增加第二个 terminal parser
  或把 selection 放入 daemon。
- `ComposedFrame` 是 active attachment 所有可见 cell、cursor、mode 与 chrome 的唯一 desired
  frame，`DesktopPresenter` 是唯一 outer-terminal writer。selection highlight 必须作为 compositor
  overlay 进入同一 frame，clipboard terminal effect 也不得与 presenter stdout 写入交错。
- child mouse ownership 已由 daemon 投影的标准 `ActiveScreen + TerminalModes` 决定；SGR child
  路径逐份原样转发。产品逻辑不得识别 Herdr、PiAgent、进程名、`TERM` 或主题颜色。
- outer terminal 的 GUI copy shortcut 只认识 outer terminal 自己的 native selection；它无法读取
  Zterm 内部 semantic selection。Ghostty 的默认 copy binding 在没有 Ghostty-native selection 时会
  fall through，但 macOS legacy 键盘编码明确不输出 Super-modified text，因此仅画出 Zterm 选区并
  不能让 `Cmd+C` 到达当前 byte-oriented CLI input path。
- outer terminal 在鼠标上报关闭时可以完整拥有拖拽选区与 `Cmd+C`，但 Zterm 当前必须开启鼠标
  上报才能同时拥有 remote-history wheel、右侧滚动条及 nested TUI 鼠标转发。标准鼠标协议没有
  “只把滚轮/滚动条交给应用、普通左键拖拽仍交给 outer terminal”的按区域所有权；Ghostty 与
  Kitty 均以 Shift-drag 作为 mouse-grab 下的 native-selection escape hatch。因此可移植方案必须
  在“普通拖拽由 Zterm 选择”和“Shift-drag 由 outer terminal 选择”之间明确选择，不能把同一
  普通拖拽同时交给两者。
- Herdr 0.8.2 的参考做法是：未声明 child mouse 时由 host 建立 screen-buffer-coordinate
  selection；声明 child mouse 时先由 libghostty-vt 编码并转发，成功后不建立 Herdr selection；
  默认 mouse-up copy-on-select，可选保留 selection 后由收到的 Ctrl/Cmd+C 复制；clipboard 优先
  平台工具，必要时回退 OSC 52。也就是说，Herdr 为了兼得普通拖拽、滚轮、内部滚动条和 nested
  TUI 鼠标，确实由自己维护选区与输入 owner。Zterm 只借鉴所有权、状态与 side-effect 分层，
  不复制其 Ghostty FFI、unsafe 或 multipane/copy-mode 复杂度。
- Herdr 还通过 stack-scoped Kitty keyboard enhancement 接收结构化 Cmd/Ctrl+C，只在 finalized
  selection 存在时消费该按键及其 repeat/release，其他按键按 pane 声明的 keyboard protocol 编码后
  继续转发。Alacritty 0.26.0 已有 Kitty keyboard set/push/pop/query 状态机，但 Zterm 当前将其关闭并
  拒绝所有 CSI-u。可靠复制与 nested TUI 键盘兼容必须共用一个通用 keyboard gateway，不能只匹配
  Ghostty 的某条字节序列。
- Herdr 的 clipboard 目标不是 pane 内部剪贴板：monolithic 模式先尝试 Herdr 所在机器的 native
  clipboard，失败或检测为 SSH/WSL/VS Code remote 时向 stdout 发 OSC 52；server/client 模式则
  将内容只发给 foreground Herdr client，再由该 client 走相同 native/OSC 52 writer。
- Herdr 自己跨 server/client 边界时也不透传 child 的原始 OSC 52：terminal callback 先将其规范化
  为 bounded clipboard content，server 发送结构化 `ServerMessage::Clipboard` 给 foreground
  client，最后才由 client 的 clipboard writer 重新产生 OSC 52。这与 Zterm 的 semantic-wire
  架构一致，是 nested clipboard bridge 可借鉴的边界。
- 当前 Zterm `TerminalIngressPolicy` 会完整消费 child OSC 52，只产生不含 payload 的
  `EffectRejected(ClipboardWrite)`；Alacritty engine 也以 `Osc52::Disabled` 作为第二层防线。因此
  Herdr 嵌套在 Zterm 中走 OSC 52 时，请求不会到达本机 Ghostty，也不会改变 attachment 机器的
  系统剪贴板。若 native writer 成功，它写入的是 Session 节点所在机器的剪贴板，而不一定是
  用户当前 attachment 所在机器。
- 已确认采用统一 attachment-local clipboard sink：remote child OSC 52 在 daemon trust boundary
  被解析为结构化 clipboard-write effect 后只发给当前 controller；Zterm 自有 selection 在本地
  copy action 时直接调用同一个 sink。desktop CLI sink 最后一跳重新编码 canonical OSC 52，Android
  后续替换为原生 ClipboardManager；任何边界都不透传 child 的原始控制串。
- 当前产品上限 `80 rows * 240 columns * 22 UTF-8 bytes + 79 newlines` 的极端完整可见选区为
  422,479 bytes。512 KiB decoded-text cap 可完整覆盖该范围；其 RFC 4648 padded Base64 最长
  699,052 bytes，结构化 decoded payload 加 protobuf overhead 仍显著低于现有 1 MiB control-message
  上限。Herdr 0.8.2 自己将 terminal clipboard write 限为 192 KiB，因此其合法请求也落在此范围。

## Root Cause Classification

这不是 Herdr 专属兼容 bug，而是三个边界缺失形成的一组架构问题：

1. **interaction ownership 缺口**：Zterm 为滚动条、history 和 child mouse 持续捕获 physical mouse，
   但在 child 不接管时没有 attachment-local selection owner，所以普通拖拽被消费后无结果；
2. **transient host-effect 缺口**：terminal ingress 能识别 OSC 52，却只有“拒绝”side event，没有一条
   controller-scoped、latest-only、不可 replay 的 clipboard effect 路径；
3. **keyboard gateway 缺口**：outer terminal 不知道 Zterm-local selection，而 Zterm 又未跟踪 child
   Kitty keyboard mode，因此既不能可靠收到 macOS Cmd+C，也不能在启用 enhancement 后保证 nested
   TUI 的其余按键语义；实现审查进一步发现，pinned history 会隐藏 live delta 的视觉重绘，但不能
   隐藏该 delta 对 application cursor/keypad、bracketed paste、focus reporting 与 Kitty keyboard
   mode 的语义推进。若这些 physical outer input modes 只随视觉帧提交，outer 产生的输入编码会落后
   于 authoritative child mode。这是 physical host-effect ownership 的边界缺陷，不是 history
   renderer 或某个 TUI 的专属问题。

修复必须分别建立统一 pointer router、attachment-local selection、structured transient effect 和
child-mode-aware keyboard gateway，并由唯一 presenter 独立投影、同步所有会改变 physical outer
输入编码的 host modes。每个 delta 必须先形成 post-delta surface、viewport/cache anchor 与 selection
候选，再由该候选 selection identity 派生 outer keyboard mode；不能从 pre-delta 选区派生后再做第二次
补偿同步。隐藏 delta 只有在 changed-controls 的单次 write/flush 成功后，才能一起推进上述本地
semantic state，且不得借机提交未显示的 live rows；候选 viewport 只能预览紧凑元数据，不能为每个
隐藏 delta 克隆缓存行。mouse mode/encoding 与 alternate-scroll 只由 Zterm router 使用，不得镜像给
outer。按 Herdr/进程名分支、只透传 raw OSC 52、只匹配一条 Cmd+C escape sequence 都会保留同类
缺陷，禁止采用。

## Requirements

### R1. One Mouse Event, One Owner

- main/alternate screen、live/history 与 child mouse mode 的所有组合必须由一个应用无关的
  interaction router 选出唯一 owner：Zterm gutter drag、已有 Zterm selection drag、child mouse、
  alternate-scroll、Zterm history wheel 或 local text selection。
- 在 child 未声明 mouse-reporting 且 pointer 位于 child content rectangle 时，普通左键按下/
  拖动/释放建立 Zterm local selection；一次纯点击不得留下空选区。
- child 已声明 mouse-reporting 时，普通 mouse event 继续逐份转发，不能同时改变 Zterm selection。
  outer terminal 保留的 Shift-selection escape hatch 不得被 Zterm 强制关闭。
- gutter、remote status row 与 viewport 外坐标不得进入文字选区；进行中的 gutter drag 与 selection
  drag 不能互相抢占。

### R2. Attachment-local Semantic Selection

- selection controller 属于一个 CLI attachment，保存 anchor、focus、gesture phase 与对应 screen/
  history identity；不得进入 Session、daemon model、wire checkpoint、磁盘或另一个 attachment。
- renderer-neutral cell range normalization 与 semantic text extraction 放在可被后续 Android client
  复用的 core helper；desktop mouse gesture/capture、高亮与 clipboard sink 仍只属于 CLI attachment。
- main history 坐标必须相对 semantic history anchor 稳定；正向与反向拖拽等价。resize/reflow、
  screen identity change、reconnect/takeover、history gap 或无法证明内容身份时清除 selection，
  不能把旧坐标套到新内容。
- 初始 MVP 支持当前完整可见 viewport 内的线性 cell selection。跨 cache-edge 自动滚动、键盘
  copy mode、矩形选择、搜索、double/triple-click word/line selection 后续独立扩展，但坐标模型
  不得阻止这些能力。

### R3. Exact Text Extraction and Highlight

- 提取顺序为阅读顺序；只输出 wide head 的 contents，跳过 continuation；保留 combining text。
  选中范围内的 semantic blank 输出一个空格，范围外的行尾 padding 不得进入结果；`wrapped =
  true` 的相邻物理行不插入换行，非 wrapped 行边界插入 `\n`。
- 选择边界落在 wide head/continuation 任一格时扩展为完整 glyph，不得复制半个宽字符或重复文本。
- 所有输出必须是已有 validated `TerminalCell.contents` 的有界 UTF-8，设置一个独立、测试覆盖的
  512 KiB decoded clipboard byte cap；超限不截断到半个 scalar/glyph，也不发 partial clipboard
  effect。该 cap 同时约束 Zterm-local selection 和 child OSC 52，未来若增加跨 cache-edge 选择，
  必须显式重审而不能静默提高。
- highlight 只修改 composed copy 中的视觉 style，不改变 `AttachmentSurface`、history cache 或
  daemon state；content、selection、gutter、status 和最终 cursor 仍由一次完整 presentation
  transaction 提交。

### R4. Keyboard Gateway and Local Clipboard Boundary

- Zterm 自有 selection 的 clipboard write 只能由明确的本地用户复制动作触发；不上传网络，
  不写日志/Debug/状态持久化，也不回传给 PTY。
- 启用 Alacritty 已有的 Kitty keyboard set/push/pop/query 状态机，把五个标准 child flags
  投影进 `TerminalModes`。只放行严格合法的 keyboard-mode CSI-u control；不把 child raw control
  直接传给 outer terminal。
- Zterm 不独立跟踪或限制 Alacritty 内部 keyboard stack 深度；通过结构校验的 set/push/pop
  直接使用锁定版本引擎的原生栈语义，不为尚未在实际使用中出现的依赖边界增加防御状态。
- Zterm UI guard 在 outer terminal keyboard stack 上只拥有一个 entry：child flags 非零时精确镜像；
  child flags 为零时通常保持 legacy，只有 finalized Zterm selection 存在时临时请求
  `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS`。第一位让 Ghostty 将
  Cmd+C/Ctrl+Shift+C 编成结构化事件，event type 保证一次物理按下只复制一次且不会把 repeat/release
  猜成 press，alternate key 保留键盘布局与 shifted/base identity 以完成 legacy downgrade。退出路径
  必须 pop 并恢复调用者状态；禁止用 timer 推断按键阶段。
- sole `HostInputCodec` 负责 bounded Kitty CSI-u 解码。仅当 finalized selection 存在时消费
  Ctrl/Super+C press 并 suppress 对应 repeat/release；没有选区时保持 child 输入语义。outer/child
  flags 相同时 raw bytes 原样转发；仅在 local selection 导致的 `0 -> flags 7` 差异下，把其他
  合法结构化 key 降级为等价 legacy bytes 后清除 selection。畸形或未知输入不得被猜测成 copy。
- desktop clipboard backend 固定为 canonical `OSC 52;c;<standard padded base64>BEL`，使用同一个
  512 KiB cap，并由唯一 `DesktopPresenter` 串行 `write_all + flush`；不调用平台 shell/clipboard
  executable，不增加配置分支，产品 Rust 代码继续 `unsafe_code = "forbid"`。
- clipboard 成功或不可确认不得伪造成 daemon/session 成功状态；selection highlight 本身是基本
  可见反馈，任何额外 toast 都必须与 status row/chrome 同帧且不能引入闪烁。
- Zterm-local selection 只能在 copy action 发生时调用 attachment-local clipboard sink；仅移动或
  完成选区不等于复制，本任务明确不启用 copy-on-select。CLI sink 向 outer terminal 输出一条由
  Zterm 重新编码的 canonical OSC 52，Android sink 后续使用系统原生 clipboard API。OSC 52
  没有 acknowledgement，成功请求后保留高亮；纯点击、普通非 copy 输入、viewport/source identity
  变化才清除。

### R5. Nested Clipboard Architecture

- child OSC 52 不得作为 raw bytes 穿过 daemon、wire 或 presenter。daemon trust boundary 必须先
  完整 framing、验证 selector/base64、实施 decoded-size cap，并转换成不含控制字节的结构化
  clipboard-write effect；畸形、超限和 read 请求保持拒绝。
- 只接受非空的 `OSC 52;c;<RFC 4648 standard padded Base64>` system-clipboard write。空 selector、
  primary/secondary/select/cut-buffer、多 selector、空 payload、非 canonical Base64、invalid UTF-8
  与 NUL 均原子拒绝；不 trim、normalize 或改写合法文本，保留 tab/newline/Unicode。`?` clipboard
  read 永久拒绝且不回复 child。
- decoded UTF-8 最多 524,288 bytes；对应 encoded payload 最多 699,052 bytes。只为已经识别为
  OSC 52 的 parser state 提供该专用上限；其他 OSC/DCS/APC/PM/SOS 继续使用现有 1,024-byte cap。
  超限后丢弃至原控制串 terminator，不能把余下 bytes 显示成文本、断开 Session、截断后写入或
  让 Base64 与 decoded buffer 同时无界增长。
- clipboard-write effect 只投递给事件产生时的当前 controller attachment；无 controller 时丢弃，
  不广播给 observer、不进入 semantic snapshot/delta/history、不在 reconnect/resync 时重放，也不
  出现在 Debug/log 中。
- child clipboard write 固定 allow；不增加当前没有消费者的配置项，也不做无法可靠绑定到进程
  身份的应用名白名单或交互式 prompt。每次 ingest 最多保留一个 clipboard payload，跨异步
  delivery 也只有一个 replaceable pending slot；多个请求 latest-wins，不让慢/断开的 controller
  反压 PTY 或积累 `N * 512 KiB` 队列。初版不再叠加无证据的按秒 timer rate limit。
- 所有拒绝都是 payload-free；日志、错误、Debug、metrics 与 toast 均不得包含 decoded text 或
  Base64。允许记录有界 reason/count，但不能记录片段；sink 失败不回传 clipboard 内容、不终止
  Session，也不向 child 声称成功。
- controller client 将 remote child effect 与 Zterm-local copy action 汇入同一个 clipboard sink。
  desktop CLI 最后一跳固定编码 canonical `OSC 52;c;<base64>BEL` 并由唯一 presenter/output owner
  串行写给 outer terminal；Android 后续直接调用平台 clipboard。

## Acceptance Criteria

- [x] 普通 shell 的 main live screen 和已缓存 history viewport 均可正向/反向拖拽选择，移动时
  只更新一份高亮，纯点击不产生选区。
- [x] ASCII、CJK/wide、combining、显式空格、wrapped 与非-wrapped多行提取结果精确，超限复制
  原子失败且不泄漏内容到 Debug/log。
- [x] OSC 52 whole/chunked/framing tests 覆盖 exact-cap/over-cap、strict Base64/UTF-8/NUL、read、
  selector、empty、cancel/terminator/overflow；合法 512 KiB write 只产生一份 redacted structured
  effect，所有拒绝均不渲染、不写剪贴板且不产生 child reply。
- [x] clipboard effect 只到事件发生时的 controller；无 controller、observer、takeover race、
  disconnect/reconnect、slow consumer 与多 write burst 均证明 no replay/no broadcast/latest-wins、
  bounded memory 且 PTY/model 不被反压。
- [x] selection 是 attachment-local；另一个 attachment、daemon model revision 与 child PTY
  均不受影响。resize/reflow/reconnect/screen identity gap 清除 stale selection。
- [x] child mouse-reporting fixture 的 press/drag/release/wheel 仍逐份只交给 child；gutter drag、
  history wheel、alternate-scroll 与 local selection 的完整路由矩阵均证明 one owner。
- [x] selection highlight 与 existing content/gutter/status/cursor 经同一 `ComposedFrame` 和单次
  DEC 2026 outer presentation 提交；failed/partial host write 不提交错误 baseline。
- [ ] child Kitty keyboard set/push/pop/query 在 snapshot/delta/checkpoint/wire 中一致；Zterm 自己的
  outer stack entry 在所有退出路径恢复。Ghostty 的 Cmd+C（macOS）和 Ctrl+Shift+C（Linux）在有
  Zterm selection 时只复制一次，无选区时不被 Zterm 吞掉；普通 shell 与 nested TUI 的 Ctrl/Alt/
  Super、function key、press/repeat/release 语义不因 host enhancement 改变。
- [ ] macOS Ghostty 与至少一个非 Ghostty outer terminal 验证选择、复制、child TUI、退出清理；
  Linux hosted runner 验证 shared selection/extraction/clipboard fallback，真实桌面 clipboard
  验收必须明确记录环境，不能由 compile-only 代替。
- [x] workspace 继续禁止 Zterm-owned `unsafe`，core/proto 不增加 host clipboard、ANSI、GUI 或
  `alacritty_terminal` 依赖。

## Out of Scope

- 搜索 UI、键盘 copy mode、矩形选择、double/triple-click word/line selection。
- 跨当前缓存边界的 selection auto-scroll 或无限 scrollback 镜像。
- Android UI、selection handles、touch magnifier、native clipboard 和无障碍集成。
- child clipboard read、图片/富文本剪贴板。
- OSC 52 allow/deny 配置、应用名白名单与 per-request prompt。
- 应用名或 outer-terminal identity 特判。

## Key Decisions

- bounded plain-text child OSC 52 write 固定 allow；clipboard read 永久 deny；只投递给事件产生时的
  controller。当前不增加 allow/deny 配置；Session 内程序可覆盖 controller clipboard 是该能力被
  启用后的明确产品语义。
- Zterm 自有 selection 在 mouse-up 后保留高亮，等 `Cmd+C`/平台 copy key 才调用同一 client
  clipboard sink，不默认 copy-on-select。
- desktop sink 固定使用 presenter-owned canonical OSC 52；不增加 native-command fallback。为可靠
  收到 Cmd+C，使用 stack-scoped、child-mode-aware keyboard gateway，而不是 Ghostty keybind 特判。
- child 控制串只在 daemon ingress 被解释一次；wire 传 domain text，client sink 才产生平台 effect。
- 合法 child Kitty keyboard stack 控制直接交给 Alacritty；Zterm 不维护独立 stack-depth 防线。
- clipboard cap 固定为 512 KiB decoded UTF-8，并采用 R5 定义的严格 OSC 52 子集。
- 本次为全节点同步升级的 v2 direct cutover；不增加旧版本 capability、fallback、dual decoder 或
  mixed-version clipboard/keyboard 分支。
