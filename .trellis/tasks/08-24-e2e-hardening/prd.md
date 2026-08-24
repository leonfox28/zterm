# Complete Phase 1 end-to-end release acceptance

## Goal

完成第一阶段 M10：只使用 M9 产生的正式签名 GitHub Release 与官方 installer，在受支持平台、真实网络和安全故障矩阵上证明第一阶段的用户主张，并把最终统一验收交给用户。源码构建、Cargo 安装、branch head 和临时 CI artifact 不能替代最终安装证据。

## Background

- M1–M6 已有直接实现与证据；M7–M8 的本机/纯状态/PTY/hosted shared matrix 已通过。GitHub Actions run `32725142928` 已在 Linux x86_64 实际执行保留的 `two_daemon_transport` real-Iroh loopback 用例；它只证明 pair/normal transport owner，不证明 remote Session 或 public CLI OS 多进程。
- M9 尚未实现；M10 必须依赖它的正式候选 Release，而不是先用开发二进制 overclaim 安装验收。
- 生产默认保持 Iroh 官方 n0 profile。loopback/direct fixture 只证明 transport 行为，不能冒充 official-n0 Relay、公网发现、双 NAT 或双物理网络证据。
- 用户选择在第一阶段所有自动门禁完成后，使用正式 installer 做一次统一人工验收；中间不要求用户安装开发版。

## Requirements

- 当前 `remote-cli` 子任务只同步 run `32725142928` 的 hosted Windows/shared 与 Linux real-Iroh transport 证据后收口；不得再创建一套只在测试中存在的 daemon-like remote Session harness。public CLI/remote Session 的 OS 多进程证据由本任务使用 M9 正式安装产物完成，未执行的 ignored/compile-only target 不得计为通过。
- M9 发布候选必须由官方 installer 在干净账户安装；验证安装前无状态、setup 后唯一 identity/daemon、update rollback 与 uninstall/reinstall identity reset。
- 自动化网络矩阵必须分别证明 direct、relay-only、DNS/Pkarr 失败与缓存、relay 故障、connection/stream 丢失、双 NAT/网络切换以及 path observation；每项证据精确标明是否使用 QAD、official-n0 Relay、task-only direct route 或 loopback。
- 两个正式安装的 macOS/Linux 设备必须完成双向配对、方向授权、remote list/create/rename/attach/takeover/close、单 connection 多 stream、断线恢复、revoke、宿主 local continuation 和 Session/PTY 持久性。
- tmux 与固定 Herdr 使用通用终端路径完成 Unicode、颜色、光标、alternate screen、bracketed paste、连续 resize、detach/reconnect/snapshot 恢复；不增加程序名特判。
- 安全矩阵覆盖 malformed/fuzzed frame/ticket/prefix/ANSI、OSC 52/DCS/APC/未知图形序列、resource bounds、peer UID、symlink、secret/log redaction、未授权/撤销竞态与 direct/relay 密文抓包。
- macOS arm64/x64 与 glibc Linux arm64/x64 必须分别有 artifact build/install 证据；真实设备无法自动化的项目使用明确的人工记录，不用不同架构的 compile-only 代替。
- 第一阶段文档必须准确披露无 boot autostart、daemon restart/update 会结束 PTY、无 daemon crash persistence、无 transcript、official services metadata/no-SLA 与支持平台边界。

## Acceptance Criteria

- [ ] M7–M8 现有 Linux real-Iroh transport gate 有实际 run URL；public CLI/remote Session 的 OS 多进程主张由正式安装产物实际通过，不以新增 task-private harness 或 compile-only target代替。
- [ ] M9 正式候选 Release 的四平台 assets、签名和 installer gates 通过，最终测试不使用源码/Cargo/branch artifact 安装。
- [ ] 正式安装的两台受支持设备在两个真实网络上完成 direct 与 relay fallback、断开/重连、同 Session/PTY/cwd/screen continuation、单 connection 多 Session 和双向 takeover/revoke。
- [ ] clean-account install/setup/update/rollback/uninstall/reinstall、unsupported platform、SSH detach、tmux/Herdr 与安全矩阵有自动化或明确人工证据。
- [ ] PRD A–E 每个勾选项都链接到直接证据；M1–M10、父/子 task、spec、README/help 和 release notes 一致，无 overclaim。
- [ ] 全 workspace quality/dependency/source/version/secret/release gates 及独立 Trellis checker 通过，无未解决高危问题或无界资源增长。
- [ ] 用户最后从官方 HTTPS installer 安装 immutable、签名的候选 release，完成 setup、两设备配对、远程/本机接续与卸载/重装边界的统一验收后，第一阶段才能归档。

## Out of Scope

- Android、Windows runtime/installer、桌面 GUI、iOS、boot/login autostart、daemon crash/reboot 后 PTY 存活、自动更新、中央账号/撤销、完整 transcript、文件传输和 Agent 专用能力。
- 把可选 self-hosted Relay 重新提升为 M5–M8 的额外发布 gate；M10 只按父任务中已定义的真实网络/official-n0 与可选 profile 边界取证。

## Deferred Inputs

- 最终真实设备验收需要用户在检查点提供至少两台受支持设备/账户与两个独立网络；在此之前所有可自动化门禁先在隔离 fixture 与 hosted runners 完成。
