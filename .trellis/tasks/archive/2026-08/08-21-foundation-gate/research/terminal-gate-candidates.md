# PTY 与 TerminalModel Gate 候选复核

## portable-pty 0.9.0

`portable-pty` 提供统一的 PTY system、master reader/writer、resize 和 child wait/kill 边界，适合把 Unix PTY 与未来 Windows ConPTY 隔离在 `zterm-platform`。Gate 使用两种 child：

- deterministic fixture：不读取用户 shell rc，精确验证输入、输出、resize、高输出完成标记、自然退出和显式关闭；
- 当前账户 login shell：只做人类可识别的真实 shell smoke，验证默认 shell/cwd 约定，不把用户 rc 输出写成快照。

attachment 永远不持有 PTY master、reader、writer 或 child handle。它只持有 terminal snapshot/delta 订阅，所以丢失 attachment 或 Iroh connection 没有可调用 PTY kill/HUP 的对象。

## vt100 0.16.2 可复用能力

- `Parser<Callbacks>` 顺序解析 bytes，并把未处理的 CSI/OSC 等交给回调；这允许 zterm 捕获 DA/DSR、title、bell 与禁止的 clipboard 请求。
- `Screen` 同时维护 main/alternate grid、固定行数 scrollback、cursor、style、application cursor/keypad、bracketed paste 与 mouse mode。
- `state_formatted()` 与 `state_diff()` 可生成受控 ANSI 状态/差量；`Screen` 可以 clone，适合作为 attachment 的私有 checkpoint。
- `unicode-width 0.2.1` 处理宽字符与组合字符，但具体等价性仍必须由固定 corpus 验证。

## 已知限制与 Gate 处理

- `state_formatted()` 是库级便利方法，不自动构成 zterm 完整协议。Gate wrapper 必须显式包含当前 screen 类型、尺寸、revision 和有界近期 history，并用 fresh parser 验证 snapshot + delta 的语义等价。
- DA/DSR 不是 screen mutation；通过 `Callbacks::unhandled_csi` 识别并生成写回 PTY 的受控 reply。未识别 OSC/DCS/APC 只记录有界 side event 或丢弃，不能原样发给控制端。
- vt100 的 scrollback 上限在 parser 创建时固定，没有直接的全局动态 trim API。Gate 先测 16 × 10k/512×256 的实际内存；若无法在 256 MiB 候选预算中可靠计量和约束，则不以“以后再优化”放行，必须在同一 `TerminalModel` wrapper/corpus 后换实现或采用可证明的固定 per-session 预算。
- 现在只有一个 VT 实现，不建立 trait-object/插件系统。`zterm-core::TerminalModel` 是具体 wrapper，候选库与 checkpoint 字段均保持私有；公共方法、corpus 与 snapshot/delta 结果才是可替换契约。

## snapshot/delta 等价方法

1. 将 corpus 前缀喂给 authoritative model，取得 `(snapshot, checkpoint, revision)`。
2. 把 snapshot 应用到同尺寸 fresh client parser。
3. 将后缀分成不同 chunk 边界喂给 authoritative model；从 checkpoint 生成合并 delta。
4. 把 delta 应用到 client parser。
5. 比较当前 screen 类型、尺寸、每个 visible cell、cursor、style 与 input modes；不得只比较 ANSI bytes。
6. 随机改变 chunk boundary 与连续 resize，确保 parser 结果不依赖 PTY read 分块。

慢 attachment 测试只保留 latest revision 通知。故意暂停 attachment consumer 时，PTY reader 与 TerminalModel 仍前进；consumer 恢复后丢弃旧水位并获取最新 full snapshot，不回放无界 delta 队列。

## 本机现状

- macOS arm64 26.6.2
- tmux 3.7c 已安装
- Herdr 0.8.2 已安装。自动黑盒不信任 PATH 上同名程序，而是临时下载官方 `v0.8.2` macOS arm64 release asset（tag commit `9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c`，GitHub asset SHA-256 `a5d4f4d504d8b309c91f811050559300faba31258425f53c50852fc96f6ae574`）并在临时目录执行，结束后删除；这仍满足“固定提交 Herdr”，且比猜测本机二进制来源可复现。
- Codex CLI 0.148.0 可用。OpenCode 当前不在 PATH；人工 smoke 使用临时下载的官方 `v1.18.20` `opencode-darwin-arm64.zip`（GitHub asset SHA-256 `b483e547c029b4f0ba381f0d0c5b420bec48c24c2bbec1fb7f22252bae83da46`），不做全局安装。若其 TUI 需要账户或 provider，只验证启动、全屏切换、resize 与退出，不发送真实 Agent 请求。

## 证据

- [portable-pty](https://github.com/wezterm/wezterm/tree/main/pty)
- [vt100 0.16.2](https://docs.rs/vt100/0.16.2/vt100/)
- [vt100 callbacks](https://docs.rs/vt100/0.16.2/vt100/trait.Callbacks.html)
- 父任务 `research/terminal-reconnect-state.md`
- 父任务 `research/shell-session-startup.md`
