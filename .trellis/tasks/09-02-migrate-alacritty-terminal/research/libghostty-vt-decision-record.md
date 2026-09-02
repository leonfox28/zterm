# libghostty-vt Implementation Decision Record

## Purpose

这是实现与检查阶段自动注入的精简决策记录。完整 API/ABI、平台、许可证、构建和
soundness 证据保留在 `libghostty-vt-integration.md`；本文件只记录已经收敛、不得在
实现中自行改变的边界。

## Selected Architecture

```text
Zterm-owned core/proto DTO
        -> host-only zterm-terminal
            ├── safe adapter -> community safe libghostty-vt crate
            │                    -> libghostty-vt-sys raw FFI
            │                    -> exact official C source, statically linked
            └── canonical projection -> Zterm allowlisted ANSI encoder
```

- Rust 包装 C 是正确边界，但 Zterm 不手写 `extern`、bindings 或 raw FFI。
- 只依赖 safe `libghostty-vt`；Zterm product crates 保持 `unsafe_code = "forbid"`，
  不直接依赖 `libghostty-vt-sys`，也不增加 `unsafe impl Send/Sync`。
- 新建 host-only `zterm-terminal` crate。`zterm-core`/`zterm-proto` 只保留 Zterm-owned
  state、modes、snapshot/delta/history DTO，因此 iOS/Android 远程客户端无需链接
  Ghostty、Zig 或 C。
- `portable-pty` 继续负责 child process、PTY byte stream、resize、wait/close；本任务
  不替换 PTY layer。
- Ghostty 是 parser/state authority，不是 Zterm wire authority。Ghostty 通用 formatter
  不能直接生成 wire；Zterm-owned encoder 只发当前 DTO 可表达的 allowlisted ANSI。

## Probe Candidate and Qualification Gate

- Wrapper repository: `https://github.com/Uzaaft/libghostty-rs.git`
- Wrapper probe revision: `5988a0b78b4aa804d1c12e66bbfe662bd97d81c0`
- Embedded Ghostty revision: `22d13172cde98a0a4dda05d3d6a3fcb0dd8ed018`
- Zig: `0.16.0`
- Cargo: full `rev`, `default-features = false`, static link
- Release native mode: `LIBGHOSTTY_VT_SYS_OPTIMIZE=ReleaseSafe`
- Release CPU: `LIBGHOSTTY_VT_SYS_CPU=baseline`

Do not use crates.io `0.2.1`: it is older source under the same nominal version, pins a different
Ghostty/Zig tuple, and predates reachable callback/string soundness fixes in the probe revision。

`5988a0b...` 不是 final cutover pin。截至 2026-09-02，它仍包含 open issue #70：safe
`RenderStateRowCells::graphemes_buf` 不传 slice capacity。Gate A 必须先选择包含上游修复的
完整 successor revision，再重新固定/审计 wrapper、Ghostty、Zig tuple；没有合格 revision
则 no-go，不在 Zterm 内写 workaround、sys 调用或 unsafe。

## Source and Build Policy

- Add exactly one `deny.toml` Git allowlist entry for the wrapper repository; `Cargo.lock` and a
  repository source-policy check verify the full wrapper revision and embedded Ghostty revision.
- Normal Cargo compilation must not fetch the network. A controlled preparation step performs
  locked fetch and creates an archivable vendor/source bundle containing the Git wrapper、Ghostty
  source 与 Zig system inputs，再导出 `GHOSTTY_SOURCE_DIR`/
  `GHOSTTY_ZIG_SYSTEM_DIR`；offline/network-disabled rebuild 是 required evidence。预热 Cargo
  Git cache 不能代替 source bundle。
- Pin and checksum Zig archives per hosted OS/architecture. Cache keys include toolchain, lockfile,
  target and build inputs; cache content is never the version authority.
- `default-features = false` disables the wrapper's default Kitty graphics Rust surface, but the
  inspected `build.rs` does not thereby prove a minimal Ghostty native `vt-features` build. Inspect
  the actual archive/SBOM/resource surface. If it violates a gate, add an audited wrapper build
  control upstream or re-review a narrow fork before cutover.
- Record wrapper revision, actual Ghostty source revision, Zig version/checksum, native sources,
  licenses and final static dependencies in release provenance/SBOM.

## Thread Ownership and Driver

All safe wrapper handles are intentionally `!Send + !Sync`.

- Create, use, callback into and drop every Ghostty handle on one dedicated model-owner thread.
- Replace the current cross-thread `Mutex<TerminalModel>` access with bounded actor commands for
  ingest, input, resize, snapshot, sync, history and shutdown.
- Commands/replies cross threads only as owned Zterm `Send` data; attachment checkpoints contain no
  Ghostty handles, borrows, selections, render state or scrollback copy.
- Keep PTY bytes fixed-capacity, blocking and no-drop; bound control requests and make scheduling
  fair so output and queries cannot starve each other。
- 增加独立 bounded `PtyWriterActor`，由 terminal actor 单一有序地产生 user-input/reply/
  resize commands。terminal actor 不执行或等待 kernel write/flush；writer full/failure、
  reply byte overflow均 terminal-fatal，child interrupt/wait 保持独立。
- The owner performs resize preflight, obtains a typed native-resize acknowledgement from the PTY
  actor, then applies Ghostty resize and revision publication. Predictable validation failure mutates
  neither side; unexpected post-PTY Ghostty failure becomes terminal-fatal rather than serving
  divergent usable state.
- PTY EOF does not immediately drop the model. Final snapshot/history stay queryable until explicit
  natural/explicit finalization drains and shuts down the actor.

## Parser, Callback and Security Semantics

- Preserve one revision per non-empty ingest and successful resize, including same-size resize;
  empty ingest remains a no-op.
- Callbacks are synchronous and non-reentrant. They only copy bounded reply/event data into
  owner-local state, never perform PTY/network/filesystem/blocking work, and never allow a Zterm
  panic to cross C.
- Callback replies are enqueued to the ordered PTY writer before the actor processes a later command；
  callback本身及 model actor都不等待实际 write。
- Preserve exact declared DA/DSR/CPR replies. Keep OSC 52 denied and keep unknown OSC/DCS/APC/
  graphics payloads out of state, replies, events, snapshots, deltas, logs and Debug.
- The safe wrapper lacks equivalent callbacks for some legacy diagnostics. `UnsupportedSequence`
  may become bounded classification-or-silent-drop; do not add a second parser to recover it.
- Ghostty `VT_PROCESSING_ERROR` is a sticky informational bit, not a per-write failure. Observe it at
  most as a no-payload bounded diagnostic; malformed/unknown bytes alone must not kill a session.
  Callback/invariant/resource failure still fails closed.

## Snapshot, Delta and History

- Keep Zterm ANSI snapshot/delta/history wire and protobuf shape. Never expose Ghostty binary
  snapshots, opaque handles or ABI types on the wire or in persistence.
- 一个 actor-owned RenderState 只 update 一次并维护 canonical Zterm surface；Ghostty dirty
  state 不能作为 per-attachment baseline。attachment 只保存 bounded owned rows/fingerprints。
- Zterm allowlisted encoder 生成 history-first/screen-second full snapshots 与
  `CUP + SGR0 + EL2 + encoded row` deltas，并在 incompatible checkpoint 或 delta不更小时
  resync。输出 vocabulary 仅为 printable UTF-8、current SGR、cursor/clear、单一 screen
  metadata 和 controlled mode constants；不使用 Ghostty general formatter作为 wire codec。
- OSC/DCS/APC、OSC8 URI、palette/PWD、graphics、protection、arbitrary modes 和当前 DTO无法
  表达的 style attributes不能进入 state/wire。
- Configure both Ghostty byte and line scrollback limits. Since internal pruning is page-granular,
  expose only the newest logical configured-row window and measure actual native memory/headroom.
- Preserve history epoch/Changed/Gap semantics using an owner-local TrackedGridRef anchored at the
  logical oldest row；anchor invalid/moved、resize/reflow或 identity不确定时推进 epoch。History
  paging uses bounded length-aware GridRef reads and must not scroll the live viewport.
- Ghostty provides authoritative scrollback/reflow/modes/encoding/selection primitives, but Zterm
  still owns UI gesture routing, live-vs-history ownership, pixel/cell mapping and network cursors.
  The migration does not automatically fix every client hit-testing bug.

## Capability and Resource Contracts

- Child profile 固定 `TERM=xterm-256color`、`COLORTERM=truecolor` 与当前 exact DA/DSR/CPR；
  不继承 Ghostty/kitty/tmux outer identity，palette/default-color/theme query静默忽略。
- Admission 使用 checked reservation formula：visible main+alternate、explicit scrollback byte
  cap、one measured page slack、canonical/encoder scratch、controller+pending-takeover checkpoints、
  all reader/control/writer/reply mailboxes与 fixed engine overhead。Foundation RSS是独立验证门。
- core/proto无 Ghostty只意味着 mobile不链接 host engine；当前 ANSI wire仍要求 mobile
  renderer使用 ANSI parser/widget，semantic surface留给后续 capability。

## Platform and Cutover Gates

- Ship evidence remains macOS arm64/x86_64 at macOS 13 and glibc Linux arm64/x86_64 at glibc
  2.28, with no Ghostty dynamic dependency. Windows remains hosted compile/shared-boundary evidence
  until runtime support is separately claimed.
- This task does not build a local engine for iOS/Android. Their acceptance boundary is a Ghostty-free
  `zterm-core`/`zterm-proto` dependency tree and unchanged Zterm wire.
- Before deleting `vt100`, pass semantic differential corpus, query/security cases, scrollback and
  real-memory gates, actor concurrency/failure/EOF tests, snapshot/delta recovery, real PTY lifecycle,
  four native release builds, Windows compile, offline build, license/SBOM and static-link checks.
- During migration only, old/new engines may coexist in test comparison. Final code contains no
  runtime double parser, fallback feature or `vt100`/`vte`; rollback is a source revert.
