# Phase 1 Foundation Gate 技术设计

## 1. 目标与停止线

Gate 0 只回答三个问题：

1. 当前 Iroh 官方生产 profile 是否具备 direct 引擎与可靠 Relay fallback；当前嵌套实验室能否证明官方 QAD 自动发现单独记录，真实网络成功率不由该模型替代；
2. attachment/connection 消失后，PTY 是否继续运行且持续被读取；
3. daemon 侧权威 TerminalModel 是否能用 snapshot + revision delta 恢复当前终端画面，并在资源上限内支持至少三个 session。

这三项共同决定后续 pairing/session RPC 是否值得开始，所以保持一个 cohesive child task 和一份 go/no-go 报告，不再创建三套 Trellis 子任务。实现仍按网络、PTY、VT 三个独立验证面分步，任一失败都保留已完成证据并停止扩展功能。

不变量：

- connection/stream/attachment 的 drop 路径没有 PTY kill/close handle；
- 所有 PTY bytes 先进入宿主 TerminalModel，网络 consumer 永不在这条路径上；
- RelayMap 恰为 Iroh 1.0.3 官方四个 n0 production Relay；不得混入 staging 或自建 Relay；
- 实验特权只存在于本机临时容器，不能改变 Colima guest 或公网服务器；
- 未通过完整 Foundation Gate 时不创建 pairing、persistence、session protocol 或 CLI UX；用户已批准在 B direct、C Relay fallback 成立时越过当前实验室的 A-only evidence gap 进入终端基础层。

## 2. 代码边界

```text
zterm-core
  TerminalModel (concrete wrapper; vt100 types private)
  revision / snapshot / delta / bounded side effects / resource report

zterm-platform
  PtyHost / PtySession
  portable-pty adapter, account login-shell command, input/output/resize/close

zterm-daemon
  InfrastructureProfile -> Iroh Endpoint builder
  minimal session driver: PTY drain -> TerminalModel -> latest revision notice

tests/foundation
  ANSI corpus, deterministic PTY fixture, Patchbay network runner,
  tmux/Herdr harness, resource runner
```

`zterm-core` 不依赖 Iroh 或 portable-pty。`zterm-platform` 不依赖 daemon。`zterm-daemon` 负责组合，不把测试网络类型、tmux/Herdr 名称或候选 VT 私有类型暴露成产品 API。`zterm-proto` 在本 Gate 不新增 terminal wire message；先证明语义，再在 M2 冻结 protobuf。

当前只有一个 VT 实现，因此不建立 trait-object、插件注册表或动态候选选择。`TerminalModel` 是一个公共具体 wrapper，内部 parser/checkpoint 字段私有；同一 corpus、方法契约和结果结构就是替换边界。只有出现第二个实现时才抽 trait。

## 3. 依赖策略

关键依赖精确固定：

- `iroh = 1.0.3`，关闭 default features，只启用实际需要的 `tls-ring` 与 `portmapper`；不启用 metrics、test-utils 或 fast Apple datapath。
- `portable-pty = 0.9.0`。
- `vt100 = 0.16.2`。
- Linux test-only `patchbay = 0.6.0`，与 Iroh v1.0.3 lockfile 和官方 NAT tests 相同。
- Unix 账户查询使用安全 wrapper（`nix` 的 user/fs API），zterm 自己不引入 `unsafe`。

Tokio、错误类型等普通依赖只添加当前代码实际使用的 feature；不为未来 RPC/数据库提前加依赖。稳定 benchmark 用 `harness = false` 的小型可执行 bench 与系统 `/usr/bin/time`，不为一次资源 Gate 引入 Criterion 或自定义 allocator。

## 4. Iroh profile 与网络 Gate

### 4.1 产品 profile

`zterm-daemon` 定义结构化 `InfrastructureProfile`，Gate 默认值为：

```text
base preset = Minimal
relay_mode = official production default map
relay regions = n0 US east + US west + EU + AP
address lookup = explicit Iroh production Pkarr publisher/resolver + DNS origin
QAD = official RelayQuicConfig on every Relay
publish direct addresses = false
portmapper = enabled
ALPN = zterm-gate/1
```

实现从 Iroh `presets::Minimal` 构建，并用 `N0_DNS_PKARR_RELAY_PROD`、`N0_DNS_ENDPOINT_ORIGIN_PROD` 与对应公开 builder 显式安装生产 lookup；Relay map 仍来自 `RelayMode::Default`，不在 zterm 复制官方 Relay URL。不得调用会读取 `IROH_FORCE_STAGING_RELAYS` 的三个 `n0_dns()` shortcut。测试直接枚举最终 RelayMap 和 lookup summary，断言四个生产 hostname、QAD、生产 lookup、无 staging 与无自建 hostname；不能只断言配置源文本。

Gate identity 全部在内存生成，退出即销毁；不写 `~/.zterm`，也不提前实现产品 identity persistence。

### 4.2 三个网络 Case

一个 Linux Gate runner 依次执行全部 Case，最后再给出综合结论，避免 Case A no-go 导致 B/C 证据丢失：

1. **A：产品原样配置**。Home NAT × Home NAT，使用官方 QAD、Patchbay 不提供 UPnP/PCP/NAT-PMP、不注入 `external_addr`。先经 n0 public Relay 建连，等待有界时间观察是否出现 selected IP path。记录所有 direct address 来源和 path events。
2. **B：已知候选对照**。使用相同 profile 与 NAT，只由 test fixture 注入双方 NAT WAN 映射候选。必须先 relay 建连，再观察 direct path，并在 direct 后通过多条 bidi stream 回显。这个 Case 只证明 QNT/holepunch，不改变 Case A 判定。
3. **C：Relay fallback**。阻断 endpoint 的所有非 DNS UDP（包括 QAD 与 endpoint 间 UDP），同时保留到公网 `443/TCP` 的 WSS。必须通过官方 Relay 建连并让多条相互独立的 bidi stream 完成回显；测试结束前确认没有 selected IP path。

Gate go/no-go（2026-08-21 用户批准后的最终解释）：

- A、B、C 都满足预期：Iroh 网络前提 go；
- A 未自动 direct、B direct、C Relay 正常：网络基础前提 go with deferred address-discovery evidence，可以进入 Step 2；报告必须保留 A 的原始结果，并把自动发现成功率安排到父任务 M10 的两条真实网络，不得宣称 A 或官方 QAD 已通过；
- B 也不能 direct：Iroh/拓扑/候选注入本身 no-go，先修实验或更换 transport candidate；
- C 失败：现网 Relay fallback no-go，不以 direct 成功掩盖。

### 4.3 单 VM 拓扑

macOS 只调用一个显式脚本：

```text
Colima VM
  └─ ephemeral privileged test container (--rm)
       ├─ Patchbay IX + public egress adapter
       ├─ NAT A -> Endpoint A
       └─ NAT B -> Endpoint B
```

优先使用 Patchbay 0.6.0 管理 router/NAT/namespace 与 drop cleanup。因为其 IX 默认隔离，runner 只补一条到临时容器外网的 veth/NAT egress，不修改 NAT 模型。若这条接线无法在不 fork Patchbay 的情况下稳定工作，回退为同一临时容器内的最小 `ip netns` fixture；不创建第二台 VM，也不修改 Colima sysctl。

runner 允许隔离 namespace 使用 DNS，并在执行前验证官方 production Relay host 可解析；不再维护单一自建 Relay 的 `/etc/hosts` 特例。连接用官方 home relay hint 建立，DNS/Pkarr adapter 的构造与 relay-only 发布策略由独立 profile test 验证。

当前开发机 Bettbox 已把 `+.iroh.link` 加入 fake-IP filter，并让 `DOMAIN-SUFFIX,iroh.link` 直连；宿主解析已不再返回 `198.18.0.0/15`。runner 仍保留测试专用 DoH A-record 注入，使 Gate 不依赖个人代理配置并避免 Patchbay 测试网段与未来 fake-IP 设置再次碰撞。

网络 Gate 是显式、低频命令，不进入每次 push CI。原因是它依赖公网 Relay 与特权容器；保留脚本供 Iroh/profile/网络基础设施变化时重跑即可。

## 5. PTY 生命周期

### 5.1 平台接口

`zterm-platform` 对上层只公开：

```text
PtyHost::spawn(command, size) -> PtySession
PtySession::take_reader()
PtySession::write_input(bytes)
PtySession::resize(size)
PtySession::try_wait()/wait()
PtySession::close_explicitly()
```

具体 portable-pty master、writer、child 和 killer 都保持私有。attachment 只订阅 daemon terminal output，不能取得 `PtySession`。

当前账户 shell 不继承 daemon 的 cwd 或 `$SHELL`：Unix adapter 从账户数据库取得 effective UID 的 home 与 login shell，用 `CommandBuilder::new_default_prog()`，再显式覆盖 `HOME`、`SHELL` 与 cwd，使 portable-pty 以 login argv0 启动。deterministic fixture 使用显式 argv，不读取用户 rc。

### 5.2 数据流与结束语义

```text
blocking PTY reader
  -> bounded byte channel
  -> one terminal driver (ordered ingest + PTY query replies)
  -> latest revision watch
  -> zero or more simulated attachments
```

byte channel 可以产生可观察的 saturation，但绝不丢 PTY bytes；attachment writer 不在该 channel 后面。attachment 只收到“最新 revision”通知，中间 revision 可合并。没有 attachment 时 reader 与 parser 一直运行。

只有两个 PTY end trigger：根 child 自然退出，或显式 session close。Iroh connection drop、attachment sender/receiver drop、slow consumer 和 Relay 中断只删除订阅。显式 close 走 portable-pty child killer 并 wait；Gate 不自行设计信号升级/进程组清理策略，那属于 M4。

PTY fixture 覆盖 input echo、终端 size、至少超过典型 kernel PTY buffer 的高输出、完成标记、自然 exit 与显式 close。完成标记来自 fixture 自身的控制 pipe/file，不以“测试读到最后一个字符”循环论证 drain 成功。

## 6. 权威 TerminalModel

### 6.1 状态与输出

`TerminalModel` 内部持有 `vt100::Parser<SafeCallbacks>` 与单调 `u64 revision`。每次非空 ingest 或 resize 在同一串行点更新状态与 revision，并返回：

- 需要写回 PTY 的受控 query replies；
- 有界 side events（title、bell、被拒的 effect 类型），不含任意大 payload；
- 当前 revision。

`SafeCallbacks` 明确处理 DA、DSR/CPR 和允许的 title/bell。OSC 52 只产生 `EffectRejected(Clipboard)`；未知 OSC/DCS/APC 不进入 snapshot/delta，也不转发给本地 terminal。reply 必须与最终声明的 `TERM` 一致；Gate 通过前不在产品 session 设置 `TERM=xterm-256color` 或 `COLORTERM`。

### 6.2 snapshot/checkpoint/delta

公开结果不含 vt100 类型：

```text
TerminalSnapshot { revision, size, active_screen, screen_ansi, recent_history_ansi, modes }
TerminalDelta    { from_revision, to_revision, ansi, modes }
TerminalCheckpoint (opaque, fields private)
```

full snapshot 显式重置受控渲染状态、选择 main/alternate screen、恢复当前可见 grid/cursor/style/input modes，并附带有界近期标准 scrollback。checkpoint 私有保存 candidate screen clone。delta 从 checkpoint 到当前状态合并生成；如果尺寸不兼容、checkpoint 无效或 delta 大于 full snapshot，则返回 resync-required/full snapshot，而不是堆积中间 delta。

等价测试把 snapshot 应用到 fresh client parser，再应用 watermark 后的合并 delta，最后比较语义投影（screen 类型、尺寸、cell、cursor、style、modes），不比较生成 bytes 是否相同。corpus 的不同 PTY chunk boundary 与连续 resize 使用同一断言。

### 6.3 资源边界

候选测量固定包含：

- 1、3、16 个 model；
- 每个最多 10,000 行 scrollback；
- 典型 120×40 与上界候选 512×256；
- ASCII、256/true color、宽字符/组合字符与高频更新；
- snapshot/delta bytes、吞吐 elapsed、进程 max RSS。

256 MiB 是 Gate 候选总 terminal-state 预算，不是先写死的产品默认值。若 16 session 超预算但至少三个 session 有界可用，报告可以调低最终 session/scrollback/viewport 默认；若三个也无法有界，Gate no-go。vt100 固定 scrollback 不能动态 trim 时，只允许用实测 worst-case 推导固定 per-session reservation + 拒绝新 session；无法证明 reservation 时更换实现。

Step 6 实测后保留的建议是每 session 2,000 行、无控制端尺寸时 120×40、接受上限 240×80、每用户最多八个 live session，并在创建/resize 前检查所有 model 的 fixed-cell projection 总和不超过 128 MiB；完整进程 RSS 目标仍是 256 MiB。八个饱和上界 model 实测 154.7 MiB，三个 120×40/10k model 也有界；512×256/10k 与 16-session 候选被拒。该策略只在后续 session registry 实现，不在 Foundation 越界新增 registry。

## 7. 黑盒兼容

- deterministic ANSI fixture 是自动化真相源；包括 main/alternate、clear、scroll region、cursor、256/RGB、Unicode、bracketed paste、mouse/focus、resize、DA/DSR 与未知控制序列。
- tmux 使用 `-f /dev/null` 和唯一 socket 名，避免用户配置/已有 server；测试后显式 kill 该测试 server。
- Herdr 临时下载官方 v0.8.2 asset并校验 GitHub SHA-256，在隔离配置/临时目录运行；不使用或停止用户现有 Herdr server。具体操作仍是普通 bytes/resize/detach/reattach，不写进程名分支。
- Codex 0.148.0 与临时 OpenCode v1.18.20 只做无提示词 current-screen、resize、断开后恢复和正常退出 smoke。按程序实际 screen mode 记录结果，不把 full-screen 等同于必然使用 alternate screen；隔离 Codex onboarding 实测使用 main，OpenCode 使用 alternate。无需发送模型请求，不对易变 UI 写快照。

所有 black-box 测试通过与否都由同一 PTY/TerminalModel API 判断。下载、启动与清理脚本属于验收工具，不成为 daemon 运行时依赖。

## 8. CI、报告与清理

常规 CI 覆盖 macOS arm64、macOS Intel、Linux x86_64、Linux arm64 与 Windows x86_64。terminal corpus 跨平台；真实 PTY lifecycle 只在 Unix runner；Windows 只编译公共 platform 边界。网络 Gate、Herdr 下载与人工 Agent smoke 不在每次 push 重复运行。

最终 `docs/foundation-gate.md` 记录：

- exact resolved dependency versions；
- Case A/B/C 的 candidate/path event 证据和 go/no-go；
- PTY lifecycle 结果；
- VT corpus 缺口与选定实现；
- 资源表与建议默认值；
- tmux/Herdr/Codex/OpenCode 版本和结果；
- 后续是否可以进入 M2。

不提交真实 Endpoint secret、qlog、终端 transcript、用户 rc 输出或公网 IP 明细。测试失败只清理本任务创建的临时 container、namespace、tmux socket、Herdr/OpenCode 临时文件；不提供自动 fallback、第二台 VM、生产 Relay 重启或服务器回滚。

## 9. 主要取舍

| 决定 | 原因 | 代价 |
| --- | --- | --- |
| 产品 baseline 与已知地址对照分开 | 防止“打洞引擎可用”冒充“官方 profile 自动地址发现可用” | 当前嵌套实验室的 A-only gap 延期到真实双网络，不能当作通过证据 |
| 使用 Iroh 官方公共基础设施 | 当前无需部署服务器，并能直接验证官方推荐路径 | 免费层限速且无 uptime guarantee，正式生产需重新决定托管或自建 |
| 复用 Patchbay 0.6.0 | Iroh 上游已有相同 NAT 模型与清理语义 | 公网 egress 仍需一层很小的 test adapter |
| concrete TerminalModel wrapper | 只有一个实现，不先造插件系统 | 更换实现时修改私有字段，但 corpus/API 不变 |
| ANSI snapshot/delta 先过 Gate，不先写 protobuf | 先证明终端语义，避免冻结错误 wire shape | M2 仍需决定最终 protobuf 编码 |
| 外部/特权 Gate 不进普通 CI | 避免公网无 SLA 与特权需求制造随机失败 | 相关依赖/网络变化时需显式重跑 |
| 无自动恢复/兜底 | Gate 无持久数据，失败可清理重试 | 失败时需要人工读报告并作下一步决定 |
