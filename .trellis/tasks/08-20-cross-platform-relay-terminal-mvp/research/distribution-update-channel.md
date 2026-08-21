# npm 与独立安装/更新通道

## Herdr 可审计事实

本次复核使用 Herdr 官方仓库 `herdrdev/herdr` 的完整公开 Git 历史、当前安装文档、更新实现和 npm registry 元数据，检查日期为 2026-08-21。

- Herdr 官方仓库首个公开提交 `a57b97285c3308fbdcc167ab3da9ae1dd78767b6`（2026-03-23）的 README 已经把 `curl -fsSL https://herdr.dev/install.sh | sh` 作为安装方式，并已经包含 `herdr update`。公开历史中没有先以 npm 平台包装包分发 Herdr 主二进制、再迁移到脚本的提交证据：[首个公开提交 README](https://github.com/herdrdev/herdr/blob/a57b97285c3308fbdcc167ab3da9ae1dd78767b6/README.md)。
- npm registry 的 `herdr` 只有 `0.0.0` 一个版本，创建于 2026-05-09；描述和 README 都明确说明它只是为 Herdr 保留包名，不包含主程序发行：[npm `herdr`](https://www.npmjs.com/package/herdr)。这发生在官方仓库首次公开 curl 安装之后。
- 因而，不能根据现有公开证据声称 Herdr 曾经正式从 npm 迁移。早期未公开试验或用户记忆中的其他包无法由当前证据排除，但不能作为 zterm 设计依据。

## “更新更方便”的准确含义

`curl | sh` 只是 bootstrap 表象。真正让 Herdr 更容易控制升级的是：官方 installer 把原生二进制安装到 Herdr 自己管理的用户路径，后续 `herdr update` 可以读取官方 manifest、选择平台产物、校验 SHA-256，并在知道当前 client/server 协议与 session 状态的前提下替换程序。当前文档只允许这种 direct install 使用内置 updater；Homebrew、mise 和 Nix 安装必须继续由各自包管理器更新：[安装与更新文档](https://github.com/herdrdev/herdr/blob/master/docs/next/website/src/content/docs/install.mdx)、[更新实现](https://github.com/herdrdev/herdr/blob/master/src/update.rs)。

Herdr 也没有把“可控更新”解释为“自动更新”。`0.4.11` 因持久 server/client 架构和混合版本风险，取消静默后台安装，只保留检查与提示，并要求用户在 shell 中手动运行 `herdr update`：[CHANGELOG](https://github.com/herdrdev/herdr/blob/master/CHANGELOG.md#0411---2026-04-16)。后续实现进一步做到先下载/准备更新，再决定是否停止不兼容的运行中 server。

## 对 zterm 的取舍

### 保留 npm 主渠道

- 优点：registry 版本、完整性、缓存、代理、安装和卸载由成熟包管理器承担；zterm 不必实现自更新器。
- 代价：宿主必须已有 Node/npm；全局 prefix、nvm/Node 版本切换和权限会影响原生 CLI 的稳定路径；原生四平台需要 JS shim 加 optional platform packages；升级仍是 `zterm daemon stop` 后再运行 npm，两步才能安全处理中断；当前还需要选择未冲突的 npm 包名。

### 官方 direct installer + 手动内置更新

- 优点：Rust 原生程序不依赖 Node；可以稳定安装在当前用户路径；同一个 `zterm update` 能先下载并校验，再显示活动 session、确认中断、停止 daemon、原子替换，仍完全由用户手动触发；不再受 npm scope/包名冲突影响。Windows 阶段可以采用版本目录加 `current` 指针，避免覆盖正在运行的 exe。
- 代价：项目必须自己维护 shell/PowerShell installer、manifest、平台选择、checksum/签名、原子替换、失败回滚、安装来源识别和卸载文档；`curl | sh` 本身扩大官网与发布链路的供应链责任。

## 已确认决定

用户于 2026-08-21 确认第一阶段采用官方 direct installer，并提供手动 `zterm update`；不把 npm 或其他包管理器作为第一阶段官方发行渠道。更新不在后台检查、下载或安装，只有用户显式运行命令才触发；允许在列出影响并确认后中断 session。

该决定使 npm scope 与包名冲突不再阻塞第一阶段，同时把 installer、manifest、校验、签名、原子替换、失败回滚和 bootstrap 信任披露提升为发布验收的一部分。

## 2026-08-21 补充：GitHub 托管与精确开发版

用户进一步确认 installer 与 release 可以直接托管在 GitHub：无参数安装最新稳定版，也可以显式指定一个稳定版或开发版。

- GitHub 官方提供稳定的 latest release 与 latest release asset 链接；REST `latest` 的定义是最近的 non-prerelease、non-draft release，所以默认安装不会误选开发 prerelease：[Linking to releases](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)、[REST releases](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)。
- GitHub Release 是 tag 对应的可部署包并可附带二进制 assets，适合作为稳定版与显式开发 prerelease 的长期下载入口：[About releases](https://docs.github.com/en/repositories/releasing-projects-on-github/about-releases)。
- GitHub Actions artifact 不是合适的用户安装源：公共仓库的 artifact/log retention 上限是 90 天，会使一个曾经可指定的开发版本自动消失：[artifact retention](https://docs.github.com/en/organizations/managing-organization-settings/configuring-the-retention-period-for-github-actions-artifacts-and-logs-in-your-organization)。
- 本地 Zedra installer 已验证这一基本 UX 可行：脚本通过 raw GitHub 托管，无参数跟随 `/releases/latest`，`--version` 直取指定 release tag（`/Users/huyuanzhe/projects/zedra/scripts/install.sh:1-4,84-106,169-190`）。zterm 不照搬其 best-effort checksum；稳定版与开发 prerelease 都必须强制验 manifest 签名、target、长度与 checksum。

因此 zterm 采用“已发布开发版”而不是“任意源码快照”：开发构建使用带日期与 short SHA 的 SemVer prerelease tag，作为 GitHub prerelease 发布完整签名 assets；无参数 install/update 始终选择 stable，开发版只通过 `--version <release-tag>` 显式选择，不建立自动 nightly 行为。
