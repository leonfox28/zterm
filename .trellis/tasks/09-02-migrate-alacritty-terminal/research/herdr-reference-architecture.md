# Herdr 参考架构调研

## 调研范围

- 上游：`herdrdev/herdr`
- 源码修订：`cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6`
- 调研日期：2026-09-02
- Herdr Cargo 版本：`0.8.2`
- vendored Ghostty 修订：`c5a21edfcbc2d5b46540ad91b7980aca31f5f1f3`
- 2026-09-02 重新核验时 Herdr remote HEAD 仍为上述 `cc88b3b...`，因此下述文件和修订
  仍代表当前实现，而不是已过期快照。

Herdr 是一个 agent-aware terminal multiplexer。它的核心用途与 Zterm 有明显重叠：后台
server 持有真实 PTY、终端状态和 session，客户端可 attach/detach，远端客户端可以通过
SSH 重新连接并恢复视图。

## Herdr 如何集成 libghostty-vt

Herdr 没有依赖 crates.io 的 `libghostty-vt` 或 `libghostty-rs`：

1. `vendor/libghostty-vt` 保存固定 Ghostty 源码，vendor manifest 记录确切 commit。
2. 根 `build.rs` 直接运行 Zig 0.15.2，把 vendored source 编译成静态
   `libghostty-vt`；支持 macOS、Linux 和 Windows target 映射。
3. `src/ghostty/bindings.rs` 保存生成的 raw C bindings。
4. `src/ghostty/mod.rs` 是 Herdr 自己维护的 Rust wrapper，覆盖 terminal、render
   state、selection、mouse/key encoding、Kitty graphics 等能力。
5. 每个 pane 的 `GhosttyPaneTerminal` 同时持有 `Terminal` 与 `RenderState`；PTY 输出
   进入 `process_pty_bytes`，然后从 Ghostty 权威状态投影 cell、cursor、history、mode
   和 side effects。

这是一条“vendor official upstream + own sys/wrapper”的路线：它不依赖第三方 Ghostty
wrapper，但 raw FFI 与 safe-surface soundness 全部由 Herdr 自己承担。Zterm 当前建议改为
先 qualification 官方 Rust-native terminal core，因此不会自动继承这条路线。

Herdr 的 `build.rs` 只映射 Linux GNU/musl、macOS 和 Windows 的 x86_64/aarch64 targets，
其他 target 会直接报 unsupported；Android/iOS 不在其当前构建矩阵。这不影响 Herdr 的
现有产品边界，因为 Ghostty state 运行在 desktop/server 端，并不随 remote client 下沉。

## 线程和同步方式

Herdr 的 raw wrapper 为 `Terminal`、`RenderState`、`KeyEncoder`、row iterator/cells 等
显式实现了 `unsafe impl Send`，注释依据是 opaque handles 只在 pane runtime 的外部同步
下使用。上层把 `GhosttyPaneCore` 放进 `Mutex`，并通过 `Arc<PaneTerminal>` 在 PTY reader、
render、agent detection、selection 和 input 路径间共享。

也就是说，Herdr 采用：

```text
Arc<PaneTerminal>
    └── Mutex<GhosttyPaneCore>
            ├── Ghostty Terminal
            └── Ghostty RenderState
```

此前评估过的社区 safe wrapper 将所有 handle 保持为 `!Send + !Sync`，但用户现在已明确
拒绝该 wrapper。若改为复制 Herdr 的 direct-official 模型，Zterm 必须自行拥有并审计
unsafe bindings、callback userdata、Send soundness 和上游升级；这与当前 workspace
`unsafe_code = "forbid"` 冲突。

因此 Herdr 证明“外部锁串行化同一个 terminal”在真实产品中有人采用，但不能证明社区
safe wrapper 的 `!Send` 可以直接忽略。Zterm 继续采用 dedicated terminal owner actor
可以取得相同的串行化效果，同时不在产品代码新增 `unsafe impl Send`。

Herdr 还有一个比 `Arc<Mutex<_>>` 更值得借鉴的所有权：Unix `PtyIoActor` 独立拥有 PTY
readiness、user writes、terminal responses 与 resize。terminal processing 返回 owned
response bytes，PTY actor 再按可写 readiness 排队/flush，因此 parser/render path 不直接
阻塞在 child write 上。Zterm 应借鉴这个 IO-owner 分离，但不能照搬其容量：Herdr 的
`SharedPtyControls.terminal_responses` 与 actor `pending_writes` 可以继续增长，Zterm 需要
固定 item/byte bound、full/failure fail-closed 和独立 child interrupt。

## PTY、状态和渲染数据流

Herdr 的 pane 启动和 Zterm 类似：

```text
child process ⇄ portable-pty ⇄ Herdr server
                               └── libghostty-vt 权威 pane state
```

Herdr 对客户端提供两种渲染编码：

### 1. SemanticFrame

完整 Herdr shell/UI 使用 semantic `PaneSurfaceFrame`：server 用 Ratatui headless backend
组合 pane、sidebar、tab、popup、cursor 和 graphics，发送 cell-based full surface 或 row
patch。客户端保留 surface baseline，再投影到实际外层终端。

输入同样是 semantic：key、text commit、mouse、paste 从 client 传到 server，server 再
依据目标 pane 的 Ghostty mode/Kitty keyboard state 编码成子进程需要的 PTY bytes。

### 2. TerminalAnsi

direct terminal attach 使用 server-side `BlitEncoder`：server 从 `FrameData` 和 per-client
baseline 生成 full/partial ANSI frame，带 sequence、width、height 和 full 标记；client
最终把 ANSI 写给 Ghostty、kitty、WezTerm 等外层终端。

因此 Herdr 也存在和 Zterm 相同的双层终端状态机：server 的 libghostty-vt 解释子进程，
外层终端解释 Herdr client 的 ANSI。它没有把原始 PTY output 透明透传到外层终端。

重新审计后，Zterm 对这一点应借鉴得更直接：Ghostty state 先投影成 Zterm-owned canonical
surface，再由一个小型 allowlisted ANSI encoder 生成 full/row patch；不要把 Ghostty 的
通用 formatter直接当作 wire codec。后者可能输出 palette、screen modes、PWD 等超出
Zterm 当前 outer-terminal virtualization 与安全 vocabulary 的序列。

## TERM 与外层终端隔离

Herdr 明确把 pane 环境设置为：

```text
TERM=xterm-256color
COLORTERM=truecolor
```

源码注释直接说明：pane 由 Herdr 自己的 terminal layer 渲染，继承启动它的外层 `TERM`
会泄漏 host terminal identity，并在 SSH/缺少 terminfo 时破坏 redraw 和 cursor movement。

这验证了 Zterm 当前规划：远端 session 必须宣告 Zterm 实际提供的 capability profile，
不能因为用户从 Ghostty 启动 CLI/daemon 就把 `xterm-ghostty` 传给子进程。

Herdr 只在确认外层为 Ghostty/WezTerm/kitty-compatible、具有精确 cell pixel size 且不在
SSH/tmux 等 blocked transport 下时启用 direct Kitty graphics；普通文本路径使用通用
terminal projection。Zterm 当前把 Kitty graphics 保持在 out of scope 更保守。

## 远程连接方式

Herdr 有两条主要远程路径：

- 用户先正常 SSH 到远端，再在 SSH PTY 内运行 Herdr；整个 Herdr client/server 都在远端。
- `herdr --remote` 在本地启动 thin client，通过系统 `ssh` bootstrap/发现远端 Herdr，
  用 SSH stdio bridge 转发远端 local socket protocol。

它依赖 SSH 的身份、加密、主机配置和可达性，并可部署/复用远端 Herdr binary。Zterm 则
拥有设备配对、自己的加密网络会话、daemon 生命周期、wire/protobuf 与面向移动控制端的
产品边界。两者的 terminal core 问题类似，但连接、信任和兼容性要求不同。

## 与 Zterm 的主要异同

| 维度 | Herdr | Zterm 当前/规划 |
| --- | --- | --- |
| 宿主状态 | server 持有每 pane 的 Ghostty state | daemon 持有每 session 的权威 terminal model |
| PTY | vendored/patch 后的 `portable-pty 0.9` | 自有 platform boundary 后的 `portable-pty` |
| Ghostty 集成 | vendored source + checked-in bindings + custom unsafe wrapper | 社区 wrapper 路线已撤销；当前推荐 qualification 官方 Rust-native core |
| 并发 | `unsafe Send` handles + `Arc<Mutex<_>>` | 推荐在单 owner actor 内保留 `!Send + !Sync` handles |
| 完整 UI wire | semantic surface/full+row patches | 当前 ANSI snapshot/delta + explicit modes/history |
| direct attach | server-side ANSI full/partial frames | daemon ANSI snapshot/delta-or-resync |
| 输入 | stable endpoint 使用 semantic input，由 server 编码到 child | 当前 CLI 依据 daemon modes 编码；未来移动 GUI 可考虑 semantic input |
| 远程 | SSH + stdio/local-socket bridge | Zterm 自有配对、加密 transport 和跨设备 wire |
| TERM | 固定 `xterm-256color` profile | 应新增明确 Zterm capability profile，禁止继承外层身份 |
| graphics | 有条件支持 Kitty graphics，payload 上限较大 | 本次迁移关闭 graphics/OSC 52，安全面更窄 |

## 对迁移设计的启示

1. Herdr 是 `libghostty-vt` 用于后台 multiplexer + attach/reattach 的强现实证据，说明
   Ghostty core 的能力与 Zterm 场景匹配。
2. Herdr 的 semantic surface 与 per-client retained baseline 证明不需要把 Ghostty 私有
   snapshot 放进 wire；应用应拥有自己的兼容协议和 resync 语义。
3. Herdr 的 semantic input 特别适合多种外层终端和未来 GUI/mobile client；这值得作为
   Zterm 后续协议方向，但本次 VT engine 等价迁移不应同时扩大 wire 范围。
4. Herdr 的固定 `TERM=xterm-256color` 进一步说明 Zterm 必须定义明确的 child capability
   identity。
5. Herdr 的 PTY actor 说明 automatic terminal replies 不应由 terminal-state owner执行
   阻塞 write；Zterm 应使用更严格有界的独立 writer actor。
6. 不直接复制 Herdr 的 custom FFI/unsafe Send。若未来把“必须使用 Ghostty”提升为最高
   约束，应把一个 Zterm-owned audited unsafe FFI crate 作为显式安全政策变更重新评审，
   不能把它描述成仍满足 zero-unsafe policy。

## 上游证据

- [Herdr repository](https://github.com/herdrdev/herdr)
- [vendored Ghostty build](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/build.rs)
- [vendored Ghostty exact revision](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/vendor/libghostty-vt.vendor.json)
- [checked-in bindgen output](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ghostty/bindings.rs)
- [custom Ghostty wrapper](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/ghostty/mod.rs)
- [pane PTY and TERM boundary](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pane.rs)
- [semantic/ANSI render state](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/server/render_stream.rs)
- [wire render and semantic input types](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/wire.rs)
- [stable endpoint contract](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/protocol/endpoint.rs)
- [remote attach implementation](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/remote/attach.rs)
- [PTY IO actor](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pty/actor/unix.rs)
