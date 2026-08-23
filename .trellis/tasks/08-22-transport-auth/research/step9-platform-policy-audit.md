# Step 9 platform and policy audit

- Date: 2026-08-23
- Scope: task-added or task-modified `core`, `proto`, `daemon`, and `cli`
  surfaces for `.trellis/tasks/08-22-transport-auth`
- Mode: read-only source audit plus local/offline checks

## Result

The checked Unix/private boundaries are structurally gated, the current public
CLI has not exposed M7/M8 commands or a production state-path override, and no
0-RTT API or sensitive tracing call site was found. Source checkout policy,
workspace version policy, the repository/Relay secret scan, native shared-lib
compilation, and native core/proto tests passed.

One acceptance qualification remains:

1. The installed Windows Rust target could not reach zterm Rust compilation on
   this macOS host because the native Windows C/assembler SDK is absent. Hosted
   `windows-latest` remains the required acceptance evidence.

## Platform boundary audit

### Shared crates

- `crates/core/src/{authorization,device,pairing,transport}.rs` and
  `crates/proto/src/lib.rs` contain no Unix imports or OS-specific `cfg` branch.
  Core remains transport-neutral; protobuf generation remains in the proto
  crate.
- `crates/daemon/Cargo.toml` keeps `nix` under
  `[target.'cfg(unix)'.dev-dependencies]`; the production daemon dependency set
  has no new unconditional Unix crate.
- Native offline compilation passed:

  ```text
  cargo check --offline -p zterm-core -p zterm-proto -p zterm-daemon \
    --lib --all-features
  => PASS
  ```

### `service.rs`

- Unix-only imports, `RemoteDeviceAccess`, `BrokerRemoteDeviceAccess`,
  `DeviceManagement`, device/pair/session dispatch, protocol helpers, and
  service tests are individually or collectively `#[cfg(unix)]`.
- The shared `DaemonService` type gates every Unix-owned field. Its non-Unix
  constructors build the intentional empty unsupported-platform placeholder.
- The unconditional projections (`ProtocolStatus`, `DaemonReadiness`,
  `DaemonStatus`, and `NetworkObservation`) contain only shared domain values.
  No Unix socket, file descriptor, Iroh secret, or SQLite handle crosses that
  public boundary.

### `local_ipc.rs`

- Unix listener/stream imports, limits, server tasks, codec helpers, mutation
  state, and all request implementations are `#[cfg(unix)]`.
- `LocalClient` retains only a `PathBuf` on non-Unix. Its non-Unix methods
  return the typed `UnsupportedPlatform` boundary rather than importing or
  emulating a Unix socket.
- `LocalDeviceClient` has explicit non-Unix unsupported methods.
  `LocalPairingClient` is wholly Unix-gated, including its randomness and
  secret-bearing request path.
- Local IPC unit tests are `#[cfg(all(test, unix))]`; new real Unix socket
  integration targets use crate/item Unix gates.

### `lifecycle.rs`

- Unix filesystem imports, lifecycle timing constants, detached spawn,
  `local_unix` lock/socket types, production composition, listener recovery,
  and cleanup helpers are fully `#[cfg(unix)]`.
- `DaemonLauncher::ensure`, `ensure_current_daemon`, and
  `run_internal_daemon` retain explicit non-Unix `UnsupportedPlatform`
  behavior. The private non-Unix helper contains no native handle.
- Production always reaches state through `production_user_paths()` and
  `LocalRuntime::current()`. Task-private paths are accepted only by
  doc-hidden Rust test constructors; no argv flag selects a state root, socket,
  identity, or database.

### `pairing_service.rs` and adjacent daemon modules

- The pairing runtime uses shared Rust/Iroh/Tokio types and contains no Unix
  import. Its filesystem/`nix::Uid` harness is enclosed by
  `#[cfg(all(test, unix))]`.
- `operations.rs` gates Unix metadata/permission/socket inspection and provides
  non-Unix branches. `session.rs` gates Unix process probes and Unix behavior
  tests.
- Static inspection found no unguarded `std::os::unix`,
  `tokio::net::Unix*`, or `nix::*` reference in the named shared production
  surfaces.

### Checkout bytes

The required policy script passed:

```text
sh tests/source-policy.sh
=> source checkout policy verified
```

Because task-added Rust files are currently untracked and the script starts
from `git ls-files`, this audit also checked all tracked and untracked `*.rs`
paths directly. Every path resolves to `eol=lf`, and no file contains a
carriage return.

## Windows target evidence

Installed targets were read without modification:

```text
aarch64-apple-darwin
aarch64-apple-ios
aarch64-linux-android
x86_64-pc-windows-msvc
```

Both requested Windows checks were forced offline:

```text
cargo check --offline -p zterm-core -p zterm-proto --all-features \
  --target x86_64-pc-windows-msvc
=> BLOCKED before zterm source: ring 0.17.14 cannot find assert.h

cargo check --offline -p zterm-daemon --lib --all-features \
  --target x86_64-pc-windows-msvc
=> BLOCKED before zterm source: ring cannot find assert.h;
   blake3 cannot find ml64.exe; bundled SQLite cannot find stdlib.h
```

`rustc --print cfg --target x86_64-pc-windows-msvc` succeeds and reports
`windows`, `target_family="windows"`, and `target_os="windows"`, but a Rust
target installation alone does not provide the MSVC C headers, assembler, or
libraries needed by these dependencies. No target, SDK, or dependency was
installed, and no network access was attempted. The repository already has a
`windows-latest` CI row which runs shared `--lib` Clippy/tests; that hosted job,
not this incomplete macOS cross-build, is the acceptance owner.

## 0-RTT audit

The following source search produced only the explanatory comment in
`network.rs` which says the fully accepted `Connection` cannot come from
Iroh's 0-RTT API:

```text
rg -i 'into_0rtt|accept_0rtt|0[-_. ]?rtt|early[_ -]?data' \
  crates/core crates/proto crates/daemon crates/cli proto
```

There is no `into_0rtt`, `accept_0rtt`, or early-data call. Inbound routing
awaits `Incoming::accept()`/`Accepting`; outbound normal and pair paths await
`Endpoint::connect(...)` and then validate the authenticated remote ID before
application work.

## Secret, ticket, proof, and diagnostics audit

### Passing boundaries

- `DeviceIdentity` has a custom `Debug` containing only public `DeviceId`;
  `NetworkStartup` omits its `SecretKey` from custom `Debug`.
- Core `PairSecret`, `PairFingerprint`, `PairProof`, and `PairAccepted`, plus
  daemon `PairTicketText`, `PairOfferCreated`, pairing state values, and
  `LocalPairAcceptInput`, use custom redacted `Debug`/`Display` where exposed.
- `FrameDecoder` has a custom `Debug` which reports byte counts rather than its
  buffered body. Ticket, proof, confirmation, frame, and encoded-request
  owners use `Zeroizing` and explicit `zeroize()` at the inspected adapter
  boundaries.
- A targeted search found no production tracing/printing/formatting call site
  which combines a private key, ticket, secret, proof, sensitive generated
  message, decoded frame, request, or response with `Debug` or tracing.
- The only tracing sites in the task-changed daemon modules are lifecycle/local
  listener state and bounded generic errors; none receives a raw ticket/proof
  value.
- Store/status/doctor searches found no secret/ticket/proof SQL column or
  projection. Status/doctor expose redacted network state and path counts, not
  a direct IP or route cache.
- Local secret scan passed:

  ```text
  sh tests/relay/secret-scan.sh
  => repository secret scan passed
  ```

### Resolved finding P1: sensitive proto/raw-frame `Debug`

The follow-up implementation now applies prost `skip_debug` to `WireFrame`,
all pair handshake messages, `PairTicketV1`, `LocalPairCreateResponse`, and
`LocalPairAcceptRequest`. Their manual `Debug` implementations redact bearer,
nonce, proof, and opaque payload bytes. `DecodedFrame` now reports only kind,
request/deadline metadata, and payload length. The proto compatibility sentinel
test proves these projections do not contain fixed secret bytes.

## CLI and production argv policy

`cargo run --offline -q -p zterm-cli -- --help` returned exactly these public
commands:

```text
setup
status
doctor
daemon
logs
```

There is no public `pair`, `device`, `connect`, or `session` command. The only
hidden top-level flag is `--internal-daemon`; it selects the production daemon
entry but accepts no state path. Setup flags remain `--name`, `--profile`, and
`--relay-url`. No `--state`, `--root`, `--home`, `--socket`, `--database`, or
identity override is declared.

`doctor` calls `LocalRuntime::observe()`. When the daemon is stopped it reads
only committed local setup and emits "network observation was not attempted";
it does not call `ensure`, construct `NetworkStartup`, bind an Endpoint, or run
address lookup. Running doctor/status consume the same typed `DaemonStatus` /
`NetworkObservation` projection for human and JSON output.

## Other local evidence

```text
CARGO_NET_OFFLINE=true sh tests/workspace-version.sh
=> workspace product version 0.1.1 is inherited by all 5 crates

cargo test --offline -p zterm-core -p zterm-proto --all-features
=> PASS: 63 tests total, including pairing proof/debug, decoder redaction,
   ticket failure redaction, compatibility, and golden vectors
```

No Iroh Endpoint was constructed, no UDP socket or DNS lookup was requested,
and no public Relay/Internet test was run during this audit.
