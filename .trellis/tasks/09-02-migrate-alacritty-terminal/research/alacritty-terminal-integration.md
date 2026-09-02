# `alacritty_terminal` 迁移证据与决策记录

日期：2026-09-02

## 结论

用户已选择迁移到 Alacritty 官方维护的 `alacritty_terminal`，并明确不做候选间性能测试。
本任务固定使用 crates.io `alacritty_terminal = "=0.26.0"`、关闭默认 `serde` feature；该
package 对应 Alacritty 官方仓库提交 `94e7c8874e526b1e67b349d9ba30ddf81669119e`。

它替换的是 daemon 中的 VT parser/grid/state，不替换 `portable-pty`，也不提供字体、glyph
shaping、GPU/CPU pixel rendering、窗口或 mobile view。Zterm 继续保持一个 Session 对应一个
PTY，客户端只消费 Zterm-owned snapshot/delta/history。

## 证据输入

- 官方 crate 文档：<https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/>
- 官方 `Term` API：
  <https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/term/struct.Term.html>
- 官方 `Config`/`Osc52` API：
  <https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/term/struct.Config.html>
- 官方 `Event` API：
  <https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/event/enum.Event.html>
- 官方源码/发布：<https://github.com/alacritty/alacritty/tree/v0.17.0/alacritty_terminal>
- 本机 crates.io 解包源码：
  `$CARGO_HOME/registry/src/.../alacritty_terminal-0.26.0` 与 `vte-0.15.0`
- 当前 Zterm：`crates/core/src/terminal.rs`、`crates/daemon/src/terminal_driver.rs`、
  terminal corpus/snapshot/history/session/black-box tests，以及相关 Trellis specs。

## 已确认的上游能力

| Zterm 所需能力 | `alacritty_terminal 0.26.0` | 集成结论 |
| --- | --- | --- |
| raw PTY bytes -> terminal state | `vte::ansi::Processor` + `Term<EventListener>` | 直接使用 |
| main/alternate screen | `TermMode::ALT_SCREEN`、双 `Grid<Cell>` | 直接映射 active screen |
| bounded scrollback | `Config::scrolling_history` | 使用 Zterm 现有 2,000-row cap |
| resize/reflow | `Term::resize` | 保留 native PTY-first transaction |
| visible/history cells | `Term::grid()`、负 `Line` history index | 投影到 Zterm-owned types |
| cursor/style | grid cursor/template、cell colors/flags | 映射当前 DTO 的有限 subset |
| input modes | `TermMode` 含 app cursor/keypad、mouse、focus、paste、alternate scroll | 映射当前 `TerminalModes` |
| query/event callbacks | `EventListener` 的 `PtyWrite`、`Bell`、`Title` 等 | 必须经过 Zterm allowlist |
| damage | `Term::damage` | 不作为 attachment baseline；多个客户端不能共享消费式 dirty state |
| snapshot/state diff | 无等价于 `vt100::state_formatted/state_diff` 的 API | Zterm 自己编码 full/changed rows |

## 与当前契约不完全相同的地方

1. Alacritty primary DA 返回 `CSI ?6c`；Zterm 当前契约是 `CSI ?1;2c`。
2. Alacritty处理标准 CPR，但不处理 Zterm 当前支持的 private `CSI ?6n` CPR。
3. Alacritty没有 Zterm 当前 `ESC g -> VisualBell`、OSC 1 icon-name 和所有 unknown-sequence
   classification 的等价事件。
4. Alacritty cell 把空白和显式 default-styled space 视为相同视觉状态；当前 `vt100::Cell`
   还能保留二者的内部差异。迁移以视觉/语义状态为契约，不以该不可见差异为契约。
5. Alacritty支持更多 style、palette、OSC 8 hyperlink、Kitty keyboard 等能力，而当前 Zterm
   DTO/wire 无法忠实表达它们。本任务不能悄悄把这些能力透传给客户端。

因此适配层需要一个有界的 `TerminalIngressPolicy`。它只负责 ECMA-48 control-string/CSI
边界与 Zterm host policy，不保存屏幕或 history，不是第二个 terminal emulator：

- canonical DA/DSR/CPR 和 resize request 由 Zterm 处理；上游任意其他 `PtyWrite` 被拒绝；
- OSC 0/1/2 被转换成最多 256 source-byte 的 title/icon event；
- OSC 52 转换成不含 payload 的 rejected-effect event；
- OSC 8、其他 OSC、DCS/APC/PM/SOS 被有界消费并分类，不交给上游，避免状态、日志和
  snapshot 泄漏；
- DEC synchronized-update 2026 和当前 wire 未声明的 query/mode response 被拒绝，避免
  上游 2 MiB sync buffer 延迟执行绕过 Zterm 的逐步资源约束；
- 普通 grid/mode CSI/ESC 仍原样进入 Alacritty processor。

policy 必须跨 `ingest` chunk 保存固定小状态，所有 control buffer 有编译期 byte cap；超限
后丢弃到当前 sequence terminator，只产生一个无 payload 分类。

## 内存事实与用户决定

Alacritty 的基础 cell 是固定结构，但 `CellExtra` 包含 heap-backed zero-width `Vec<char>`、
underline color 和 hyperlink。`Cell::push_zerowidth` 本身没有长度上限。VTE 的 OSC raw buffer
最多 1,024 bytes，ANSI synchronized-update buffer 固定预留 2 MiB；后者是每个 model 的已知
内部capacity，不等同于立即增加同量RSS。

2026-09-02，用户决定不再对全部Session实施128 MiB aggregate terminal-memory admission。
因此迁移删除现有`TerminalResourceProjection`和
`ResourceLimits::aggregate_cell_projection_bytes`，不构造新的Alacritty容量公式、高水位账本
或`reserved_terminal_bytes`，也不因estimated memory拒绝create/resize。Alacritty自己的grid、
row cache、processor buffer和shrink retention由其library与allocator管理，并随整个model drop。

该决定不取消不可信PTY输出的安全边界。迁移仍必须：

- 在 ingress policy 中完全阻止 OSC 8 进入 engine，并关闭 `Config::osc52`；
- 明确禁止/归一化当前 DTO 不支持的 underline-color extra；
- 把当前 `vt100` 的约 22-byte inline cell text 行为提升为 Zterm 显式 `MAX_CELL_TEXT_BYTES`；
- 同时限制每 screen/session 的 combining-extra cell 数和总 combining bytes；达到上限时
  丢弃新增 zero-width scalar并产生有界分类，不能让 engine heap 无界增长；
- checkpoint 使用 Zterm-owned fixed/inline compact cells，只保存一个 latest active viewport；
  active-screen、size 或 checkpoint-format 变化直接 full resync；
- 保留8 live sessions、240x80 viewport、2,000-row history，以及policy/reply/event/wire byte
  caps；checked arithmetic仍保护size转换和allocation count；
- 单测验证各cap、overflow、history eviction和model/checkpoint drop；不计算aggregate bytes，
  不跑RSS/CPU/throughput benchmark，也不再维护128/256 MiB判定或性能结论。

## Crate 与线程边界

新增 host-only `crates/terminal` (`zterm-terminal`)：

- `zterm-core::terminal` 保留 Zterm-owned DTO、snapshot/delta/history/event 和 byte limiter；
- `zterm-terminal` 持有 `TerminalModel`、checkpoint/error、Alacritty adapter、policy、
  projector 和 ANSI encoder；
- daemon 同时依赖core与terminal；proto不依赖terminal。CLI是同时承载daemon的host binary，
  因此会传递包含engine，但不直接依赖terminal，且UI/wire边界只使用core/proto-owned values；
- `cargo tree -p zterm-core` 与 `-p zterm-proto` 不出现 `alacritty_terminal`、`vte` 或
  `zterm-terminal`，因此未来 remote mobile client 不需要链接 host terminal engine；
- `alacritty_terminal::tty` 和 `event_loop` 不被产品代码调用；`portable-pty` 仍是唯一 PTY
  lifecycle owner。

当前 `TerminalDriver` 已有唯一 ordered model thread、fixed-capacity no-drop byte queue、共享
model mutex、separate child interrupt 和 latest-only attachment checkpoint。Alacritty handle 是
普通 safe Rust value；本迁移不需要 Ghostty 方案中的 `!Send` actor、Zig/C FFI 或额外 writer
actor。`EventListener` 使用有界、可跨线程的 safe Rust sink；Zterm-owned crate 继续继承
`unsafe_code = "forbid"`。

## Snapshot、delta 与 history

- 私有 `ProjectedScreen` 把 Alacritty cell 映射成 fixed/inline Zterm cells、row wrap、cursor、
  active screen 和当前受支持 modes。
- full snapshot 使用 allowlisted ANSI encoder：screen metadata selector、clear/home、CUP、
  current SGR subset、printable UTF-8、cursor visibility 与明确 modes。
- merged delta 比较 attachment 的 owned projected rows；只重画 changed rows并恢复 cursor/modes。
  future revision、size/screen/version mismatch 或 delta >= full 时 resync。
- main history 用 `Line(-history_size..-1)` oldest-to-newest读取；paging 不改变 `display_offset`。
  resize、clear/shrink、capacity eviction ambiguity 或 screen identity ambiguity 推进 epoch。
- snapshot 继续 history-first、screen-second，并沿用 8 MiB frame limiter，只删除最老的完整
  history lines。

## 平台与依赖结论

- 官方 crate 是 Rust crate，MSRV 1.85，低于 Zterm 固定的 Rust 1.98；Apache-2.0 已在
  `deny.toml` 全局 allowlist。
- `default-features = false` 只去掉可选 serde；crate 仍编译其公开 Unix/Windows tty modules，
  所以必须隔离在 host crate，但 Zterm 不调用它们。
- 现有 CI 已覆盖 macOS arm64/x86_64、Linux arm64/x86_64 与 Windows shared boundary；
  `just ci-windows` 需显式加入 `zterm-terminal` tests。
- Android/iOS 不在本任务运行本地 engine。此前 minimal compile probe 只能说明编译可能；
  官方没有 mobile support contract。当前正确验收是core/proto这两个mobile-facing library
  dependency graph不含engine；host CLI不属于该验收边界。

## 不做的事项

- 不引入社区 Ghostty wrapper、`libghostty-vt`、Zig、C ABI、bindings 或 in-repo unsafe island。
- 不把 Alacritty 的 PTY/event loop 取代 `portable-pty`。
- 不实现 pixel renderer、Kitty graphics、hyperlink、advanced underline/strike/hidden、Kitty
  keyboard、selection/search UI 或 semantic surface wire v2。
- 不引入 runtime dual parser/fallback，也不保留 `vt100` 作为 dev/test oracle。
- 不运行或新增候选对比、throughput、latency、CPU、RSS benchmark；性能与新引擎RSS是明确
  的deferred risk，也不保留aggregate terminal-memory admission。

## 迁移判定

这是一次直接 cutover，而不是资格赛。只有 functional/security/resource/cross-platform build
门全部通过后才删除 `vt100`；失败时用源码 revert 回迁移前版本，不在产品中保留双引擎。
