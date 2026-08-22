# Core、本地 daemon 与 IPC

## Goal

完成父任务第一阶段的 M2–M3：冻结跨层领域类型和 wire 边界，在 macOS 与主流 glibc Linux 上交付普通用户自己的唯一 zterm daemon、幂等 setup、安全持久状态、same-UID local IPC 与基础 CLI 生命周期。完成后，下一子任务可以直接在同一 daemon 和 IPC 上实现唯一的 `SessionService`，而不需要重写身份、路径、协议或进程模型。

## Background

Phase 1 Foundation Gate 已证明并保留：

- Iroh 1.0.3 官方 N0 生产 profile 可作为当前默认基础设施；受控 Case B 能升级到 Direct，阻断非 DNS UDP 的 Case C 能继续通过 WSS/TCP Relay。嵌套 Colima/Patchbay/TUN 中 Case A 的自动地址发现证据仍延期到两条真实网络验收，不阻塞本任务。
- `portable-pty` 0.9.0、zterm-owned `TerminalModel`、持续 PTY drain、latest-only attachment 和资源候选已经通过 Foundation。它们在本任务中保持可编译、可测试，但不提前实现 session registry。
- 1.0 采用每 OS 用户一个非特权 daemon；不注册开机/登录启动项，不增加 supervisor。运行本地命令时按需 detached-spawn。
- 本机目标最终通过 same-UID IPC 调用与远端 adapter 相同的 `SessionService`，不 self-dial Iroh、不查询 DNS/Pkarr、不经过 Relay。

本子任务是父任务已经批准的 M2–M3 的可执行拆分，不改变阶段顺序或产品边界。

## Requirements

### R1. 共享领域与资源契约

- `zterm-core` 是 `DeviceId`、`SessionId`、`AttachmentId`、`AttachmentPrincipal`、`Revision`、`ControllerLease`、能力位、资源限制和领域错误的唯一 owner。
- `AttachmentPrincipal` 明确区分 `REMOTE_ENDPOINT` 与 `LOCAL_SAME_UID`；本地来源由 peer UID 建立，不能伪装成已配对远端设备。
- Foundation 实测后的默认资源候选进入共享契约：每用户最多 8 个 live session、每 session 2,000 行近期历史、无控制端时 120×40、最大 240×80、所有 terminal model 的 fixed-cell projection 合计不超过 128 MiB；256 MiB 只作为未来完整 daemon RSS 目标，不伪装成结构预算。
- 状态变更操作使用 128-bit `OperationId { client_epoch, sequence }`。固定 epoch 的有界结果窗口对 retained ID 重放原结果；已越过低水位的 ID 返回 `operation_outcome_unknown`，不得再次执行。

### R2. Versioned protobuf 与 framing

- `proto/zterm/v1/*.proto` 是 wire source of truth；Rust 继续使用 vendored `protoc` + prost，未来 Android/iOS 可使用标准 generator。
- 定义 versioned envelope，以及 common/local/control/pairing/session/terminal 的消息形状。M3 只执行 local lifecycle RPC；未实现的 pairing/session/terminal service 不挂假 handler。
- 每个 frame 使用 `varint length + WireFrame`。总 frame 上限 8 MiB；control payload 上限 1 MiB；外层长度在分配 frame buffer 前校验，inner payload 长度在解码具体消息前按 kind 校验。
- protobuf unknown field 按兼容规则忽略；unknown message kind、wire major 不兼容、畸形 varint/protobuf、过大 frame/payload 返回 typed protocol error。
- 每个跨层对象只有一个 decoder/validator；CLI 和 daemon 不各自解释 protobuf bytes。

### R3. 用户路径、配置与设备身份

- `zterm-platform` 从 effective UID 的账户数据库取得 home、login shell 与 UID；不得把 daemon 启动环境的 `$HOME`/`$SHELL` 当作权威来源。
- 第一阶段持久根目录固定为该账户 home 下的 `~/.zterm/`，至少保留 `config.toml`、`identity.key`、`state.sqlite3`、`install.json`、`logs/` 和 lifecycle lock 路径。
- 根目录和 runtime 目录为 `0700`；敏感/普通持久文件、lock 和 Unix socket 不宽于 `0600`。拒绝托管路径上的 symlink；最终敏感文件使用 no-follow/open-new 边界。
- 原子文件写入使用同目录 create-new sibling、`0600`、写入、file sync、rename、directory sync。失败只能留下完整旧文件或完整新文件。
- `identity.key` 是 Iroh `SecretKey` 的精确 32 个原始字节，只在不存在时生成。重复 setup、配置错误或部分 setup 恢复不得轮换身份；数据库 metadata 的公开 EndpointId 必须与该 key 派生结果一致。
- `config.toml` 有显式 schema version。默认 profile 是官方 N0 production；可表示一个显式 self-hosted-only Relay profile，但不得混合两套 map、读取 staging profile 或接受空 Relay URL。配置无效不改变 identity。

### R4. SQLite 持久状态

- 使用 `rusqlite` bundled SQLite，由 daemon 的单一 store owner 访问；启用 foreign keys、事务迁移和 `synchronous=FULL`，schema 版本由 SQLite `user_version` 唯一拥有。
- 最小 schema 保存 identity metadata、`device_auth` 的 generation/status/revoked tombstone，以及 `known_devices` 的 alias/名称/版本化 route cache。
- migration 和授权状态变更在单事务中提交；未来 schema、过新 schema 或 metadata/key 不一致返回可操作错误，不静默降级或重建。
- terminal bytes、screen、scrollback、PTY、live session、operation replay window、pair offer 和完整 ticket 不入库；不建立持久 audit-event 表。

### R5. 每用户 daemon 与 same-UID local IPC

- 只发行一个 `zterm` 可执行文件；同一文件的隐藏 internal daemon 入口运行 daemon，不增加第二个二进制、supervisor 或脚本守护进程。
- daemon 持有稳定 `daemon.lock`；CLI/setup 使用独立的短期 lifecycle/spawn lock。并发 `ensure_daemon()` 最终只能产生一个 daemon，且 daemon 不反向等待 spawn lock。
- Linux 优先使用经过 ownership/mode 校验的 `$XDG_RUNTIME_DIR/zterm/daemon.sock`；macOS 优先使用经过校验的 `$TMPDIR/zterm-<uid>/daemon.sock`；失败时使用短、owned、`0700` 的 `/tmp/zterm-<uid>` 回退目录。
- daemon 只有在取得 `daemon.lock`、确认现有 socket 无法连接、并验证目标是当前 UID 拥有的真实 socket 后，才删除 stale socket。launcher 不抢先删除。
- Linux 使用 `SO_PEERCRED`，macOS 使用 `getpeereid`；peer UID 不等于 effective UID 时，在解码/dispatch 请求前拒绝。
- local IPC 复用共享 frame codec。首版 unary request 每个 Unix socket connection 一个请求；包含 request ID、相对 deadline、structured result/error，并为未来有状态 mutation 保留 OperationId。M3 的 lifecycle stop 只有在 response flush 后才触发退出，重复 stop 在 CLI 边界幂等且不声明 replay；`OperationWindow` 从 M4 的 create/rename/close/takeover 等 `SessionService` mutation 开始接线。
- `local` 是保留 target selector。本任务只冻结它通向未来同一个 `SessionService` 的协议边界，不创建第二套 registry，也不通过 Iroh 自连。
- daemon 内部入口启动时先用 safe `setsid()` 脱离控制终端；stdin 连接 `/dev/null`，stdout/stderr 追加到用户日志，cwd 稳定。不得使用违反 workspace `unsafe_code = forbid` 的 `pre_exec` 实现。

### R6. Setup、基础 CLI 与副作用边界

- `zterm setup` 校验路径/权限，确认设备名和 profile，通过 daemon-owned bootstrap 模块幂等创建或恢复 identity/config/database，然后调用统一 `ensure_daemon()` 并等待 local readiness。
- 本任务实现：`zterm setup`、`zterm status [--json]`、`zterm doctor`、`zterm daemon status|stop|restart`、`zterm logs`，以及隐藏 internal daemon 入口。
- `setup` 与 `daemon restart` 可以启动 daemon；`status`、`daemon status`、`doctor`、`logs` 和 `daemon stop` 不自动启动 daemon。未来 `connect`/session 命令统一调用同一个 `ensure_daemon()`。
- stop/restart response 预留活动 session 数和名称；存在 session 时交互确认，非交互要求 `--force`。M3 尚无 session engine，正常返回零，不伪造 session。
- `doctor` 在 daemon 离线时仍能检查账户 home、目录/文件 mode、identity/config/database 一致性、login shell、socket/lock 和已知 logind 生命周期限制；M3 不把 DNS/Relay 可达性设为本地 readiness 前置条件。
- CLI 不直接解析 secret key 或执行 SQL；这些能力由 daemon-owned library/server 提供。测试使用注入的临时 `UserPaths`，不得创建、读取或删除开发者真实 `~/.zterm`。

### R7. 简洁与平台边界

- 第一阶段 M2–M3 支持 macOS x86_64/arm64 与主流 glibc Linux x86_64/arm64；Windows 只要求共享 core/proto 继续编译，Unix 生命周期 API 返回明确 unsupported，不提前实现第三阶段能力。
- 不访问外网、不 bind Iroh endpoint、不做 pairing/revoke/update/install/uninstall，不注册 systemd/launchd/cron/Login Item。
- 每个 invariant 只有一个 owner 和一组能跨过真实边界的测试；不为标准 SemVer/TOML/protobuf/SQLite parser 再写平行语法校验器，不用重复静态 grep 代替已存在的 runtime test。

## Acceptance Criteria

- [x] core ID/principal/resource/error 和 fixed-epoch operation-window 状态机测试通过；同一 OperationId 只执行一次、窗口内返回原结果、低水位之前返回 outcome unknown。
- [x] proto round-trip、unknown-field compatibility、unknown-kind rejection、wire-major mismatch、truncated/overlong varint、8 MiB frame 和 1 MiB control payload 边界测试通过；超限在 inner message decode 前被拒绝。
- [x] 在隔离临时账户路径中，首次 setup 创建 `0700` 根目录、`0600` identity/config/database/locks；重复和并发 setup 保留同一个 EndpointId，部分失败后重试也不轮换 key。
- [x] symlink、wrong owner/mode、identity 长度错误、metadata/key 不一致、过新 schema 和非法/mixed infrastructure profile 被明确拒绝，且不改写原状态。
- [x] SQLite migration/authorization transaction 的成功与注入失败测试证明不存在半写状态；数据库不包含 terminal/session/pair-offer/audit-event 数据。
- [x] 多进程并发 `ensure_daemon()` 只产生一个持有 `daemon.lock` 的实例；stale socket 仅由 lock owner 安全清理，live socket 不被删除。
- [ ] same-UID 真实 Unix socket 请求成功；Linux CI 以独立 UID 实际连接测试证明 peer credential gate 拒绝非 owner，macOS 覆盖真实 `getpeereid` same-UID 路径与纯决策负例。
- [x] detached daemon 在启动它的 CLI/终端退出后继续响应；stop 有界退出并清理自己的 socket，restart 复用原 identity。没有 OS 启动项或 supervisor。
- [x] setup 后 status、JSON status、doctor、logs、daemon status/stop/restart 行为与副作用表一致；检查类命令在 daemon 停止时不悄悄拉起。
- [x] local readiness/status/stop 在外网、DNS/Pkarr 和 Relay 完全不可用时仍通过；产品代码没有 self-dial 本机的 Iroh 路径。
- [ ] source policy、workspace version、fmt、Clippy `-D warnings`、workspace tests、docs、cargo-deny、macOS/Linux/Windows CI 与任务校验全部通过；文档准确写明 M3 尚未提供 session/connect。

## Out of Scope

- M4 的 `SessionRegistry`/`SessionActor`、main session、local attach、input/resize/controller lease、snapshot/delta wire execution与真实 CLI terminal rendering。
- Iroh endpoint bind、connection broker、pairing、normal auth、device revoke、NAT/Relay 路径测试；Foundation profile 代码只需保持回归通过。
- installer、updater、uninstaller、identity reset、manifest 签名或 artifact 替换。
- 开机/登录自启、systemd user service/linger、launchd、cron、Login Item、crash supervisor。
- Windows daemon/IPC/CLI、Android、iOS、桌面 GUI、多 observer 和专有 Agent 状态/通知。
- terminal/session 运行态持久化，以及 daemon/宿主重启后的进程恢复。

## Risks and Mitigations

| Risk | Mitigation |
| --- | --- |
| Unix socket 路径、peer credential API 在 Linux/macOS 不同 | `zterm-platform` 单点封装；两平台真实 same-UID 测试，Linux 再做一次实际 cross-UID 拒绝 |
| partial setup 意外生成第二个身份 | setup lifecycle lock + identity create-new；数据库只接受与现有 key 派生公钥一致的 metadata |
| launcher 与 daemon lock 顺序形成死锁 | lifecycle/spawn lock 只由 launcher 短持有；daemon 只持 lifetime `daemon.lock`，启动后 readiness 由 socket 证明 |
| framing 为未来移动端冻结错误 Rust 细节 | source of truth 保持标准 proto 与数值 kind；zterm-owned decoder 不暴露 prost/Rust enum layout |
| 为尚未实现的 session 路径造临时状态机 | 只冻结 selector/message/capability；M4 才挂唯一 `SessionService` |
| 测试污染真实用户状态 | 所有 mutation 测试注入 task-private temp paths；产品没有通用 `ZTERM_HOME` 环境后门 |

## Deferred Evidence

- Foundation Case A 的自动地址发现成功率仍在父任务 M10 使用两条真实网络验证；与本地 daemon readiness 无关。
- macOS 实际 cross-UID 连接可在后续专用权限 lab 补验；本任务由 `getpeereid` 实际 same-UID、共享拒绝状态机及 Linux 实际 cross-UID 覆盖相同策略。
- 完整 daemon RSS、session 数/scrollback 资源执行由 M4 registry 实现后重新测量；本任务只冻结 Foundation 已批准的共享默认值。
