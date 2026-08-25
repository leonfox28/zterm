# Implement signed installer and release lifecycle

## Goal

完成第一阶段 M9：让不具备 Rust、Node/npm 或管理员权限的 macOS/Linux 用户，只通过项目官方 GitHub Release 的签名原生产物完成安装、显式升级/回滚和卸载。该任务交付 M10 与用户最终统一验收所使用的唯一正式安装入口。

## Background

- 父任务已经确定 direct installer + 手动 `zterm update`，不采用 npm、Homebrew、mise、Nix、branch build 或 GitHub Actions 临时 artifact 作为第一阶段官方渠道。
- 任务启动时官方仓库 `leonfox28/zterm` 只有 Relay release，尚无 zterm 客户端原生 release workflow、安装签名 secret 或 protected release environment。
- 当前 `install/` 只有保留边界；M7–M8 的实现与现有 hosted matrix 先独立收口，正式安装后的 public CLI/remote Session 证据由 M10 负责，不阻塞 M9 开始。
- 用户明确不进行中间开发版安装；M9 与 M10 完成后才使用正式 installer 统一验收。

## Requirements

- 为 `aarch64/x86_64-apple-darwin` 与 `aarch64/x86_64-unknown-linux-gnu` 生成版本一致、可追溯的压缩 artifact；release metadata、binary version、target、checksums、SBOM 与 provenance 必须相互一致。
- GitHub Release 必须先形成完整 draft，上传全部资产并通过验证后再发布；启用并验证 immutable release/attestation 能力，不允许发布后替换 tag 或资产。
- versioned manifest 必须包含 schema、release version/classification、commit、发布时间以及每个 target 的 URL、size 与 SHA-256，并以 zterm 内置受信公钥可验证的 detached signature 认证。签名私钥不得进入 Git、artifact、日志或普通 CI job。
- `install/install.sh` 无参数只选择 latest non-prerelease stable；`--version <tag>` 只接受已发布的精确 stable 或 prerelease。它必须在下载 artifact 前拒绝未知 OS/arch、Alpine/musl、NixOS native 和低于 glibc 2.28 的系统。
- installer 使用有界 HTTPS 下载、临时目录、签名/长度/checksum/target/version/self-check、同文件系统 `fsync` + atomic rename；默认安装 `~/.local/bin/zterm`，不调用 sudo、不修改 shell rc、不执行 setup、不生成 identity/state、不启动 daemon 或注册服务。
- 实现显式 `zterm update [--version <tag>] [--force]`：在停止 daemon 前完成候选下载与验证；活动 Session 默认拒绝并准确显示影响；确认后停止 daemon/PTY，原子激活并 post-check，失败自动恢复旧 binary。成功后不自动启动 daemon。
- 实现 `zterm uninstall [--yes] [--force]`：先显示 Session、identity、authorization 与重新配对影响，再停止 daemon、删除完整受管理 `~/.zterm/`，最后删除 binary；可安全重试，不实现 `RevokeSelf` 或中央撤销。
- 安装来源、当前 build/protocol、install metadata 与 update compatibility 必须可诊断；手工替换形成的不兼容 CLI/daemon 组合必须明确拒绝而不静默杀进程或迁移。
- 发布与安装测试必须使用本地/隔离 HTTPS fixture 和 GitHub hosted matrix，不把真实签名私钥或生产 Release 作为普通单元测试依赖。

## Acceptance Criteria

- [ ] 四个受支持 target 的 release artifact、manifest、detached signature、checksums、SBOM 与 provenance 在同一 tag 上生成并交叉验证；stable 与 prerelease 选择规则有正负测试。
- [ ] 干净的 macOS arm64/x64 与 glibc Linux arm64/x64 普通账户可从测试 release endpoint 安装、运行 `zterm setup`、升级、注入激活失败后回滚、卸载和重装，全程无需管理员权限、Rust 或 Node/npm。
- [ ] 安装完成但 setup 前没有 `~/.zterm`、identity、配置、daemon、socket、PTY 或启动项；重复 setup 保留 EndpointId/授权且只有一个 daemon。
- [ ] manifest/signature/checksum/size/target/version/self-check 任一故障都发生在 daemon stop 前，当前 binary 与 Session 不变。
- [ ] 活动 Session 的 update/uninstall 默认拒绝；明确确认或 force 后才结束 PTY。更新失败恢复旧 binary；卸载后 state 与 binary 均移除，重装/setup 生成新 EndpointId。
- [ ] installer 在下载 artifact 前明确拒绝 unsupported target；PATH、手工审阅安装、immutable release、签名密钥轮换和紧急恢复边界有准确文档。
- [ ] 独立 Trellis checker 与完整 main CI（含四平台 release-mode build）通过后才允许人工创建精确 tag；tag workflow 必须在签名、安装矩阵、round-trip 与 attestation 全部成功后自动发布供 M10 使用的 immutable 正式候选 Release。

## Out of Scope

- 自动/后台更新、nightly channel、npm/Homebrew/mise/Nix、Windows installer/runtime、系统级 service、sudo 安装与中央撤销控制面。
- M10 的双物理网络、终端功能验收和最终用户验收；M9 只交付它们所依赖的正式发行入口。

## Key Decisions

- 正式 manifest 使用长期 Ed25519 release key。私钥只保存为 GitHub `release` Environment secret；引用该 environment 的发布 job 必须在 GitHub-hosted runner 上运行，并在读取 secret 前等待人工批准。普通 CI、pull request、self-hosted runner、artifact、日志和仓库均不得接触私钥。
- 发布同时启用 GitHub immutable releases 与 artifact/release provenance attestation。它们补充但不替代 zterm updater 对 detached manifest signature 的验证。
- CI 不创建 tag；人工只在精确 main commit 的 push CI 成功后创建并推送 `v*` tag。tag 自动触发发布，且只有读取签名 seed 的 job 使用一次 protected `release` Environment 审批。
- 用户接受 GitHub 托管密钥相对于完全离线签名的信任取舍，以换取可重复、低人工差错的正式发布流程。
