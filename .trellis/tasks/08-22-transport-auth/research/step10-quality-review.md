# Step 10 Trellis quality review

- Date: 2026-08-23
- Reviewer mode: independent `trellis-check` after all implementation slices
- Local platform: macOS arm64
- Network safety: no Iroh `Endpoint`, UDP listener, DNS lookup, or public Relay
  test was executed after the user rejected the macOS firewall prompt

## Outcome

The task-owned code is locally compile-, lint-, test-, documentation-, policy-,
and task-validation clean. Cross-layer create/accept/revoke flows use one set of
daemon owners, errors stay typed, sensitive values are redacted/zeroized, and
the public CLI remains within the M5-M6 boundary.

The task is **not ready to archive or mark M5-M6 complete**. Linux real-Iroh,
disposable/public Relay, hosted Windows, and the required platform matrix remain
external acceptance gates. A Linux-only self-spawn target now composes the
production `PairingService` in two separate task-private owner processes, but
this macOS review compiled it without executing either Endpoint.

## Findings fixed during review

### F1: later relay sources could be starved

`RouteResolver` treated `max_relay_hints` as a bound on the merged
fresh/cache/ticket list. Four fresh results therefore removed every cache and
ticket fallback even when all fresh dials failed. The core contract defines the
value per ticket or route cache, and each source is independently validated.

The merge now caps each source, deduplicates globally, and retains ordered later
sources. `full_fresh_source_does_not_starve_cache_or_ticket_fallback` is the
socket-free regression.

### F2: normal QUIC unidirectional streams were not disabled early

Normal connections applied only the bidirectional limit, and only after
Hello/Welcome promotion. An authenticated peer could therefore open
unidirectional streams which no product actor accepts. Incoming and outgoing
connections now apply the configured bidirectional limit plus `uni = 0` before
the application handshake. A source-contract regression proves both call sites
remain earlier than their handshake.

### F3: specification and audit drift

The daemon crate still described Iroh/pairing as future work, and the platform
audit still listed sensitive generated `Debug` as open after the proto fix. The
crate docs and audit are corrected. The new active backend
`transport-auth.md` records the executable ownership, routing, pairing,
authorization, revoke, and platform-test contracts.

### F4: multi-process helper cleanup and ticket-channel privacy

The first Linux self-spawn harness piped child output and used unbounded child
waits. A full pipe or stuck helper could hang CI, and inherited temporary-path
permissions were weaker than the bearer control channel needed. The reviewed
gate now discards child output, shares one exit deadline, kills and boundedly
polls/reaps on timeout or unwind, creates the control root as `0700`, and sets
each Unix control socket to `0600`. Ticket-bearing control packets are bounded,
deadline-protected, and dropped or zeroized immediately after transfer. The
long-term transport/auth spec records these test contracts and explicitly does
not treat the private ALPN router as `run_daemon`/`NetworkStartup` evidence.

No unresolved task-owned code finding remains from this review.

## Cross-layer traces reviewed

### Pair create

Same-UID local request kind 12 -> strict unary decoder -> semantic fingerprint
and bounded operation cell -> online/network observation -> `PairingManager`
offer -> canonical core ticket -> proto text adapter -> zeroizing local reply.
No SQLite authorization or known-device row is written.

### Pair accept

Same-UID kind 14 -> ticket/fingerprint validation -> shared `DeviceDirectory`
alias reservation -> transient pair ALPN -> TLS ID check ->
Begin/Challenge/Proof -> proof-before-CAS -> host StoreActor authorize ->
registry publish -> PairAccepted -> normal Hello/Welcome confirmation ->
controller StoreActor known-device/verified-relay commit -> directional
`DeviceSummary` response. Started-but-unanswered StoreActor work is reconciled
as outcome unknown rather than rolled back speculatively.

### Revoke

Same-UID kind 20 -> exact DeviceId -> fair authorization write guard -> SQLite
revoke -> registry/watch publish -> close matching broker candidates -> detach
matching remote principal from retained Sessions -> directional response.
Database failure stops before any live owner changes; repeated revoke is
idempotent.

### Network lifecycle

Production composition prepares one identity/profile/broker/pairing service,
then the supervisor owns the sole Endpoint outside listener recovery. Local
readiness does not depend on network state. Final/fatal cleanup uses bounded
pairing then broker/Endpoint quiesce without making connection health the
Session lifetime owner. The local-only test daemon has an explicit entry which
never prepares Iroh.

## Local quality evidence

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo check --workspace --all-targets --all-features` | pass |
| workspace Clippy with `-D warnings` | pass |
| `CARGO_NET_OFFLINE=true cargo test --workspace --all-features` | pass; real-Iroh macOS tests ignored |
| `cargo doc --workspace --all-features --no-deps` | pass |
| `CARGO_NET_OFFLINE=true cargo deny check` | pass; allowed duplicate warnings only |
| source checkout policy | pass |
| workspace version policy | pass |
| Relay static/Compose contract | pass |
| Relay/repository secret scan | pass |
| Trellis task JSONL validation | pass |
| `git diff --check` | pass |

The pre-follow-up workspace run included 85 daemon unit tests plus all safe integration
targets: authorization, duplicate arbitration, local pair/device/session IPC,
network lifecycle, pairing manager/protocol/secrets, route/path planning,
persistence, revoke races, and stream limits. `connection_broker` and the real
case in `two_daemon_transport` compiled but were ignored on macOS before bind.
The later multi-process production-PairingService target also passed lib-test
compilation and Clippy; Linux must execute it before it becomes runtime evidence.

## Acceptance mapping and remaining gates

| PRD acceptance area | Current evidence | Remaining evidence |
| --- | --- | --- |
| one endpoint, exact profiles/ALPNs, lifecycle | profile and injected pre-bind lifecycle tests; composition review | Linux successful bind/close and full hosted matrix |
| singleflight and duplicate convergence | pure broker/duplicate tests; real target compiles | execute real `connection_broker` on Linux |
| resource isolation | pure permit/frame/deadline tests plus local Session/PTY drain corpus | Linux hostile real-stream exercise where applicable |
| directional pairing | core/StoreActor/PairingService fake-transport tests; two-process production gate compiles | execute the full PairingService gate on Linux |
| ticket/proof/replay/ambiguity secrecy | golden vectors, manager/service faults, proto/debug and SQLite sentinels | platform CI rerun |
| pre-frame authorization/current generation | registry/broker pure tests and revoke barriers | real normal-ALPN Linux execution |
| durable revoke without Session loss | `revoke_races`, device IPC, principal detach, restart preload | platform CI rerun |
| fresh/cache/ticket fallback and path safety | route/path pure tests including F1 regression | self-hosted/public Relay execution on Linux |
| same-UID hidden adapters and no M8 CLI | local pair/device IPC and CLI source/help tests | Linux cross-UID gate |
| full quality/platform matrix | all local offline gates pass | macOS Intel, Linux x86_64/arm64, hosted Windows shared compile/tests |

The existing `ci.yml` needs no task-owned wiring change: Unix matrix rows run
the full workspace suite (so Linux executes the real-Iroh targets, including
the lib-test self-spawn pair gate, while macOS honors their ignores), Windows
runs the shared library boundary, and the Relay job runs `static.sh`. CI still
does not replace the explicit public Relay handshake.

Required next commands run only in their intended environments:

```sh
# Linux with real Iroh loopback enabled
cargo test -p zterm-daemon --test connection_broker
cargo test -p zterm-daemon --test two_daemon_transport
cargo test -p zterm-daemon --lib --all-features \
  pairing_service::multiprocess_test::two_process_production_pairing_service_is_directional_and_reuses_one_endpoint -- --exact
sh tests/core-local-daemon/cross-uid.sh

# Disposable/public Relay acceptance
sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn

# Hosted Windows runner with the MSVC SDK
cargo check -p zterm-core -p zterm-proto --all-features
cargo check -p zterm-daemon --lib --all-features
```

Do not run `trellis-finish-work`, archive this child task, or mark the parent
M5-M6 complete until those gates, including the Linux multi-process
PairingService result, are recorded.
