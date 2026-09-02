# libghostty-vt 与 kitty 调研

## 结论

两者不是对称的可嵌入库候选：

- `libghostty-vt` 是 Ghostty 面向嵌入场景公开的无窗口 VT 核心，提供 C API，覆盖
  parser、screen、scrollback/reflow、modes、输入编码、selection、render state 和有界
  effects。
- kitty 是完整终端应用，同时定义了一组被其他终端采用的协议。它的 parser、screen
  和 history 虽然源码可见，但内部核心与 Python 对象、GPU cell、graphics manager、
  callback 和应用生命周期紧耦合，并没有受支持的 `libkitty` ABI。

因此：

1. 不把 kitty 应用或内部 screen/parser 嵌入 Zterm。
2. 把 Kitty keyboard/graphics 等协议当作可独立采用的规范；这不要求链接 kitty，
   `libghostty-vt` 自身也实现了部分 Kitty 协议。
3. `libghostty-vt` 是目前更适合替换 `vt100` 的候选，但只能进行带门禁的迁移，
   不是依赖替换式的 drop-in upgrade。

## 上游证据与比较

| 维度 | libghostty-vt | kitty | 对 Zterm 的含义 |
| --- | --- | --- | --- |
| 产品形态 | 官方公开的 embeddable VT C API | 完整终端应用 + 协议规范 | Ghostty 的集成边界与需求同构；kitty 没有库边界 |
| 核心能力 | screen、scrollback、reflow、modes、mouse/key encoding、selection、render state、effects、snapshot | 成熟 parser/screen/history、GPU renderer、字体与图形集成 | 两者应用内能力都强，但只有前者对外提供相应 API |
| API 稳定性 | 功能成熟，但 C API 明确仍可能 breaking；没有独立 tagged lib 版本 | 内部 C/Python API，不承诺嵌入兼容性 | 两者都必须固定源码；kitty 甚至缺少可依赖的外部 ABI |
| 线程模型 | C render state 支持外部锁下的双线程更新；当前安全 Rust wrapper 保守地全部 `!Send + !Sync` | screen/parser 绑定 Python 对象与应用线程/锁 | Ghostty 需要单 owner actor；不能给 wrapper 手写不安全 `Send` |
| 构建 | Zig 构建，可产静态/动态库；上游 CI 覆盖 macOS、Linux、Windows、iOS、Android | Python + C + Go + OpenGL 及 Harfbuzz、font/render/image 等依赖；官方不推荐 cross compile | Ghostty 的复杂度可控但需引入固定 Zig；kitty 明显超过 headless daemon 所需 |
| 平台 | VT core 的 CI 包含桌面、Windows 和移动 target | 预构建/应用重点是 macOS、Linux | Ghostty 与当前 host 及未来平台路线更一致；移动控制端仍无需链接它 |
| 许可证 | Ghostty MIT；当前 Rust wrapper MIT OR Apache-2.0 | GPL-3.0 | Ghostty 符合当前 allowlist；嵌入 kitty 会先触发项目许可证决策 |
| 协议扩展 | Kitty keyboard；wrapper 的 Kitty graphics Rust surface 可关闭，native feature 裁剪能力仍需单独审计 | Kitty 协议的参考实现 | 可在 Ghostty 上按产品需求逐项启用协议，不需要采用 kitty 应用 |
| 资源/安全 | scrollback 字节/物理行限制、unknown sequence 和 clipboard 上限、effects opt-in、title report 默认禁用 | 完整应用内有自己的限制与策略 | Ghostty 更容易映射到 daemon 的拒绝/有界副作用策略，但仍要由 Zterm 再设总预算 |

主要上游入口：

- Ghostty 的 [`vt.h`](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt.h)
  和 [`terminal.h`](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/terminal.h)
  说明了公开能力、effects 和资源限制。
- Ghostty 的 [`render.h`](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/render.h)、
  [`modes.h`](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/modes.h) 与
  [`snapshot.h`](https://github.com/ghostty-org/ghostty/blob/main/include/ghostty/vt/snapshot.h)
  给出了渲染同步、终端模式及内部快照契约。
- Ghostty 的 [CMake 说明](https://github.com/ghostty-org/ghostty/blob/main/dist/cmake/README.md)
  和 [CI targets](https://github.com/ghostty-org/ghostty/blob/main/.github/workflows/test.yml)
  是构建与平台证据。
- kitty 的 [overview](https://sw.kovidgoyal.net/kitty/overview/)、
  [build 文档](https://sw.kovidgoyal.net/kitty/build.html)、
  [`screen.c`](https://github.com/kovidgoyal/kitty/blob/master/kitty/screen.c) 和
  [`vt-parser.c`](https://github.com/kovidgoyal/kitty/blob/master/kitty/vt-parser.c)
  表明其应用结构和内部耦合。
- Kitty 的 [protocol extensions](https://sw.kovidgoyal.net/kitty/protocol-extensions/)、
  [keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) 和
  [graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) 可以独立于其应用实现。

## 与当前 Zterm 架构的适配

### 外层终端不是共享实例，而是串联状态机

用户从 Ghostty、kitty、iTerm2 或其他终端运行 Zterm CLI 时，实际数据流是：

```text
远端应用 → PTY bytes → daemon 的权威 VT model → Zterm snapshot/delta ANSI
         → CLI stdout → 用户的外层终端 parser → 像素

用户键盘/鼠标 → 外层终端编码 → Zterm CLI 解码/路由
              → Zterm input bytes → PTY → 远端应用
```

如果外层应用是 Ghostty，确实可能在链路两端各运行一次同源 VT 代码，但它们位于不同
进程、不同内存和不同职责边界：daemon 实例解释远端应用，Ghostty 实例解释 Zterm CLI
输出。不存在指针、allocator、全局状态或 ABI 的嵌套/共享，和在 Ghostty 中运行
`ssh`、`tmux` 的结构相同。

风险来自 terminal-over-terminal 的协议投影，而不是重复链接：

- daemon 从任意输入语义重新合成 ANSI 时，只有 Zterm wire/renderer 明确表达的能力能
  到达外层终端；graphics、OSC、shell integration 等不能假设透明穿透。
- 外层终端负责采集键盘/鼠标，远端 model 决定子应用当前需要的 mouse、focus、
  bracketed paste 和 application cursor/keypad 模式；CLI 必须显式翻译并在退出时恢复
  外层状态。
- daemon 的 DSR/DA 等 query reply 必须写回远端 PTY，不能把 query 发到 CLI stdout
  让外层终端回答，否则会把外层终端能力错误暴露给远端应用。
- daemon history 是会话权威历史，外层终端也会形成一份显示历史；CLI 需要继续明确
  主屏滚轮、alternate screen 和远端 mouse reporting 的所有权。
- 远端 shell 的 `TERM`/terminfo/DA 必须描述 Zterm 实际提供的能力，而不是继承启动
  daemon/CLI 的 Ghostty 或 kitty 身份。否则应用可能输出 Zterm 未声明支持的私有序列。

因此 formatter 必须以一个跨终端的、受测试的 ANSI 子集为目标，不能因为外层也使用
Ghostty core 就依赖 Ghostty 私有行为。兼容测试至少应同时覆盖一个 Ghostty 外层和一个
非 Ghostty 的 xterm-compatible 外层。

### 能保留的边界

- `portable-pty` 继续只管理进程、PTY、resize 和退出。
- `TerminalModel` 继续是宿主权威语义边界；Ghostty 类型不进入公开领域 API。
- snapshot/delta/history 继续使用 Zterm 自有 wire，客户端和移动控制端无需链接
  `libghostty-vt`。
- DSR/DA 回复、bell/title/clipboard/unknown-sequence 等仍经过 daemon 的显式、有界
  side-effect policy，而不是直接透传。

### 不能原样保留的实现

#### 1. 所有权与线程

当前 `SharedTerminal` 把 `TerminalModel` 放进 `Mutex`，多个 daemon 路径可加锁查询；
当前安全 wrapper 的 terminal/render/tracked handles 则是 `!Send + !Sync`。迁移不能通过
人工实现 `Send` 绕过这个契约。

推荐改为 dedicated terminal actor：

- 在 owner 线程内 create、ingest、query、resize 和 destroy Ghostty terminal；
- attachment 通过有界 command channel 请求 Zterm-owned snapshot/delta/history 值；
- actor 回传的都是拥有所有权的 Rust 数据，不回传 Ghostty refs/pointers；
- PTY bytes 使用独立高优先级/有界策略，客户端查询不能阻塞持续 drain。

#### 2. Delta/checkpoint

当前 `vt100` 提供基于保存 screen 的 `state_diff`。`libghostty-vt` 没有等价的稳定外部
API。Ghostty dirty render state 适合驱动 renderer，但不能直接替代 Zterm 当前 ANSI
delta/recovery wire。

迁移必须由 Zterm 定义 semantic checkpoint 和 ANSI delta synthesis；初期允许在无法
证明小 delta 正确时返回 resync snapshot，但必须用 corpus 验证正常小更新不会退化为
持续全量恢复。

Ghostty binary snapshot 可保存 parser continuation、screen 和 history，但 format v1
明确不保证二进制兼容。它只能用于本版本内部实验/测试，不能进入 Zterm wire 或长期
持久化。

#### 3. Scrollback 与资源预算

Ghostty 提供最大字节数与最大物理行数，但分页分配及 reflow 会使限制成为估算值。
Zterm 仍需在 adapter 层维护：

- 外部输入、effects 和响应的硬上限；
- 可观测的 history epoch/revision/cursor 语义；
- 面向 wire 的固定单元格/字节 admission；
- resize/reflow 后的分页一致性和 alternate screen 不污染主历史。

#### 4. 安全副作用

迁移首期应关闭 Rust wrapper 的默认 `kitty-graphics` feature，并将 runtime image
storage 设为零。当前 wrapper build script 不会因此自动裁剪 Ghostty native feature；
实际静态归档、资源与输入处理面必须审计，不能宣称已完成 native 最小化。只注册现有
需求所需的 bounded callbacks。OSC 52、文件/共享内存图像、title report 及未识别
载荷继续拒绝或有界记录。

## Rust/C 集成方式

### 推荐：固定社区 safe wrapper，并包在 Zterm adapter 后

候选是 `libghostty-rs` 的 `libghostty-vt-sys` + `libghostty-vt` 两层：

- safe crate 已覆盖 terminal、render state、modes、mouse/key、tracked refs、selection、
  formatter 和 snapshot；
- unsafe 集中在 wrapper，而 Zterm 产品 crate 可以继续 `unsafe_code = "forbid"`；
- probe revision 已修复 clipboard callback 的空/null/binary-data问题，但仍有 open issue
  #70 的 safe buffer-capacity soundness bug；因此它只能做 probe，final pin必须包含上游修复；
- 许可证 MIT OR Apache-2.0 与当前依赖策略相容。

集成时必须：

1. 固定 wrapper git revision、其 Ghostty commit 和 Zig 版本/校验和，不能只写语义版本；
2. `default-features = false`；
3. 为 CI/release 预取或 vendor Ghostty 与 Zig，使用 `GHOSTTY_SOURCE_DIR`、
   `GHOSTTY_ZIG_SYSTEM_DIR` 等显式离线输入，禁止构建时隐式联网；
4. 审计 wrapper `build.rs`、生成的 SBOM/license notices 和各 release target 产物；
5. 只在 Zterm-owned narrow adapter 后使用 wrapper；Ghostty general formatter不直接生成
   wire，先投影 canonical surface，再由 allowlisted Zterm encoder生成 ANSI。

调研时 `libghostty-rs` 仓库 revision
`5988a0b78b4aa804d1c12e66bbfe662bd97d81c0` 的 crate 仍标为 `0.2.1`，但它与 crates.io
同名版本选择的 Ghostty commit 和 Zig 版本不同，因此不能把 nominal crate version
当作完整供应链 pin。

若没有修复 #70 的精确上游 successor revision，结果是 Gate A no-go，而不是自动切换到
Zterm 自建 raw wrapper；后者会改变用户已确认的 unsafe policy，需要重新规划。

### 备选：自建 raw sys crate + safe wrapper

只有在 spike 证明社区 wrapper 缺少必要 API、无法实现可重复 release，或存在不能及时
修复的 soundness 问题时采用。它能完全控制 bindgen/build/ABI，但会把 allocator、callback、
thread、unwind、UTF-8、slice 和析构的 unsafe 审计永久转移给 Zterm，维护成本显著更高。

### 否决：直接在 core 内写 extern 或嵌入 kitty 内核

- 直接 `extern` 会使 FFI 细节扩散，违反当前 layer 和 unsafe 约束。
- kitty 没有稳定、独立的 terminal-core ABI；抽取其内部核心等同维护一个 Python/GPU
  强耦合的 GPL fork，不符合项目跨平台与最小宿主 daemon 目标。

## 本地 spike 结果

在 macOS arm64、workspace Rust 1.98.0 上，用固定 `libghostty-rs` revision 和 Zig 0.16.0：

- `cargo check --no-default-features` 通过；
- wrapper 的 13 个 unit tests 与 17 个 doctests（2 ignored）通过；
- release 静态链接构建通过；
- 自建 probe 验证 create/drop、write、scrollback、mouse/focus/SGR modes、PTY reply、bell、
  formatting 和 history selection。

这些结果只证明一个 macOS arm64 host 的 API 与工具链可行性。四个首发 release targets、
macOS/glibc deployment floor、Windows shared contract、release size、离线构建和完整 Zterm
corpus 仍是迁移门禁，不能从上游 CI 或本次 probe 推定为已经验收。

详细输入与输出见 `local-integration-spike.md`。

## 建议的迁移门禁

先做 gated integration spike，再决定是否删除 `vt100`：

1. 先取得修复 issue #70 的 exact wrapper/Ghostty/Zig pin，完成 soundness review、离线、
   关闭 wrapper 默认 graphics surface 的四目标
   构建，并记录实际 native feature/archive surface。
2. 建立 terminal actor、独立 bounded PTY writer和 narrow adapter，只跑新引擎 probe，
   不改 wire。
3. 用现有 corpus 做 semantic differential；补齐 modes、effects、resource bounds、history、
   resize/reflow 与 alternate-screen tests。
4. 实现 canonical surface + allowlisted ANSI checkpoint/delta，验证断线恢复、小更新效率与
   OSC/DCS/APC/hyperlink/palette payload隔离。
5. 跑真实 PTY drain/detach/reattach/close 测试与目标平台构建。
6. 所有门禁通过后一次性切换并删除 `vt100`；失败则保留当前实现，不引入长期双引擎。
