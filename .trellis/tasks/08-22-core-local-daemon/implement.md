# Core、本地 daemon 与 IPC 实施计划

## 0. Execution rules

- 本任务是父任务 M2–M3；严格停在 session engine 之前。
- 每一步只在 focused gate 通过后进入下一步。失败先修 owning boundary，不通过增加 retry、sleep 或重复 validator 掩盖。
- 所有会写文件/起进程的测试必须使用 task-private TempDir/UserPaths；任何门禁都不得触碰真实 `~/.zterm`。
- 不重跑 Foundation 的昂贵 A/B/C NAT lab；只保留已有 Iroh profile 回归。
- 实施开始前重新运行 `trellis-before-dev`，读本任务 context 和适用 spec。

## Step 0. Baseline、spec 与依赖

### Work

- [x] 记录 clean baseline：source policy、workspace version、fmt、Clippy、workspace tests、docs、cargo-deny。
- [x] 在 backend spec 新增并索引三个 owner：
  - core/wire domain contract；
  - effective-user state/config/identity/SQLite；
  - local daemon/IPC/lifecycle。
- [x] 根 workspace 添加最小依赖：
  - clap 4.6.6；
  - serde/serde_json/toml；
  - rusqlite 0.40.2，`default-features = false` + `bundled`；
  - tracing/tracing-subscriber（只用于 lifecycle/diagnostic log）；
  - tempfile 仅 dev。
- [x] 扩展已有 Tokio/nix features；不添加 fs4、uuid、chrono、dirs、gRPC 或 connection pool。
- [x] 更新 `PHASE_NAME` 只在真实 M2/M3 API 落地时完成，不先改占位文案制造虚假完成状态。

### Gate

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
python3 .trellis/scripts/task.py validate .trellis/tasks/08-22-core-local-daemon
```

Stop if a selected dependency needs unsupported Rust, introduces an unresolved license/advisory, or duplicates Rust 1.98/platform capability without a current consumer.

## Step 1. Shared core contract

### Work

- [x] Add zterm-owned fixed-byte IDs, Revision, principals, controller lease, capability bits and domain error categories.
- [x] Replace terminal model public raw revision values with the shared `Revision` type without changing terminal semantics.
- [x] Add Foundation-approved resource defaults and separate the 128 MiB admission projection from the 256 MiB RSS measurement target.
- [x] Implement fixed-epoch bounded `OperationWindow<R>` with exact-result replay and low-water `OutcomeUnknown`.
- [x] Test conversions/length errors, principal distinction, capability unknown-bit retention, resource values, out-of-order sequences, duplicate success/error replay, eviction and epoch mismatch.
- [x] Keep the state machine unbound in M3; actual replay integration starts with M4 stateful `SessionService` create/rename/close/takeover mutations.

### Gate

```sh
cargo fmt --all -- --check
cargo clippy -p zterm-core --all-targets --all-features -- -D warnings
cargo test -p zterm-core --all-features
cargo doc -p zterm-core --no-deps
```

Stop if core must depend on prost, Iroh, SQLite or OS APIs; move that conversion to its owning adapter.

## Step 2. Product protobuf and bounded framing

### Work

- [x] Replace the bootstrap-only schema with `proto/zterm/v1/{common,wire,local,pairing,session,terminal}.proto`; keep vendored `protoc`.
- [x] Generate all v1 files from one build script and expose only documented modules/validated projections.
- [x] Implement the incremental varint frame decoder and matching encoder with 8 MiB total and 1 MiB control-payload bounds.
- [x] Implement the single numeric kind registry, wire-major check and proto↔domain validators.
- [x] Define M3 local readiness/status/validate-setup/stop/update-preflight messages; define complete M4-facing list/create/rename/close/takeover and attach/snapshot/delta/input/resize/detach/snapshot-applied/sync shapes without dispatching later pairing/session/terminal services.
- [x] Replace Foundation build-probe version reporting with product wire major/schema identity; update product ALPN constant from `zterm-gate/1` to `zterm/1` without binding an endpoint.
- [x] Add round-trip/golden tests readable by future non-Rust generators; cover unknown optional fields, unknown kind, malformed/overflow varint, truncated body, payload limits and invalid ID lengths.

### Gate

```sh
cargo fmt --all -- --check
cargo clippy -p zterm-proto --all-targets --all-features -- -D warnings
cargo test -p zterm-proto --all-features
cargo test -p zterm-daemon --test iroh_profile_gate
cargo doc -p zterm-proto --no-deps
```

Stop if frame validation exists independently in CLI/daemon or if prost/Rust enum layout leaks into the public contract.

## Step 3. Effective-user platform boundary

### Work

- [x] Extract `EffectiveAccount` from the existing PTY-private account record; keep PTY login-shell behavior and corpus green.
- [x] Add `UserPaths` for state/config/key/database/log/locks/runtime socket from account home and validated platform runtime candidates.
- [x] Implement owner/mode/type/symlink validation and exact 0700/0600 creation.
- [x] Implement atomic sibling write with natural writer-error rollback; implement identity create-without-replace.
- [x] Add standard-library File lock guard using Rust 1.98 `File::try_lock`.
- [x] Add Unix runtime socket prepare/bind/chmod/stale-candidate validation.
- [x] Add generic peer UID retrieval: Linux `SO_PEERCRED`, macOS `getpeereid`, plus pure `authorize_peer`.
- [x] Add detached-spawn stdio/cwd builder and child-side safe `setsid` helper; no `pre_exec`.
- [x] Return typed unsupported errors on non-Unix without breaking Windows compile.

### Tests

- [x] Current account home/shell is shared with PTY.
- [x] Temp UserPaths, wider mode, wrong owner where testable, symlink file/dir and Unix socket path-length/fallback behavior.
- [x] Atomic write success and writer failure leave exact expected file.
- [x] Two handles/processes prove lock exclusivity and release-on-drop.
- [x] Real same-UID Unix stream returns expected UID on macOS/Linux; pure wrong-UID policy rejects.
- [x] Existing `pty_lifecycle` remains green.

### Gate

```sh
cargo fmt --all -- --check
cargo clippy -p zterm-platform --all-targets --all-features -- -D warnings
cargo test -p zterm-platform --all-features
cargo doc -p zterm-platform --no-deps
```

Stop if a mutation test resolves the developer's real account paths or if stale-socket removal can run without the daemon lifetime lock.

## Step 4. Config、identity、SQLite 与幂等 bootstrap

### Work

- [x] Add tagged ConfigV1 with official-n0 default and explicit self-hosted-only alternative; one semantic validator owns device name/profile.
- [x] Refactor `InfrastructureProfile` to consume the validated enum while keeping the explicit production-constant N0 regression invariant under staging env.
- [x] Add 32-byte SecretKey load/create and public EndpointId projection; never log private bytes.
- [x] Add SQLite v1 open/migration with 0600 precreation, NOFOLLOW, foreign keys, rollback journal, FULL sync and user_version.
- [x] Add metadata/device_auth/known_devices schema and typed store methods; no terminal/session/pair-offer/audit table.
- [x] Add `StoreActor` as the running daemon's single connection owner.
- [x] Implement bootstrap state matrix and lifecycle lock; invalid/missing key beside committed config/DB is a hard error, never auto-rotation.
- [x] If daemon is already live, validate setup through IPC instead of opening the live DB.

### Tests

- [x] First/repeated/concurrent setup produces one stable EndpointId.
- [x] Key-only, key+DB, key+config partial states resume; DB/config without key and key/metadata mismatch refuse.
- [x] Invalid TOML/schema/name/profile/self-host URL does not modify identity or DB.
- [x] Transaction closure failure rolls back generation/status changes.
- [x] Too-new user_version refuses; v0→v1 succeeds exactly once.
- [x] SQLite schema inventory proves excluded tables are absent.

### Gate

```sh
cargo test -p zterm-daemon --test persistence
cargo test -p zterm-daemon --test config_profiles
cargo test -p zterm-daemon --test setup_idempotency
cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings
cargo doc -p zterm-daemon --no-deps
```

Stop on any setup path that overwrites an existing key or silently recreates an incompatible DB.

## Step 5. Local IPC server/client and daemon lifecycle

### Work

- [x] Add bounded Tokio Unix listener and one-request-per-connection client/server adapters over zterm-proto framing.
- [x] Check peer UID before frame decode; cap active connections/deadline and map cancellation/EOF to typed errors.
- [x] Add DaemonService readiness/status/validate-setup/stop/update-preflight; unimplemented kinds return `ServiceNotImplemented`.
- [x] Add `daemon.lock` lifetime guard, lock-owner-only stale socket cleanup and graceful own-socket removal.
- [x] Implement `ensure_daemon` double-probe/singleflight lifecycle lock and bounded readiness wait.
- [x] Hidden internal child calls `setsid`, resolves production paths itself, opens store before readiness, and never acquires lifecycle lock.
- [x] Implement bounded lifecycle log output and simple pre-spawn rotation without high-volume transport machinery.
- [x] Implement graceful stop response-before-shutdown; no PID file or kill-by-guessed-PID fallback.
- [x] Keep lifecycle stop naturally idempotent and outside `OperationWindow`; do not claim committed-response replay before M4 stateful session mutations exist.

### Tests

- [x] local IPC same-UID readiness/status/stop.
- [x] wrong major/kind/oversize/deadline/client-close affect one request only.
- [ ] Linux CI cross-UID harness reaches the socket and is rejected by `SO_PEERCRED`.
- [x] N concurrent launchers yield one daemon; all receive readiness.
- [x] live socket is never removed; stale owned socket is cleaned only by new daemon lock owner; symlink/non-socket refuses.
- [x] launcher exit/closed terminal does not end child; explicit stop does and removes its socket.
- [x] restart preserves identity and returns to readiness.
- [x] after an ungraceful daemon/process loss, no daemon starts by itself; a fresh `ensure_daemon` call starts one instance with the existing identity and no phantom session state.

### Gate

```sh
cargo test -p zterm-daemon --test local_ipc
cargo test -p zterm-daemon --test single_instance
cargo test -p zterm-daemon --test detached_lifecycle
sh tests/core-local-daemon/cross-uid.sh
```

The cross-UID script may skip locally without noninteractive privilege, but the existing Ubuntu x64 CI job must run and pass it. Stop if same-UID correctness depends on external network or if daemon startup requires Iroh/DNS/Relay.

## Step 6. Thin CLI and diagnostic UX

### Work

- [x] Split `zterm-cli` into a testable library + thin main and add clap command tree.
- [x] Implement `--help`/`--version`, interactive/noninteractive setup, status/JSON, doctor/JSON, daemon status/stop/restart, bounded logs tail.
- [x] Keep hidden internal daemon mode out of help; it accepts no arbitrary state path from normal product argv.
- [x] Encode side-effect table:
  - setup and restart may ensure/spawn;
  - status/daemon status/doctor/logs/stop never spawn.
- [x] Stop/restart render exact active-session impact and require force only when nonzero; M3 returns zero through service schema.
- [x] No-argument CLI prints accurate milestone help until M4 implements bare local connect.
- [x] Ensure CLI invokes daemon-owned bootstrap/status/doctor APIs and never imports rusqlite or parses identity bytes.

### Tests

- [x] `--help` contains only public commands; `--version` has no filesystem/process side effect.
- [x] setup in TempDir starts exactly one daemon and outputs same identity on repeat.
- [x] status human/JSON share fields; stopped status does not create socket/process/state.
- [x] doctor reports unsafe/missing paths and logind limitation without starting daemon.
- [x] logs reads bounded tail/no secret; stopped stop succeeds; restart starts one daemon.
- [x] noninteractive first setup without required values fails before creating identity.

### Gate

```sh
cargo test -p zterm-cli --test setup_permissions
cargo test -p zterm-cli --test daemon_autospawn
cargo test -p zterm-cli --test command_side_effects
cargo clippy -p zterm-cli --all-targets --all-features -- -D warnings
cargo doc -p zterm-cli --no-deps
```

Stop if a display/inspection command causes daemon spawn or if CLI gains direct SQLite/secret ownership.

## Step 7. Integration、docs and final gate

### Work

- [x] Add `docs/core-local-daemon.md` and update README/architecture references with exact supported commands and known limits.
- [x] Document that session/connect/local terminal attach starts in M4; do not claim a usable remote terminal yet.
- [x] Document official N0 default, optional self-host config shape, no network requirement for local readiness, no autostart and daemon restart ending future PTYs.
- [x] Update backend specs from implementation evidence and verify no source/template mirror exists.
- [x] Update `PHASE_NAME` and remove bootstrap-placeholder wording only after all behavior is real.
- [x] Run secret scan and check logs/errors/fixtures contain no identity private bytes or real user state.
- [ ] Run hosted CI; inspect every matrix job, not only aggregate status.

### Full gate

```sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
sh tests/relay/static.sh
sh tests/secret-scan.sh
python3 .trellis/scripts/task.py validate .trellis/tasks/08-22-core-local-daemon
git diff --check
```

Hosted CI must pass macOS latest ARM, macOS Intel, Ubuntu x64, Ubuntu ARM64, Windows and dependency/Relay jobs. Windows is compile/shared-contract evidence only for M2–M3 Unix lifecycle.

## Completion checklist

- [x] Every PRD acceptance criterion has one named test/evidence owner.
- [x] No temporary retry, duplicate parser, PID fallback, second daemon or fake session registry remains.
- [x] No product/test mutation touched real `~/.zterm`.
- [x] No open blocker is hidden as “deferred”; only the already approved Foundation real-network evidence remains deferred to M10.
- [x] Child task remains active until independent check and hosted CI are green.
