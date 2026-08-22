# M2–M3 依赖与平台原语

核对日期：2026-08-22。工具链固定为 Rust 1.98.0。

## 结论

### 文件锁：使用标准库，不添加 fs4

Rust 从 1.89.0 起稳定提供：

- `std::fs::File::lock()`
- `std::fs::File::try_lock()`
- `std::fs::File::unlock()`

项目最低 Rust 已固定为 1.98，因此 lifecycle/spawn 和 daemon lifetime lock 直接使用标准库。锁由打开的 File handle 持有，guard 负责保持 handle 生命周期；不重复包装一套 lockfile 协议，也不引入 `fs4`。

### SQLite：rusqlite 0.40.2 + bundled

选择：

```toml
rusqlite = { version = "=0.40.2", default-features = false, features = ["bundled"] }
```

原因：

- bundled 避免用户系统 SQLite 版本差异和开发/安装前置依赖。
- 关闭 default feature，避免当前默认的 wasm FFI 路径和无关 statement cache。
- `OpenFlags::SQLITE_OPEN_NOFOLLOW` 可拒绝 database symlink。
- 单一 store owner 不需要多 connection/WAL；使用默认 rollback journal、foreign keys、事务 migration 与 `synchronous=FULL` 即可。这样也不增加 WAL checkpoint/sidecar 生命周期。
- schema version 只使用 SQLite `PRAGMA user_version`，不再在 metadata 复制第二个版本字段。

### CLI/config/JSON

- `clap = 4.6.6`，只启用 std/help/usage/error-context/suggestions/derive 所需能力。
- `serde = 1.0.229` + derive。
- `toml = 1.1.4+spec-1.1.0` 负责标准 TOML 语法；zterm 只验证语义 profile，不重写 TOML parser。
- `serde_json = 1.0.151` 只用于稳定的 `status --json` 与结构化诊断投影。

### 异步 local IPC

沿用 lockfile 中的 Tokio 1.53.1，按 owner crate 开启最小的 `rt-multi-thread`、`macros`、`net`、`io-util`、`sync`、`time`。Tokio 只负责 Unix socket accept/read/write、deadline 和 cancellation；SQLite connection 仍由单一 store actor 独占。

不引入 gRPC、tokio-util codec 或第二套 framing。zterm-proto 拥有纯 framing state/validator；daemon 只提供 AsyncRead/AsyncWrite adapter。

### Unix 账户、peer UID 与 detach

继续固定 `nix = 0.28.0`，增加已有 crate 的 features：

- `user`：`geteuid`、`User::from_uid`
- `socket`：Linux `getsockopt(PeerCredentials)` 与 macOS/BSD `getpeereid`
- `process`：safe `setsid()`
- `fs`：现有 effective-access/path 能力

标准库 `UnixStream::peer_cred` 在 Rust 1.98 仍不稳定，因此 peer credential 继续由 nix 单点封装。产品代码不调用 libc，不新增 unsafe。

### Iroh identity

固定 Iroh 1.0.3 的公开 API：

- `SecretKey::generate()`
- `SecretKey::to_bytes() -> [u8; 32]`
- `SecretKey::from_bytes(&[u8; 32])`
- `SecretKey::public()`

`identity.key` 保存精确 32 bytes；不再加自定义 token、证书 envelope 或 checksum。文件长度、mode、no-follow 和派生 public key/DB metadata 一致性是当前消费方真正需要的检查。

## 路径与文件策略

- effective UID 的账户 home 是持久目录 source of truth。
- `OpenOptionsExt::mode(0o600)` + create-new 用于 temp/final 文件；托管节点以 `symlink_metadata` 拒绝 symlink。
- atomic replace：同目录 unique create-new sibling → write → `sync_all` → rename → parent directory `sync_all`。
- SQLite 文件先以 0600 create-new，再用 READ_WRITE + NOFOLLOW 打开；existing 文件在 open 前复核 type/owner/mode。
- Linux runtime dir：验证 `XDG_RUNTIME_DIR` 后创建 `zterm`；macOS：验证 `TMPDIR` 后创建 `zterm-<uid>`；两者不可用时回退 `/tmp/zterm-<uid>`。
- daemon socket bind 后立即 chmod 0600；其父目录 0700，因此 bind/chmod 之间也不暴露给其他 UID。

## 明确未选

- `fs4`：标准库已覆盖。
- `uuid`：领域 ID 是固定字节 newtype；本任务不生成 SessionId。
- `chrono`：持久时间使用 Unix epoch integer；不需要时区/格式化依赖。
- `dirs`/`home`：不允许用环境变量 home 替代 effective account database。
- WAL/connection pool：单 store owner 没有并发读需求。
- gRPC/tonic：local IPC 和未来 QUIC 都复用轻量 protobuf frame。
- 自定义 SemVer/TOML/SQLite/protobuf syntax validator：标准 parser 已拥有这些边界。
