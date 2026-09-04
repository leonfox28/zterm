# CI 与发布流程审计和优化

## Goal

把 zterm 的开发和发布反馈循环收敛为一条维护者能记住、能在本地复现、失败时知道下一步的路径：日常改动在受保护的 `main` 之前由 PR CI 拦截；正式版本通过 reviewable release PR 和一个显式 publish 命令发布；tag 后不再首次发现普通代码问题，同时保留四平台原生产物、签名更新链、relay 镜像和不可变发布保证。

## Pre-refactor baseline

> Captured before the 2026-09-01 implementation. These bullets explain why the
> task existed; they are not current operating instructions. The requirements
> and acceptance criteria below describe the implemented target, while
> `docs/development.md` and `docs/releasing.md` are the current runbooks.

- 改造前 `.github/workflows/ci.yml` 监听所有 branch push、所有 PR 和手工触发。普通 branch/PR 实际运行 8 个 job，`main` push 再增加 4 个 release-mode job，共 12 个。
- 五个 Rust runner 重复 workspace version、format 和 docs，四个 Unix runner 重复无参数 CLI smoke；这些是可合并的纯重复。跨 OS checkout policy、编译和运行时测试产生独立证据，不能仅因命令相同删除。
- GitHub 当前没有 `main` branch protection/ruleset，历史上没有 PR；红色 CI 都发生在 commit 已进入 `main` 之后。
- 截至 2026-09-01，61 次 CI 中 34 次成功、18 次失败、9 次取消；从 2026-08-24 起排除取消后的失败率约 43.5%。真实失败同时包含普通格式/Clippy、Windows/平台缺陷、macOS 生命周期 race、外部 yanked dependency 和 relay secret-scan 误报。
- `detached_lifecycle` 的 macOS 失败不是应当忽略的随机噪声：测试在 socket 消失后立即重启，但 daemon lifetime lock 可能仍被退出中的旧进程持有；产品 `LocalRuntime::restart` 的现有等待也只观察 readiness/socket，因此共享同一 ownership-release race。
- `tests/secret-scan.sh` 的注释排除了 Trellis runtime state，但实现仍递归扫描 `.trellis`，把普通 Python 变量 `token = ...` 判为 secret。
- tag-triggered `.github/workflows/release.yml` 已经不运行普通 format、Clippy、workspace tests、docs、cargo-deny 或 relay test bundle。它验证 exact green source，构建四个正式资产，组装/签名，执行最终 installer，round-trip draft，attest 并发布 immutable Release。
- v0.1.9 release 用时约 17 分钟：构建/assembly 约 7 分钟，等待 `release` Environment reviewer 约 8 分钟，批准后的签名、四平台 installer 和发布不到 2 分钟。installer matrix 不是主要耗时。
- 改造前 relay image 仅监听 `release: published`，但原生 Release 由 workflow 的 `GITHUB_TOKEN` 发布；GitHub 默认不会因此递归创建新 workflow run。实际后续 release 也没有对应 relay-image run，正式 relay 发布需要显式编排。
- Herdr 值得复用的是 `just check`/`just ci` 命令所有权、PR 工作方式、缓存、timeout 和清晰 release surface；其 release commit 可直接 push `master`、tag 后仍跑 Nix/docs checks，并且没有 zterm 等价的 exact-green gate、detached signature、protected signing、installer matrix、late draft 或 relay image contract。

## Requirements

### R1 — PR-first integration

- 日常实质性改动使用 branch → PR → required CI → human merge；直接 push `main` 不再是正常开发路径。
- `ci.yml` 只监听 PR、`main` push 和手工触发，不再同时为同一个 PR branch 启动 branch-push 和 pull-request 两份完整运行。
- 提供一个固定名称的 aggregate required check；仓库设置文档必须给出 `main` protection 的最小配置，并适合只有一名维护者、不强制他人 approval 的仓库。

### R2 — One local command owner

- 新增仓库级 `justfile`，至少提供 `doctor`、`check-fast`、`check`、CI profile recipes、`release-prepare <version>` 和 `release-publish <version>`。
- `just check` 是普通实质性改动的 push 前权威入口；由 operator 确定性生成的两文件 release commit 只跑 focused version/lock validation，完整 gate 由其必经 PR CI 拥有。CI 调用相同 recipes/underlying scripts，不在 YAML 复制一套容易漂移的命令。
- `doctor` 对 Rust、just、ShellCheck、actionlint、cargo-deny、gh/jq 和需要 Docker 的 hosted-only 能力给出明确结果与安装/复现提示；不得静默跳过一个 required owner。
- 本地单平台检查明确列出 hosted-only evidence，不能冒充 macOS/Linux/Windows 或另一架构的运行结果。

### R3 — Conservative CI deduplication

- workspace version 和 formatting 各有一个 canonical owner；docs 只构建一次；无参数 CLI smoke 最多保留一个 Linux 和一个 macOS owner。
- `sh tests/source-policy.sh` 继续在每个 Linux、macOS、Windows Rust matrix checkout 后、format/compile 前运行，符合 cross-platform spec。
- 第一轮保留现有 Unix 原生 Clippy/tests 和 Windows shared-boundary tests，不切换 nextest，不用 path-classifier 掩盖变更；收集 20–30 次新流程数据后再决定是否缩减非主架构测试。
- 四目标 main release-readiness 继续证明 macOS 13、glibc 2.28、CPU 架构和 exact-SHA buildability，但共享仓库脚本，避免 CI 与 formal release 的 shell 逻辑分叉。
- Rust/依赖缓存和 per-job timeout 只优化获取与增量构建；正式 release 二进制仍从 frozen tag 构建，普通 CI artifact 不得升级为正式资产。

### R4 — Gate reliability and diagnostics

- 修复 restart ownership-release race：restart 在 bounded deadline 内同时等待 readiness 消失、socket 清理和 daemon lock 释放；测试通过 public `LocalRuntime::restart` owner 证明该契约。
- relay secret scan 不再扫描非发布输入 `.trellis`，但保留对 tracked source/deployment material 的真实 credential patterns。
- 每个 CI job 名称和 step summary 标出 owner、失败含义和本地复现 recipe；aggregate gate 对 failed/cancelled/skipped 给出稳定结论。
- CI 采用显式 timeout、取消旧 PR head 和 pinned cache/action dependencies；runner 排队时间与执行时间不得混为一谈。

### R5 — Two-phase release operator

- `just release-prepare <version>` 从干净且同步的 `main` 创建本地 release branch，验证 canonical newer SemVer，显式运行 Cargo 的 workspace lockfile 更新，再以 locked metadata、workspace-version 和 exact changed-file inventory 做 focused validation，创建 commit，push branch 并打开 release PR；完整 format/Clippy/test/docs/dependency gate 由随后必经的 release PR CI 拥有，任何检查失败都不得 push tag 或公开 release。
- release-time 当前版本文本从 README/docs/test fixture 中移除或动态派生，使正常版本升级只需修改 Cargo version/lock，而非手工搜索多处数字。
- `just release-publish <version>` 只能从干净、与 `origin/main` 相同的 `main` 执行；它重新检查版本、tag/release vacancy、exact-SHA successful main CI 和 branch protection precondition，随后创建 annotated tag、push，并显示/跟踪 release run。
- 两个命令都 fail closed，不 force push、不覆盖 branch/tag/release，不自动删除失败现场，并输出明确恢复动作。

### R6 — Artifact-only tag workflow

- tag 后不运行普通 lint、workspace tests、docs 或 dependency policy；只执行 source/tag validation、正式构建和拿到最终 bytes 后才可能完成的 release verification。
- 四个 native build job 只构建/执行/检查 shipped `zterm` binary；私有 release tool 的 archive/prepare owner 集中在 Ubuntu assembly，避免四个平台重复编译非发布工具。
- 保留 generated installer 的单一 Ubuntu ShellCheck、protected Ed25519 signing、四目标 POSIX syntax + authenticated install/negative fixture、signed inventory re-verification、late draft round-trip、attestation 和 immutable assertion。
- 删除 tag workflow 中已由 exact-green CI 拥有的 `tests/release/static.sh` 重跑和 installer matrix 中被实际 fixture execution 覆盖的独立 Python `py_compile`；不得删除具有不同 trust-boundary owner 的 verify。

### R7 — Explicit relay publication

- `relay-image.yml` 支持受控 `workflow_call` 和现有 manual development dispatch；正式发布不再依赖 `release: published` 的隐式事件。
- 原生 release 成功后显式调用 relay reusable workflow，传入 frozen commit、exact tag 和 prerelease classification；stable 仍发布 version + `latest`，prerelease/manual 仍只进入 `zterm-relay-dev`。
- relay 发布失败使总 release run 明确失败并可单独重试；不得覆盖 native Release、增加第二签名格式或声称 GitHub Release 与 OCI registry 能跨服务原子提交。

### R8 — Documentation and maintainability

- `docs/development.md` 解释 CI、日常 branch/PR/push、local recipes、hosted-only evidence 和常见失败复现。
- 新增或收敛一份 release operator 文档，解释 prepare PR、main CI、tag、build、approval、sign/install/publish、relay、失败恢复和各阶段典型耗时。
- README 只链接权威说明并保留简短命令，不复制易漂移的完整流水线。
- workflow 的策略逻辑尽量由可 ShellCheck/测试的仓库脚本拥有；相关 Trellis distribution/relay/cross-platform spec 与新 owner 同步。

### R9 — Post-release prepare reliability

- 2026-09-04 对 v0.1.10–v0.1.14 的实际发布复盘确认：`cargo metadata --no-deps` 不会刷新 workspace package 在 `Cargo.lock` 中的版本；operator 把它当生成动作，而 fixture 又伪造了该副作用，造成每次 prepare 首次失败。这是实现与测试证据不匹配，不是发布状态机缺失。
- lockfile 生成动作必须是显式 `cargo +1.98.0 update --workspace`；随后 `cargo +1.98.0 metadata --locked --format-version 1 --no-deps` 只能验证，不得被测试替身赋予真实 Cargo 不具备的写入行为。
- changed-file inventory 失败必须同时打印 expected 与 actual；prepare 不再在已受 release PR CI 保护的路径上重复运行本地 `just check`。
- 只允许从当前 `release/vVERSION` 上的 clean、单一、exact release commit 续跑不确定的 branch push/PR create：commit parent 必须是当前 `origin/main`，版本、message 和两文件 diff 必须匹配；同 SHA 的 remote branch/PR 可复用，任何 divergence 都 fail closed。dirty/partial branch 仍保留给人工诊断，不自动修复。
- 不引入通用状态文件、Cargo.lock 自定义 parser、生产临时 worktree 或 publish 状态机重写；现有 tag/Release vacancy、exact-green main、签名与 immutable publication 契约保持不变。

## Acceptance Criteria

- [ ] AC1：维护者只需记忆 `just check`、`just release-prepare VERSION`、`just release-publish VERSION` 三个主入口，`just --list` 和文档能解释其边界。
- [ ] AC2：一个故意制造的 format、Clippy、workspace-version、release static 或 shell error 能在 push 前由相同 owner 失败，并给出安装缺失工具或复现提示。
- [ ] AC3：PR branch 的一次 push 不会同时产生 branch-push 与 pull-request 两份完整 CI；`main` push 仍生成 exact-SHA integration/release-readiness 证据。
- [ ] AC4：version/fmt/docs 不再跨五个 runner 重复，CLI smoke 不再跨四个 Unix runner 重复；source-policy 和平台专属 compile/test evidence 保留。
- [ ] AC5：缓存键包含 toolchain/lock/platform 边界，所有 job 有合理 timeout，旧 PR head 会取消，最终 `CI gate` 可作为 required status check。
- [ ] AC6：macOS restart regression 在 public owner 下稳定通过，且停止等待必须观察 daemon lock 释放而非仅观察 socket disappearance。
- [ ] AC7：Trellis 中普通 `token` 变量不再触发 relay secret scan，已覆盖的 private-key/AWS/GitHub token fixtures仍会失败。
- [ ] AC8：release preparation 只改动预期 version/lock 文件并创建 PR；在 dirty tree、落后 main、非法/非递增版本、已有 branch/tag/release 或 preflight failure 时无 tag/publication side effect。
- [ ] AC9：release publish 在 exact main SHA 没有成功 CI 时拒绝；成功路径只创建一个 annotated tag，并给出对应 run URL/等待 reviewer 提示。
- [ ] AC10：tag workflow 没有普通 fmt/Clippy/unit/docs/deny 重跑；正式四目标 binary、manifest/signature、installer、SBOM、attestation 和 immutable Release 契约保持兼容。
- [ ] AC11：四个 native jobs 不再重复构建 `zterm-release-tool`；assembly 统一产生 deterministic archives，下载后的 binary bytes、identity、floor 和 manifest inventory仍通过现有 release-tool tests。
- [ ] AC12：正式 native Release 通过显式 reusable call 启动同 commit/tag 的 relay image publish；static/publication tests覆盖 stable、prerelease、manual 和禁止隐式 trigger。
- [ ] AC13：任一 release verification 失败都不会替换既有 tag/Release；relay retry 的非原子边界和恢复命令被文档明确说明。
- [ ] AC14：actionlint、ShellCheck、focused lifecycle/release/relay tests、workspace tests/docs/dependency checks和一次非破坏性 operator fixture均通过。
- [ ] AC15：branch-protection checklist 指定稳定 `CI gate`、禁止 direct/force push，并说明 workflow 合入后由管理员启用；不假装代码能自动修改 repository settings。
- [ ] AC16：真实 Cargo 1.98 fixture 证明 `cargo update --workspace` 更新 lockfile，随后 locked metadata/workspace-version/exact inventory 通过；fixture 不再伪造 metadata 写锁文件。
- [ ] AC17：prepare 成功路径不调用 `just check`，只提交 `Cargo.toml` 与 `Cargo.lock`；完整 gate 仍由 release PR CI 和合并后的 main CI 拥有。
- [ ] AC18：clean exact release commit 在 push/PR 响应不确定后可安全重跑；同 SHA remote branch/开放 PR 被复用，remote/commit/message/version/inventory 任一不一致均拒绝。
- [ ] AC19：inventory 错误展示稳定的 expected/actual 文件集合，partial dirty branch 不被自动提交、清理、push 或转成 tag。

## Key Decisions

- 2026-09-01：采用 branch + PR + required CI + human merge；`main` 不再作为日常直接 push 分支。
- 2026-09-01：第一轮采用保守精简，先删除纯重复、修复 flake/误报并增加缓存；跨平台 test matrix 的进一步缩减延后到实际使用数据。
- 2026-09-01：release commit 同样通过 PR；发布入口明确拆成 `release-prepare` 与 `release-publish`，不为一条同步命令 bypass protected main。
- 2026-09-01：tag 后允许 release-specific final-artifact verification，但不重复普通 CI suite。
- 2026-09-01：保留 zterm 强于 Herdr 的 exact-green、签名、protected approval、installer、draft round-trip、immutable 和 relay contracts。
- 2026-09-04：把 release-prepare 收敛为“确定性两文件生成 + focused validation + PR CI”，并只为 exact clean commit 增加有限续跑；拒绝用通用状态机掩盖局部 Cargo 命令与测试替身错误。

## Out of Scope

- 第一轮改用 nextest、立即删除非主架构 tests、加入复杂 path-based job classifier，或引入 self-hosted/付费 runner。
- Herdr 的 scheduled preview channel、自动关闭 issue、多语言版本文档快照或 mutable `latest.json` 工作流。
- 修改终端、网络、认证或 Session 行为；唯一产品代码例外是修复已由 CI 证实的 daemon restart ownership race。
- 在本任务中创建真实版本 tag、公开 GitHub Release、推送生产 GHCR image，或自动更改 branch protection/environment/immutable repository settings。
- 以缓存的普通 CI binary 代替 frozen-tag formal build，或削弱现有签名/installer/update/rollback contract。

## Risks and Deferred Follow-up

- `just`、actionlint 和 ShellCheck 是新的显式本地工具要求；`doctor` 和 bootstrap 文档必须让缺失状态可理解，不能把工具安装失败伪装成代码失败。
- GitHub-hosted runner queue 可能仍造成长 wall-clock；cache/timeout只能减少实际执行和挂死，不能保证排队时延。
- GitHub Release 与 GHCR 不支持一个跨服务原子事务；本任务选择 native 成功后显式发布 relay，并把失败显示/重试作为 owner。
- 新 CI 在实际 20–30 次 PR/main runs 后复盘耗时、flake 和平台独立命中率，再决定第二轮是否把某个非主架构降为 compile-only。
- 第一次真实 release 是 operator UX 和 reusable relay publication 的最终线上证据；本任务只做无生产副作用的本地/静态 fixture，真实失败使用下一个版本而不修改 immutable asset。
