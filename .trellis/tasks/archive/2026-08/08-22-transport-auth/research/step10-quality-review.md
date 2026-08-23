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

The implementation has no unresolved task-owned code or external-environment
gate. The final scope-corrected head `4ec0cba` passed all seven hosted jobs in
run `32615123176`: Linux x86_64/arm64 (including real-Iroh loopback and
cross-UID gates), Windows, macOS arm64/Intel, dependency policy, and the
repository-wide optional Relay bundle. Relay/path acceptance composes the
already accepted Foundation official-n0 Case C, the current exact production
profile regression, and the current Linux real-Iroh M5-M6 loopback tests. The
optional self-hosted Relay is not an M5-M6 completion condition. This macOS
review did not execute an Endpoint locally; the task is ready to archive.

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

### F5: Unix-only private state remained compiled on Windows

The first hosted run found three `dead_code` errors in the Windows shared
boundary: the hidden device client's Unix socket field and the complete private
Session remote-attachment-count query chain. The stable non-Unix client API now
constructs a stateless value and continues to return typed
`UnsupportedPlatform`; the Unix-only service method, actor command, dispatch
arm, and helper are gated together. Hosted Windows then passed compile, shared
contract tests, and documentation with `-D warnings`.

### F6: a pre-poll notification was not a lock-queue barrier

The second hosted run failed the authorization fairness regression on Linux
arm64 because the test sent "writer started" before the first poll of
`write_owned`. On a multi-thread Tokio runtime, that send can wake a receiver on
another worker; the later reader can be polled before the writer ever enters the
fair queue. Source order without a yield is not a cross-worker happens-before.

The doc-hidden test seam now wraps the real lock future in
`tokio::task::unconstrained`, pins it, and sends its observer after the first
actual poll. `Pending` therefore proves the writer was queued, while `Ready`
means the write guard is already held. The later reader uses the same barrier
and explicitly asserts its first poll is `Pending`. Production lock paths are
unchanged; Tokio still owns wake-up and cancellation behavior. Independent
review found no issue, the failing regression passed 500/500 in isolated test
processes, and the complete revoke-order race passed 50/50. The exact formerly
failing test then passed in the clean Linux arm64 hosted run.

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
| focused authorization/revoke tests after F6 | pass; 5/5 and 2/2 |
| F6 isolated stress loops | pass; fairness 500/500, full revoke order 50/50 |

The pre-follow-up workspace run included 85 daemon unit tests plus all safe integration
targets: authorization, duplicate arbitration, local pair/device/session IPC,
network lifecycle, pairing manager/protocol/secrets, route/path planning,
persistence, revoke races, and stream limits. `connection_broker` and the real
case in `two_daemon_transport` compiled but were ignored on macOS before bind.
The later multi-process production-PairingService target also passed local
lib-test compilation and Clippy; the hosted evidence below records its actual
Linux execution.

## Hosted CI evidence

| Run | Evidence | Result / follow-up |
| --- | --- | --- |
| [`32608814512`](https://github.com/leonfox28/zterm/actions/runs/32608814512), `193e008` | Linux x86_64 and arm64 both executed `connection_broker` (1/1), `two_daemon_transport` (2/2), and the two-process production `PairingService` gate; both macOS rows, dependency policy, and Relay bundle also passed | Windows alone failed on the three F5 dead-code boundaries |
| [`32609483826`](https://github.com/leonfox28/zterm/actions/runs/32609483826), `80f8852` | Windows shared compile/tests/docs passed; Linux x86_64, both macOS rows, dependency policy, and Relay bundle passed | Linux arm64 alone exposed F6 in `revoke_waits_for_in_flight_commit_and_rejects_stale_generation`; a clean rerun on the first-poll fix is required |
| [`32610795848`](https://github.com/leonfox28/zterm/actions/runs/32610795848), `4b85260` | All seven jobs passed on one head: Linux x86_64/arm64, Windows, macOS arm64/Intel, dependency policy, and Relay bundle. Both Linux rows executed the real broker/two-daemon/two-process pairing gates; arm64 also passed the exact F6 regression | clean hosted matrix achieved; later ordinary runs add the cross-UID evidence |
| [`32611781285`](https://github.com/leonfox28/zterm/actions/runs/32611781285), `5e021cd` | All seven ordinary jobs passed; both Linux rows also executed the harness-false cross-UID test under `CI=true`, where unavailable `sudo -u nobody` is a hard failure rather than a skip | cross-UID environment gate achieved |
| [`32612691539`](https://github.com/leonfox28/zterm/actions/runs/32612691539), `bf3d313` | All seven ordinary jobs passed again; production Rust/proto remained the same as the clean M5-M6 head | clean matrix and cross-UID evidence retained |
| [`32615123176`](https://github.com/leonfox28/zterm/actions/runs/32615123176), `4ec0cba` | All seven jobs passed after restoring the official-n0 acceptance scope: Linux x86_64/arm64, Windows, macOS arm64/Intel, dependency policy, and optional Relay bundle | final archive head; no public/self-hosted M5-M6 gate remains |

The later green runs preserve the required clean matrix and add cross-UID
evidence; the first two runs are retained to document the task-owned failures
and their fixes. None of these ordinary Linux runs is described as public-n0
runtime evidence: their M5-M6 Endpoint fixtures are loopback-only with Relay
disabled.

## Acceptance mapping and evidence boundary

| PRD acceptance area | Current evidence | Remaining evidence |
| --- | --- | --- |
| one endpoint, exact profiles/ALPNs, lifecycle | production composition/profile/injected lifecycle tests plus successful Linux x86_64/arm64 real-Iroh loopback gates; the private pair router is not separately claimed as `run_daemon` evidence | — |
| singleflight and duplicate convergence | pure gates plus real `connection_broker` on Linux x86_64/arm64 in the clean matrix | — |
| resource isolation | pure permit/frame/deadline tests, local Session/PTY corpus, and real Linux broker streams in the clean matrix | — |
| directional pairing | fake-transport faults plus the two-process production gate on Linux x86_64/arm64 in the clean matrix | — |
| ticket/proof/replay/ambiguity secrecy | golden vectors, manager/service faults, proto/debug and SQLite sentinels across the clean Unix/Windows matrix | — |
| pre-frame authorization/current generation | registry/broker gates and real one-way normal ALPN on both Linux architectures; F6 passed on arm64 | — |
| durable revoke without Session loss | `revoke_races`, device IPC, principal detach, restart preload, first-poll fairness stress, and the clean arm64 rerun | — |
| fresh/cache/ticket fallback and path safety | route/path pure tests including F1; current `iroh_profile_gate` for exact official map/QAD/lookups/ALPNs; accepted Foundation Case C for official WSS/TCP Relay fallback with non-DNS UDP blocked; Linux real-Iroh loopback M5-M6 composition | — |
| same-UID hidden adapters and no M8 CLI | local pair/device IPC, CLI source/help tests, and Linux x86_64/arm64 cross-UID execution in green hosted runs | — |
| full quality/platform matrix | all seven jobs passed again on `bf3d313` in run `32612691539` | — |

The existing `ci.yml` needs no task-owned wiring change: Unix matrix rows run
the full workspace suite (so Linux executes the real-Iroh targets, including
the lib-test self-spawn pair gate, while macOS honors their ignores), Windows
runs the shared library boundary, and the Relay job runs the optional
self-hosted image/Compose static contract. That repository-wide static job is
not M5-M6 runtime evidence.

No public or self-hosted Relay gate remains for M5-M6. The optional deployment
keeps its direct post-update handshake runbook, while representative
two-physical-network automatic discovery remains explicitly deferred to parent
M10. The final scope-corrected head passed the normal non-public quality and
hosted checks; only parent-progress bookkeeping, task validation, finish-work,
and archive remain.
