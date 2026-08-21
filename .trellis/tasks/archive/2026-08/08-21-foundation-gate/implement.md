# Phase 1 Foundation Gate 实施计划

## 执行原则

- 先验证最可能改变架构的 Iroh/NAT 前提，再投入 PTY/VT；每个硬检查点只回答一个已定义问题。
- Gate 允许输出 no-go。no-go 是有效结果；Iroh 官方公共 Relay/QAD 是当前明确选择的产品 baseline，不再叠加其他 Relay 或实验 fallback。用户已明确批准把当前嵌套实验室的 A-only address-discovery gap 延期到真实双网络；B direct 或 C Relay fallback 失败仍是硬停止。
- 当前任务只有一个实施所有者，避免 `Cargo.toml`、`Cargo.lock`、CI 与报告在并行分支互相覆盖；完成后再由独立 checker 验证。
- 所有下载、容器、namespace、tmux/Herdr state 都是临时的；不连接或修改生产服务器。

## Step 0. 基线与依赖

- [x] 运行现有 source policy、format、Clippy、workspace tests/doc、cargo-deny，记录干净基线。
- [x] Step 1 先只增加网络硬检查点实际使用的 Iroh 1.0.3 最小 features、Tokio 必要 features与 Linux-only Patchbay 0.6.0；修订 Gate 允许继续后，Step 2 再精确加入 vt100 0.16.2，portable-pty 与 Unix user lookup 仍未提前加入。
- [x] 保持五个产品 crate 的 workspace 0.1.1 lockstep；不创建第六个产品 crate，不修改 Relay 发布/部署。
- [x] 用依赖/feature 与 cargo-deny 门禁确认没有混入 Iroh server、metrics 或 fast Apple datapath；已批准的 duplicate warning 有可追踪来源。

验证：

```sh
sh tests/source-policy.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
```

## Step 1. Iroh profile 与双 NAT Gate（第一个硬检查点）

### 1.1 Retained profile code

- [x] 在 `zterm-daemon` 建立最小 `transport` 模块与 `InfrastructureProfile`，从 `presets::Minimal`、Iroh 显式生产 lookup 常量与 production default Relay map 构建 endpoint。
- [x] 使用 Iroh 自己的 production Relay map 与生产 n0 DNS/Pkarr，不复制 Relay URL，也不发布 direct IP；隔离子进程证明 staging 环境变量不能改变 profile。
- [x] 暴露只读的 effective RelayMap/profile summary 给测试；正式类型中不得出现测试 NAT、staging/self-host Relay 或 hard-coded candidate address。
- [x] 用 in-memory SecretKey 与 `zterm-gate/1` ALPN 写两 endpoint、多 bidi stream、path event collector；所有 future/task 都有 deadline，失败返回 case-specific diagnostics。
- [x] profile unit/integration test 枚举 effective RelayMap，断言恰为 v1.0.3 的四个 n0 production host、每个都有官方 QAD 配置，并排除 staging 与 `relay.zenithconsulting.cn`。

### 1.2 Ephemeral Linux network lab

- [x] 在 `tests/foundation/network/` 添加最小 Dockerfile/runner，使用仓库 Rust 1.98.0，安装 Patchbay 唯一需要的 `nft`/`tc`/`iproute2`/CA 工具。
- [x] runner 使用 Colima 中一个 `--privileged --rm` 容器；容器名固定到任务范围并在开始前拒绝覆盖非本次遗留的同名容器。
- [x] 使用 Patchbay 0.6.0 创建 IX、Home NAT A/B 和 endpoint namespace，只补公网 egress；没有保留第二套拓扑实现。
- [x] 依次运行 Case A（官方生产 profile）、B（注入已知 NAT WAN candidate）、C（除 DNS 外 endpoint UDP 全部 blocked）；A 的 direct timeout 被记录后仍继续 B/C，最后才汇总 go/no-go。
- [x] 每个 Case 开三条独立 bidi stream 并校验各自 payload，记录 initial/selected path、可通过公共 API取得的 candidate 与 path event timeline。
- [x] 外部官方 Relay 只用于 Gate；未修改服务端或加入自建 fallback。Bettbox fake-IP 碰撞由测试专用 DoH 真实 A-record 注入隔离。
- [x] 无论测试结果如何都删除 container，并确认无测试 container/network/namespace/link/nft 残留。

验证：

```sh
cargo test -p zterm-daemon --test iroh_profile_gate
sh tests/foundation/network-gate.sh
```

硬检查点：

- Case A/B/C 满足 design：继续 Step 2。
- A 无 direct、B direct、C relay：把当前结果写入 `docs/foundation-gate.md`，标记为当前嵌套实验室 address-discovery evidence deferred；根据用户批准继续 Step 2，并把真实双网络补验保留到父任务 M10。
- B 失败：先排除 fixture/candidate 注入错误；仍失败则记录 transport no-go 并停止。
- C 失败：记录 Relay fallback no-go 并停止，不以 A/B direct 代替。

## Step 2. `zterm-core::TerminalModel`

- [x] 移除 core 的“只有 Phase Zero identity”表述但保留 build identity API；新增 `terminal` module。
- [x] 实现 concrete `TerminalModel` wrapper、opaque `TerminalCheckpoint`、`TerminalSnapshot`、`TerminalDelta`、`TerminalModes`、bounded side events 与 typed error；所有 vt100 字段私有。
- [x] `SafeCallbacks` 处理 DA、DSR/CPR、title、bell，拒绝 clipboard；unknown OSC/DCS/APC 不出现在 ANSI output。
- [x] 实现 ingest、resize、revision、snapshot/checkpoint、delta-or-resync、resource estimate；revision overflow 返回显式错误，不 wrap。
- [x] snapshot 显式恢复 main/alternate、size、current screen/cursor/style/input modes 和有界 recent history；delta 大于 snapshot或 baseline 不兼容时选择 resync，不建立第二套 fallback format。
- [x] 建立固定 Rust ANSI corpus；同一 case 按 1-byte、固定块与伪随机块 boundary 运行。

验证：

```sh
cargo test -p zterm-core --test terminal_corpus
cargo test -p zterm-core --test terminal_snapshot_delta
cargo test -p zterm-core
```

硬检查点：vt100 无法满足 current-screen 语义、DA/DSR、Unicode 或 snapshot/delta 等价时，保留 API/corpus，换候选后重跑；不得先进入 PTY integration。

## Step 3. `zterm-platform` PTY lifecycle

- [x] 建立 `PtyHost`/`PtySession` wrapper，portable-pty 私有类型不跨 crate。
- [x] Unix 从 effective UID 的账户记录取 home/login shell，显式覆盖 `HOME`、`SHELL`、cwd 后用 default program login argv0；无效 shell/cwd 在 spawn 前失败。
- [x] 添加 `harness = false` 的 self-child PTY integration fixture：测试进程用 `current_exe --fixture-child` 作为 deterministic child，不新增可发布产品 binary。
- [x] 覆盖输入/输出、resize、超过 PTY buffer 的高输出完成标记、根 child 自然 exit、显式 close；Windows target只编译 boundary并明确跳过 Unix lifecycle behavior。

验证：

```sh
cargo test -p zterm-platform --test pty_lifecycle
cargo test -p zterm-platform
```

## Step 4. 无 attachment drain 与慢 attachment

- [x] 在 `zterm-daemon` 添加最小 retained terminal driver：blocking PTY reader -> bounded byte channel -> ordered TerminalModel -> latest revision watch。
- [x] attachment test adapter 只能请求 snapshot/delta，不能持有 PTY handle；drop adapter与模拟 transport后 child仍运行。
- [x] 零 attachment 下运行高输出 fixture，以 fixture control marker 证明 child未被 PTY backpressure卡住。
- [x] 慢 consumer 故意暂停至多轮输出；producer/PTY继续，consumer恢复后直接 resync到最新 snapshot，断言队列没有按 revision增长。
- [x] 模拟 Iroh connection guard drop只删除 attachment；不调用 session close。

验证：

```sh
cargo test -p zterm-daemon --test terminal_drain
cargo test -p zterm-daemon --test attachment_resync
```

## Step 5. 黑盒与安全控制序列

- [x] deterministic corpus 是普通 test gate；tmux/Herdr 是显式 integration gate，不进 daemon 进程名逻辑。
- [x] tmux 用无配置、唯一 socket/server；完成交互、resize、外层 attachment drop、高输出、重新 attachment snapshot，然后只清理该测试 server。
- [x] 临时下载 Herdr v0.8.2 对应平台 asset，在下载边界校验 GitHub SHA-256；使用隔离配置目录，避免连接或停止用户现有 Herdr server。
- [x] 对 tmux/Herdr 重用同一 test adapter断言 current screen、resize与 drain；不对完整动态文本做黄金快照。
- [x] 无提示词启动 Codex 0.148.0 与临时 OpenCode v1.18.20，验证各自实际 current screen（隔离 Codex onboarding 为 main，OpenCode 为 alternate）、resize、attachment drop/resync和正常退出；未发送模型请求、未记录 transcript。
- [x] 结束时确认无测试 tmux socket、Herdr server/process或临时下载残留。

验证：

```sh
sh tests/foundation/terminal-blackbox.sh
```

## Step 6. 资源测量与 Gate 报告

- [x] 添加 `cargo bench -p zterm-core --bench terminal_state` 的稳定 `harness=false` bench，输出机器可读的 session count、viewport、scrollback、workload、elapsed、snapshot/delta bytes。
- [x] `tests/foundation/resource-gate.sh` 用平台 `/usr/bin/time` 收集 max RSS；不写自定义 allocator或常驻 monitor。
- [x] 测 1/3/16 session、120×40/512×256、10k scrollback候选与代表性 ANSI workloads。
- [x] 根据数据给出最终建议默认值；16 session与超大 viewport候选被拒，三个 session有界可用，八个 240×80/2k 饱和 session仍低于 256 MiB。
- [x] `docs/foundation-gate.md` 已列出 exact lockfile versions、A/B/C、PTY、VT、black-box、资源、平台证据、缺口和最终 go/no-go。

验证：

```sh
cargo bench -p zterm-core --bench terminal_state
sh tests/foundation/resource-gate.sh
```

## Step 7. 平台 CI 与最终质量门

- [x] CI 使用真实 hosted labels覆盖 `macos-latest`、`macos-15-intel`、`ubuntu-24.04`、`ubuntu-24.04-arm`、`windows-latest`；保持 source-policy 在 format/compile 前执行。
- [x] Unix runner执行 PTY tests；Windows编译 platform boundary并运行非 Unix tests。网络/Herdr/人工 Agent Gate保持显式，不加入普通 push。
- [x] 更新 `PHASE_NAME`/相关 placeholder文案为 foundation gate，但不改变 CLI 功能面或产品版本。
- [x] 运行全 workspace format、Clippy、test、doc、deny、diff/secret/source checks；检查没有 task之外的生产 Relay变更。

最终命令：

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
git diff --check
```

## 预期改动范围

- `Cargo.toml`, `Cargo.lock`
- `crates/core/Cargo.toml`, `crates/core/src/**`, `crates/core/tests/**`, `crates/core/benches/**`
- `crates/platform/Cargo.toml`, `crates/platform/src/**`, `crates/platform/tests/**`
- `crates/daemon/Cargo.toml`, `crates/daemon/src/**`, `crates/daemon/tests/**`
- `tests/foundation/**`
- `.github/workflows/ci.yml`
- `docs/foundation-gate.md`

明确不改：`crates/proto` schema、`deploy/relay/**`、Relay publication workflow、服务器、OpenResty、Cloudflare、installer、pairing/session persistence/CLI UX。

## 风险与回退点

| 风险 | 处理 | 回退点 |
| --- | --- | --- |
| 官方 QAD 下 Case A 仍无 reflexive/direct path | 记录地址来源、home Relay 与 path events；B隔离打洞能力，C隔离 Relay | 当前嵌套实验室不再阻塞 TerminalModel；父任务 M10 用两条真实网络补验，不增加第二套基础设施 |
| 免费公共 Relay 限速或短时不可用 | 最多按外部前提重跑一次并记录 | Gate 不承诺 SLA；正式生产前另行决定托管或自建 |
| Patchbay公网 egress不稳定 | 同一临时容器改为最小 ip netns；只保留一套 | 删除 test fixture，不触碰 Colima/服务器 |
| vt100 snapshot/mode/Unicode不等价 | 保留 wrapper/corpus替换 engine | 回退 core私有实现，不改公共协议 |
| portable-pty drop意外影响 child | 检查 handle所有权；attachment禁止持有 PTY | 回退 platform adapter，不改变生命周期语义 |
| 资源超过候选预算 | 调低未承诺默认；至少三个必须有界 | 无法满足则 no-go，不加磁盘 scrollback |
| black-box留下用户态进程 | 唯一测试 namespace/config/socket + cleanup assertion | 只停止明确测试 ID，不处理用户进程 |

## `task.py start` 前复核

- [x] PRD 决策已收敛，A/B/C 的 go/no-go含义可观察。
- [x] `design.md` 与本计划未把实验对照写成产品 fallback。
- [x] `implement.jsonl` 与 `check.jsonl` 均包含真实 spec/research entries并通过 validate。
- [x] 用户在看到最终规划摘要后的新消息中明确批准实施。
