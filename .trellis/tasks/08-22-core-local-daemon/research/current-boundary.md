# M2–M3 当前边界核对

## 已完成基础

本子任务开始时，仓库已经不是空 workspace：

- `zterm-core` 已拥有 zterm-owned `TerminalModel`、snapshot/delta、受控 PTY query reply 和安全 side event。
- `zterm-platform` 已拥有 `portable-pty` 0.9.0 包装与 effective-account login shell 解析。
- `zterm-daemon` 已拥有单一 TerminalModel owner、持续 PTY drain/latest attachment，以及显式生产常量构造的 Iroh 1.0.3 官方 N0 profile。
- `zterm-proto` 只有 vendored-protoc build probe；`zterm-cli` 仍是无副作用 Foundation 状态输出。
- Foundation 的 macOS ARM/Intel、Ubuntu x64/ARM64、Windows、Relay/dependency CI 已全部通过。

因此 M2–M3 必须扩展现有边界，不得重写 PTY/VT，也不得把已经绿的 Foundation fixture 变成产品分支。

## 父任务已批准决定

- 一个 OS 用户只有一个 native zterm daemon；没有第二个 supervisor。
- 第一阶段不注册 systemd/launchd/cron/Login Item，不承诺机器重启后无人运行本地命令时可达。
- setup/需要 daemon 的未来命令按需 detached-spawn；普通检查命令不应因检查本身改变 daemon 状态。
- 本机 self target 只经 same-UID IPC 进入未来唯一的 `SessionService`，不 self-pair、不 self-dial Iroh、不访问 DNS/Pkarr/Relay。
- 持久根目录是 effective account home 下的 `.zterm`，不是环境变量 `$HOME`。
- identity 在 setup 生成；installer 不生成。卸载删除 key 后，正常重装是新的 EndpointId；没有中央撤销服务。
- active PTY/session/terminal bytes 不持久化。daemon 或主机重启可结束全部 PTY。
- 当前产品默认基础设施是 Iroh 官方 N0 production；自建 Relay 是可选显式 profile，不混入默认 map。

## 本子任务必须改动的现有占位

- `zterm_core::PHASE_NAME` 从 Foundation 更新为 core/local-daemon milestone。
- build-only `BOOTSTRAP_SCHEMA_VERSION`/BuildProbe 由真实 wire major/schema owner 取代。
- `TerminalModel` 的裸 `u64` revision 应改用 core 唯一的 `Revision` 类型，避免协议和 terminal 各自定义 revision。
- `zterm-platform::pty` 内部的 effective account lookup 应上提到共享 account 模块，由 PTY 与 user paths 共用。
- Foundation ALPN `zterm-gate/1` 在真实 v1 wire 落地后改为产品 `zterm/1`；不在本任务 bind endpoint。
- CLI 改成 clap 驱动的薄入口；daemon library 拥有 setup/state/server，CLI 不直接执行 SQL 或读取 secret。

## 明确不提前做

- 不创建 `SessionRegistry` 或 main PTY；M4 直接接入此任务冻结的 local service/target 边界。
- 不启动 Iroh endpoint，不实现 pairing/auth/revoke/connection broker。
- 不用临时 session map 伪造 `connect local`。
- 不用真实 `~/.zterm` 做测试，也不增加通用 `ZTERM_HOME` 产品后门。
- 不因为 Foundation Case A 延期而更换官方 profile、增加 QAD service 或修改本地 daemon 边界。
