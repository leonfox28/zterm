# Official Rust VT Cores and C Rewrite Evaluation

Date: 2026-09-02

## Executive Finding

`libghostty-vt` 不是“因为用 C 写，所以 Rust 不能调用”。其实现主体是 Zig，官方提供
C-compatible ABI。Rust 完全可以链接并调用它；限制是 Rust FFI declaration/call 必须经过
`unsafe`，而 C header、bindgen 或静态链接都不会自动证明指针、lifetime、callback 和 ABI
契约安全。

在以下三个约束下不存在当前可用的交集：

1. 只使用 Ghostty 官方维护的接口；
2. Zterm-owned Rust code 继续 `unsafe_code = "forbid"`；
3. 在当前 Rust 进程内直接集成 `libghostty-vt`。

不采用社区 wrapper 后，Ghostty 路线只有三个诚实选项：Zterm 自己维护一个隔离的 unsafe
FFI crate（改变安全政策）、用独立 C/Zig sidecar 做进程隔离（增加 IPC 系统），或等待
Ghostty 官方 Rust API。在当前约束下，更合理的是评估由原终端项目直接维护的 Rust core。

## What “Official” Means Here

下面的“官方 Rust core”指 crate/source 由对应终端项目本身维护，而不是第三方把另一个
项目的 C ABI 再包装为 Rust。它们仍然是 Zterm 的外部依赖，不等于 Zterm 自有代码。
workspace 的 `unsafe_code = "forbid"` 约束 Zterm-owned crates；它不会、也无法证明所有
transitive dependencies 内部都没有 unsafe。

## Candidate Comparison

| Candidate | Upstream/status | Useful capabilities | Gaps and risks | Fit |
|---|---|---|---|---|
| `alacritty_terminal 0.26.0` | Alacritty 官方仓库与 crates.io package | raw-byte parsing, screen/scrollback, resize/reflow, modes, replies, selection/search, damage | 不提供 Zterm wire snapshot/delta、稳定远程 history cursor 或完整 input encoder；依赖内部有 unsafe；总内存仍需实测 | **首选 qualification candidate** |
| `rio-vt 0.5.26` | Rio 官方仓库与 crates.io package；2026-07 才从 Rio 拆出 | headless API, grid/scrollback, reflow, damage, events/replies, modern modes；graphics 可选 | API/packaging 很新；发布源码与 README 的 unsafe 描述不完全一致；OSC heap fallback 未见总上限，graphics payload 上限也高于 Zterm daemon budget | 观察/实验，不宜当前 cutover |
| `wezterm-term` | WezTerm 官方源码 workspace；未作为独立 crates.io package 发布 | 功能最完整，含 scrollback、input encoding、graphics、hyperlinks | 需要 pin 整个官方 Git workspace，内部依赖面大，边界不是稳定独立 package，安全和资源审计成本最高 | 不推荐当前采用 |
| `avt 0.18.0` | asciinema 官方 Rust crate | 小、无界面、scrollback/reflow/dirty lines，自有源码禁止 unsafe | 输入为 UTF-8 text 而非原始 PTY bytes；mode/reply/OSC/combining-char 能力不足 | 不满足长期功能目标 |
| `vt100 0.16.2` | 当前 Rust dependency | raw bytes、headless screen、state diff，现有 Zterm adapter 已稳定 | 功能上限是本次重新选型的原因 | 在新候选通过 Gate 前保留 |
| `vte` | Alacritty 官方 parser crate | ANSI/VT parsing building block | 只有 parser，没有 screen、scrollback、reflow 或 terminal state | 不能单独替代当前模型 |

## Alacritty Mobile Target Probe

`alacritty_terminal` 的 manifest 只有 Unix/Windows target dependencies；crate root 无条件公开
`tty` 和 `event_loop`，没有只编译 state core 的 feature。Alacritty v0.17.0 官方 CI 只测试
Windows、macOS 和 macOS x86_64 cross-build，没有 Android/iOS support contract。

本机安装了 Rust 1.98.0 的两个 mobile targets。使用以下最小 dependency：

```toml
alacritty_terminal = { version = "=0.26.0", default-features = false }
```

且 crate 只执行 `pub use alacritty_terminal::Term;` 时：

```text
cargo check --target aarch64-apple-ios       PASS
cargo check --target aarch64-linux-android  PASS
```

所以当前版本可以在两个主流 mobile target 上完成 Rust compile/type-check。它没有证明：

- 最终 app/framework link、真机执行或 upstream regression support；
- iOS 能启动任意本地 shell（平台 sandbox 本身通常不允许这种宿主行为）；
- Android local PTY/process lifecycle；
- renderer、字体、IME、keyboard/mouse 和 mobile UI 集成。

对当前 Zterm 更重要的是，terminal engine 只属于 daemon host。Android/iOS remote client
消费 Zterm-owned snapshots/deltas，不应依赖 `alacritty_terminal`。因此 engine mobile
compile 是有利的可移植性信号，不是本次选择的 release blocker；client dependency graph
保持无 engine 才是当前正式要求。若产品未来增加 mobile-local sessions，再建立独立的
engine + platform PTY + device runtime gate。

### Why Alacritty Is the Best First Gate

- 它是成熟终端项目自己使用和发布的 Rust core，不存在 Ghostty C ABI wrapper 的所有权问题。
- 它接收原始 PTY bytes，已有 bounded-by-lines 的 scrollback、resize/reflow、mode/event 与
  damage 基础，和 Zterm 的 daemon-owned authoritative model 边界相符。
- 它不是完整解决方案：Zterm 仍需拥有 projection、snapshot/delta、history anchor、资源
  reservation、output filtering 和未来 semantic input API。这正好保持 wire/domain ownership，
  不把上游私有类型泄漏到 mobile/CLI clients。
- qualification 必须先证明 Zterm 语料、Unicode、alternate screen、reply routing、2000-row
  history、240x80 hard limit、128 MiB aggregate projection reservation、256 MiB daemon RSS 与
  release-target build。通过之前不能删除 `vt100`。

### Why Rio Is Not Yet the Default

`rio-vt` 的 API 形状最接近“可嵌入 headless terminal engine”，但拆包时间太短，仍有密集的
WIP/fix/performance commits。更重要的是，当前 parser 的 OSC storage 在 inline capacity 后
可转为 heap `Vec`，没有看到与 Zterm hostile-output model 相称的 hard cap。即使关闭 graphics，
也需上游或 adapter 前置层给出可验证的 byte bound 才能进入 production shortlist。

## What Happens If Zterm Is Rewritten in C

### Whole-project rewrite

这不是仅把 terminal module 换一种语言。当前仓库约有 63,413 行 product Rust、15,696 行
Rust tests（80,436 行 tracked Rust），并深度依赖 Tokio、Iroh、prost、serde、rustls/ring、
rusqlite、portable-pty 和 Rust 的跨平台工具链。

收益只有一项明显变化：C 可以按其原生调用约定直接 include Ghostty 官方 header。代价是：

- Ghostty core 仍由 Zig 构建，C rewrite 不会消除 Zig toolchain 或 ABI/version pin；
- C 没有 `unsafe` 关键字，但 pointer ownership、buffer length、callback lifetime、threading 和
  ABI 风险仍然存在，只是默认不检查；
- Iroh 是 Rust-native stack。官方 `iroh-ffi` 面向 Swift/Kotlin/Python/Node 等 binding，并非
  可替换全部 Zterm networking semantics 的稳定 plain-C SDK；最终很可能仍要嵌 Rust，或者
  重写 P2P/QUIC/NAT traversal/relay/auth；
- Tokio concurrency、protobuf/domain model、加密、persistence、CLI、PTY abstraction 与测试
  都要重新选择和验证，远超过终端引擎迁移范围；
- 对承载未信任远程输入和 hostile PTY output 的 daemon，扩大 memory-unsafe code surface
  会降低而不是提高安全性。

结论：为一个 ABI seam 重写整个项目，成本和风险均不成比例，不推荐。

### Only a C terminal module

如果 C module 仍在 Rust daemon 进程内，Rust 侧依然要通过 unsafe FFI 调用，因此没有满足
Zterm-owned zero-unsafe policy。若把它做成独立 sidecar，Rust 可以只使用 safe IPC，但会新增
每 session process/connection ownership、framing、backpressure、state serialization、crash
recovery、version negotiation、packaging 和 latency。当前 one-session-one-PTY 架构没有足够
收益来支付这套第二分布式边界。

## Recommendation

1. 保持 Rust，不做全量或局部 C rewrite。
2. 正式撤销社区 `libghostty-rs` cutover 方案。
3. 先把 `alacritty_terminal` 与现有 `vt100` 放进同一 qualification harness；`rio-vt` 只做
   watchlist/probe。
4. 保留 Zterm-owned `unsafe_code = "forbid"`。候选依赖内部的 unsafe 需要供应链与路径审计，
   但不要求为了“全依赖零 unsafe”而虚构不可实现的承诺。
5. 若未来 Ghostty 发布官方 Rust API，再按同一 corpus/resource/release gates 重新比较；若用户
   把“必须 Ghostty”提升为最高约束，则单独评审一个 Zterm-owned audited unsafe sys crate，
   不以 C rewrite 或第三方 wrapper 掩盖该政策变化。

## Primary Sources

- Ghostty embeddable library status: https://github.com/ghostty-org/ghostty/blob/main/README.md#cross-platform-libghostty-for-embeddable-terminals
- Ghostty official C API header: https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h
- Ghostty official C demo and binding FAQ: https://github.com/ghostty-org/ghostling
- Alacritty terminal core: https://github.com/alacritty/alacritty/tree/v0.17.0/alacritty_terminal
- Alacritty terminal manifest: https://github.com/alacritty/alacritty/blob/v0.17.0/alacritty_terminal/Cargo.toml
- Alacritty CI target coverage: https://github.com/alacritty/alacritty/blob/v0.17.0/.github/workflows/ci.yml
- Rio VT README: https://github.com/raphamorim/rio/blob/v0.5.26/rio-vt/README.md
- Rio VT extraction commit: https://github.com/raphamorim/rio/commit/a0a7a0e05f8665d9e6994e5326a8d6517e16d55a
- WezTerm terminal core: https://github.com/wezterm/wezterm/tree/main/term
- asciinema virtual terminal: https://github.com/asciinema/avt
- Iroh FFI project: https://github.com/n0-computer/iroh-ffi
