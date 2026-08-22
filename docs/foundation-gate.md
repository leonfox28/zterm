# Foundation Gate final report

Status: **GO with automatic address-discovery evidence deferred to parent M10**.
The TerminalModel, PTY lifecycle, retained-drain, black-box compatibility,
resource, platform, and aggregate quality Gates passed. In the nested
Colima/Patchbay/TUN lab, the official Relay fallback and controlled direct-path
sample passed while the unchanged product profile stayed relayed. The approved
Gate interpretation permits Foundation work to continue and defers automatic
discovery across two real networks to parent milestone M10. The parent task may
now enter M2; this conclusion does not claim that automatic NAT traversal has
been validated on representative physical networks.

## Scope and environment

- Rust 1.98.0 (`aarch64-apple-darwin` host; `aarch64-unknown-linux-gnu` Gate
  container).
- Colima 0.10.3, Docker Engine 29.5.2 on Linux arm64.
- One ephemeral privileged container named `zterm-foundation-network-gate`;
  the repository was mounted read-only and the container was absent after the
  run.
- Patchbay created a fresh isolated Home NAT × Home NAT lab for each Case. Only
  the disposable container received the IX egress route and NAT rule.
- Bettbox fake-IP DNS returned `198.18.0.0/15` addresses for the n0 Relay names,
  which collides with Patchbay's simulated range. The runner therefore resolves
  the four Relay A records through DNS-over-HTTPS before entering Patchbay and
  injects those records only into the disposable lab DNS. Product DNS behavior
  is unchanged.
- After that retained run, Bettbox was configured to exclude `iroh.link` from
  fake-IP and route the suffix directly; local DNS then returned real public
  addresses. This removed the local name-resolution collision but was not used
  as new Case A path evidence and did not trigger another A/B/C run.
- The optional self-hosted Relay, which is excluded from this product profile,
  and its deployment, OpenResty, Cloudflare, firewalls, and server state were
  not changed.

Exact resolved dependencies used by the implemented scope:

| Dependency | Version / enabled feature boundary |
| --- | --- |
| `iroh` | 1.0.3; default features off; `portmapper`, `tls-ring` only |
| `patchbay` | 0.6.0; Linux-only test dependency |
| `noq` | 1.1.1 (resolved transitively by Iroh) |
| `tokio` | 1.53.1; `macros`, `rt`, `sync`, `time` requested by zterm |
| `anyhow` | 1.0.104; test-only |
| `futures-util` | 0.3.34; test-only |
| `vt100` | 0.16.2; private implementation of `zterm-core::terminal` |
| `vte` | 0.15.0 (resolved transitively by `vt100`) |
| `unicode-width` | 0.2.2 (resolved transitively by `vt100`) |
| `portable-pty` | 0.9.0; default feature set (empty), private behind `zterm-platform::pty` |
| `nix` | 0.28.0; direct Unix `fs`, `user` features for effective-account lookup and pre-spawn access checks |

## Effective Iroh profile

The retained `InfrastructureProfile` builds from Iroh's `presets::Minimal`,
installs the Pkarr publisher, Pkarr resolver, and DNS endpoint lookup from
Iroh's explicit public production constants, then selects
`RelayMode::Default`. This reproduces the production parts of `presets::N0`
without allowing `IROH_FORCE_STAGING_RELAYS` to redirect lookup services. The
Relay map still comes from Iroh's pinned production default rather than a
copied zterm list. The executable profile test confirmed the exact Iroh 1.0.3
map:

- `use1-1.relay.n0.iroh.link` (US east)
- `usw1-1.relay.n0.iroh.link` (US west)
- `euc1-1.relay.n0.iroh.link` (Europe)
- `aps1-1.relay.n0.iroh.link` (Asia-Pacific)

Each entry retains Iroh's default QAD configuration on UDP 7842. The same test
asserted that the production map is disjoint from `RelayMode::Staging` and does
not contain `relay.zenithconsulting.cn`. The n0 DNS/Pkarr publisher, resolver,
and DNS lookup remain installed; publication remains relay-only; port mapping
remains enabled; and the Foundation Gate accepted its then-temporary
`zterm-gate/1` ALPN. M3 subsequently replaced it with product ALPN `zterm/1`
when the versioned wire contract landed. An isolated child
process regression also confirmed that setting `IROH_FORCE_STAGING_RELAYS`
does not change any production lookup URL, DNS origin, or Relay entry.
Identities were generated in memory and were not written to `~/.zterm`.

## Double-NAT evidence

The final runner executed all three Cases before returning its aggregate
result. Each Case used a new Patchbay lab, began on an official production
Relay, and verified three independent end-to-end encrypted bidirectional QUIC
streams with distinct payloads. Evidence omits endpoint IDs and IP addresses.

| Case | Candidate evidence | Initial → final selected path | Path-event timeline | Streams | Result |
| --- | --- | --- | --- | --- | --- |
| A — product profile | two public-API candidates per endpoint; internal source type unavailable | Relay → Relay | no selection change | 3/3 | automatic direct not observed |
| B — known-candidate diagnostic | raw UDP passed; exact reflected `Config` candidate injected on both endpoints | Relay → Direct | `opened:direct`, `selected:direct` | 3/3 | controlled direct path passed |
| C — UDP blocked | one public-API candidate per endpoint; all endpoint non-DNS UDP egress blocked | Relay → Relay | no selection change | 3/3 | encrypted WSS/TCP Relay fallback passed |

Home Relay selection varied by endpoint and run within the official production
map; it is dynamic and is not a product region pin.

For Case B, a Patchbay reflector first measured the real Home NAT mappings.
The raw simultaneous-UDP control passed on separate ports. The fixture then
measured the Iroh ports without opening peer-specific NAT filters and injected
those exact addresses as the diagnostic `Config` candidates. Iroh opened and
selected a direct path before all three streams completed. This proves that the
tested Iroh transport and Home NAT topology can hole-punch when usable external
candidates are known.

For Case C, both endpoint namespaces allowed DNS but dropped all other outbound
UDP before either Iroh endpoint was bound. The authenticated Iroh connection
stayed on an official Relay and all three streams completed. With QAD/direct UDP
unavailable, that result proves the encrypted Relay fallback over HTTPS
WebSocket/TCP rather than a direct path.

`EndpointAddr` exposes direct socket addresses but not Iroh's internal
`DirectAddrType`. The retained runner therefore labels only the exact injected
Case B address as `Config`; all other candidates are `Unclassified` rather than
claiming they are local, QAD, or port-mapped.

## Network hard-checkpoint conclusion

The current aggregate evaluator maps the retained A/B/C evidence to:

```text
NETWORK_GATE=GO_WITH_DEFERRED_ADDRESS_DISCOVERY: Case A stayed relayed in the nested Colima/Patchbay/TUN lab; Case B became direct; Case C official WSS/TCP Relay fallback passed; real two-network automatic discovery is deferred to parent M10
```

This is not a Relay failure: Case C proves official WSS/TCP forwarding works
when direct UDP is unavailable. It is also not a general hole-punch failure:
Case B proves direct selection in the same Home NAT topology once usable
external candidates are supplied. Case A is retained as deferred
address-discovery evidence: the product profile did not automatically select a
direct path in this nested Colima/Patchbay/TUN environment despite the official
map carrying QAD configuration.

The result does not claim that official Iroh QAD fails on ordinary physical
networks, and it does not claim Case A passed. The validated B direct path and C
official Relay fallback satisfy the revised transport prerequisite for Step 2.
Parent M10 must still validate the unchanged product profile across two real
external networks before treating automatic discovery as representative.

The A/B/C transport experiment was not rerun after changing only the aggregate
verdict and exit semantics. Its raw evidence above is unchanged; transport
profile, candidate construction, and path observation code are unchanged.

## Step 2 TerminalModel result

The retained `zterm-core::terminal` boundary uses exactly `vt100 0.16.2`
behind private fields. Public snapshots, deltas, checkpoints, states, modes,
cells, side events, resource projections, and errors are all zterm-owned types;
no parser type or wire format crosses the boundary. At the time of this
retained report, `BuildIdentity` reported `phase-one-foundation-gate` and the
CLI was a side-effect-free build probe. M3 now reports
`phase-one-core-local-daemon` and provides lifecycle commands; terminal/session
attach remains unimplemented.

Ordered non-empty ingest and every successful resize advance one checked `u64`
revision. Empty ingest is a no-op. Invalid sizes, revision overflow, and
resource-projection overflow are typed errors, and the overflow regression
proves state is not mutated before returning the error. Side events retain at
most 32 entries per update and replace overflow with one dropped-count event;
title/icon payloads retain at most 256 source bytes.

The fixed ANSI corpus passed with whole-input, one-byte, fixed-width, and
deterministic pseudo-random chunk boundaries. It covers main/alternate screen
and restoration, clear, scroll region, cursor movement, indexed and RGB color,
wide and combining Unicode, bracketed paste, mouse/focus modes, repeated
resize, DA/DSR/CPR, title, audible/visual bell, OSC 52 rejection, and unknown
OSC/DCS/APC containment. Unknown and rejected payloads do not enter rendered
state, replies, snapshots, or deltas.

The explicitly supported query capability is VT100 primary device attributes
with Advanced Video Option (`CSI ?1;2c`), status OK (`CSI 0n`), and standard or
private cursor position reports. Foundation does not yet set or claim
`TERM=xterm-256color` or `COLORTERM`.

A full snapshot restores active main/alternate selection, visible cells,
cursor and drawing style, supported input modes, focus mode, and bounded
standard scrollback. Applying a snapshot to a fresh model and then one merged
delta from its opaque checkpoint produced the same latest semantic state under
all corpus chunkings. Active-screen transitions also matched. Size mismatch,
future checkpoints, and a delta no smaller than the full payload selected one
full resync rather than another fallback representation.

`vt100` exposes formatted/diff state for only the active visible grid. The
current API therefore snapshots current-screen semantics and standard history
when the main screen is active; it does not serialize the inactive main grid or
its history while an alternate screen is active. A later transition still
formats the new latest active screen, so this does not break the approved
current-state reconnect contract. Concurrent inactive-screen preview would
require a future engine review, not a parallel parser in Foundation.

Step 2 exposes checked arithmetic over vt100 0.16.2's fixed inline cell slots.
It excludes parser state, row/container overhead, snapshots, and transient
workload allocations, so it is not presented as measured memory or an RSS
limit by itself. Step 6 combines it with saturated-process measurements.

## Step 3 PTY lifecycle result

The retained `zterm-platform::pty` boundary owns `PtyHost`, `PtySession`, the
single-transfer `PtyReader`, explicit fixture commands, sizes, child states,
exit statuses, and typed errors. Portable-pty master, reader, writer, child,
killer, status, and error types remain private. Existing `PlatformFacts`,
`current()`, and `build_identity()` APIs remain available.

On supported Unix hosts, current-account login-shell creation looks up the
effective UID through the account database, validates the account home, login
shell, and requested cwd before opening a PTY, then uses
`CommandBuilder::new_default_prog()`. It explicitly sets `HOME`, `SHELL`, and
cwd, so daemon cwd and inherited `$SHELL` do not select session defaults; the
portable-pty Unix adapter supplies login argv0 semantics. The explicit argv
constructor is retained only as a low-level platform fixture primitive, not as
a first-stage arbitrary-command product feature.

One `harness = false` self-child integration binary ran the real macOS PTY
lifecycle without adding a product binary. It proved ordered input/output and
a child-observed resize from 24×80 to 47×123; natural root-child exit returned
code 23; and explicit close terminated the blocking child with SIGHUP and
retained that terminal status. `take_reader` succeeded once. A separate case
wrote 1,048,576 payload bytes—well above a typical kernel PTY buffer—and wrote
an fsynced control-file completion marker only after all payload writes
completed. The test drained and counted the output too, but the marker, rather
than observing the last PTY byte, is the independent progress proof.

Only `close_explicitly()` invokes portable-pty's child killer and then waits.
Zterm adds no signal escalation, process-group policy, restart, or Drop-time
recovery.

The public boundary and non-Unix skip branch are cfg-safe. With the
`x86_64-pc-windows-msvc` Rust standard-library target installed,
`cargo check -p zterm-platform --all-targets --all-features --target
x86_64-pc-windows-msvc` passed locally. A whole-workspace cross-check from
macOS still lacks the Windows MSVC C sysroot required by `ring` (`assert.h`),
so the real Windows CI runner remains authoritative for the complete workspace.
Current-account login shell lookup returns a typed unsupported-platform result
on Windows, Android, and Redox; the Step 3 behavior Gate covers macOS/Linux
Unix hosts.

## Step 4 retained drain and latest-state attachments

`zterm-daemon::terminal_driver` now composes the retained platform and core
boundaries without adding a session registry or wire protocol. One blocking
reader sends every PTY chunk through a fixed-capacity `VecDeque` guarded by
condition variables. A single model-owner thread removes chunks in order,
ingests them into `TerminalModel`, writes only its controlled query replies
back through the same `PtySession`, and publishes one latest revision
watermark. A full queue blocks the reader rather than dropping bytes; the
observable high-water mark can never exceed the configured chunk capacity.

An attachment owns only shared read access to the authoritative model, one
opaque checkpoint, and the latest revision condition. It cannot reach the PTY
session, reader, writer, child, or close method. Intermediate revisions are not
queued. A normal sync returns one merged delta or full snapshot; a consumer
which intentionally discards a stale checkpoint gets exactly one latest full
resynchronization and then establishes a new watermark.

The real-PTY zero-attachment fixture wrote 1,048,848 observed bytes through a
two-chunk queue. Its independent fsynced marker appeared only after the child
had completed more than 1 MiB of writes, while the root child remained alive
waiting for input. The model later contained `BULK-COMPLETE`; the queue reached
but did not exceed its capacity. A separate simulated Iroh connection guard
owned only an attachment. Dropping it reduced the attachment count to zero,
left the child running, and a later attachment observed
`CHILD-STILL-RUNNING`.

The slow-consumer fixture retained a checkpoint at revision 1, then paused
while thousands of colored lines advanced the authoritative model through
roughly two hundred PTY chunks using a three-chunk queue. The child completion
marker and latest state appeared without a consumer sync. After discarding its
old watermark, one full resync replayed to a fresh client model and matched a
separate latest authoritative snapshot semantically. The exact chunk/revision
count is scheduling-dependent; the invariant is that it exceeds one queue
window and the measured pending high-water never exceeds capacity.

## Step 5 black-box compatibility result

One generic external-program adapter exercised tmux 3.7c, Herdr 0.8.2,
Codex CLI 0.148.0, and OpenCode 1.18.20 through the same retained
`PtyHost -> TerminalDriver -> TerminalModel` boundary. The adapter contains no
program-name branches and never compares a full dynamic screen transcript.

tmux ran with `/dev/null` configuration and a task-unique server socket. Herdr
was downloaded from its fixed GitHub release, verified against SHA-256
`a5d4f4d504d8b309c91f811050559300faba31258425f53c50852fc96f6ae574`,
and ran with a temporary home, config, and socket plus `--no-session`. Both
entered the alternate screen, observed the 24x80 -> 47x123 outer resize,
completed 400 colored output lines while no attachment existed, and exposed
the latest marker after one full reattachment resync. The two-chunk byte queue
reached but never exceeded its capacity.

Codex used the installed exact `codex-cli 0.148.0` binary with an empty
temporary `CODEX_HOME`; no account state was copied. Its isolated onboarding
UI uses the main screen in this version, so the result records `screen=main`
rather than falsely claiming alternate-screen use. It still advanced while
detached, retained the exact 47x123 current screen, resynced, and exited without
a prompt. OpenCode's fixed arm64 release archive was verified against SHA-256
`b483e547c029b4f0ba381f0d0c5b420bec48c24c2bbec1fb7f22252bae83da46`;
its isolated TUI used the alternate screen and passed the same resize,
detached-progress, resync, and no-prompt assertions. Neither smoke sent a model
request or retained UI text.

Every adapter waited for its root program to exit successfully. The isolated
Herdr socket and tmux server were absent, then explicit cleanup removed the
temporary homes, archives, and binaries before reporting
`BLACKBOX_CLEANUP=PASS`.

## Step 6 resource result

The stable harness-free `terminal_state` benchmark used the release/bench
profile and emitted one machine-readable record per case. Every model received
the same mixed ASCII, indexed-color, wide/combining-Unicode, and high-update
workload. `/usr/bin/time -l` measured the complete benchmark process on the
Apple arm64 host. Dimensions below are columns x rows; MiB values use 2^20
bytes.

The required shallow 512-line candidate matrix was:

| Sessions | Viewport / scrollback | Fixed-cell reservation | Peak RSS | Benchmark elapsed | Snapshot / delta-or-resync | CPU user + system |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 120x40 / 10,000 | 36.9 MiB | 6.0 MiB | 0.64 ms | 11,206 / 1,121 B | <0.01 s |
| 3 | 120x40 / 10,000 | 110.7 MiB | 10.5 MiB | 1.73 ms | 33,618 / 3,363 B | <0.01 s |
| 16 | 120x40 / 10,000 | 590.6 MiB | 39.3 MiB | 8.74 ms | 179,302 / 17,936 B | <0.01 s |
| 1 | 512x256 / 10,000 | 164.2 MiB | 22.0 MiB | 3.00 ms | 13,213 / 7,232 B | <0.01 s |
| 3 | 512x256 / 10,000 | 492.8 MiB | 46.4 MiB | 8.30 ms | 39,639 / 21,696 B | <0.01 s |
| 16 | 512x256 / 10,000 | 2,628.0 MiB | 204.1 MiB | 42.31 ms | 211,414 / 115,712 B | 0.03 s |

The low shallow RSS is expected because vt100 allocates scrollback rows as
output reaches them. It cannot be used as the admission value. Four saturated
representatives therefore exercised enough lines to fill the configured
history; the runner independently counted each snapshot's retained history and
required it to equal the configured 10,000 or 2,000 rows before accepting the
RSS sample:

| Sessions | Viewport / scrollback | Fixed-cell reservation | Peak RSS | Benchmark elapsed | Snapshot / delta-or-resync | CPU user + system |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 3 | 120x40 / 10,000 | 110.7 MiB | 162.0 MiB | 35.34 ms | 633,696 / 3,333 B | 0.03 s |
| 1 | 512x256 / 10,000 | 164.2 MiB | 329.0 MiB | 43.75 ms | 217,785 / 7,232 B | 0.03 s |
| 3 | 240x80 / 2,000 | 47.5 MiB | 69.5 MiB | 11.57 ms | 133,302 / 6,699 B | <0.01 s |
| 8 | 240x80 / 2,000 | 126.6 MiB | 154.7 MiB | 28.72 ms | 355,472 / 17,864 B | 0.02 s |

The 10,000-row/512x256 candidate is rejected: one saturated session already
reached 329.0 MiB RSS, while three and sixteen sessions reserve 492.8 MiB and
2,628.0 MiB of fixed cells before parser/container overhead. Sixteen sessions
are also rejected at the typical viewport because their fixed-cell reservation
is 590.6 MiB. Conversely, the required three-session case remained bounded
even with 10,000 saturated rows, and eight saturated sessions at the retained
upper viewport/history recommendation remained at 154.7 MiB peak RSS.

The Foundation recommendation for the later session registry is therefore:

- default standard scrollback: 2,000 rows per session;
- fallback initial viewport when no controller size is available: 120x40;
- accepted Foundation viewport ceiling: 240x80 per session;
- maximum live sessions per user: 8;
- checked fixed-cell admission ceiling: 128 MiB, with a 256 MiB terminal
  process-RSS target.

All five conditions must be enforced together before allocating or resizing a
model. The 128 MiB structural ceiling is only 1.4 MiB above the measured
126.6 MiB eight-session fixed-cell reservation; parser, snapshot, and transient
allocations are not charged to that structural limit. The saturated benchmark
process measured those exercised overheads at 154.7 MiB against the separate
256 MiB RSS target, leaving roughly 101 MiB of observed headroom. This is not a
guarantee for an unimplemented full daemon/session registry, which must enforce
the admission bounds and recheck whole-process RSS when it is built. The
current `resource_projection()` provides the checked per-model input.
Arbitrary 512x256/10,000 allocations are not supported by this recommendation.

`tests/foundation/resource-gate.sh` builds the benchmark once, exercises all
ten cases, checks the accepted saturated three/eight-session configurations
against the 128 MiB fixed-cell and measured 256 MiB limits, checks both
structural and observed-RSS rejection evidence, and rejects a saturated sample
unless its snapshot actually retains the configured history depth. It
explicitly removes its temporary measurement directory. No custom allocator
or monitor was introduced.

## Step 7 platform and aggregate result

The ordinary Rust CI matrix now uses the five current standard hosted labels:
`macos-latest` (arm64), `macos-15-intel`, `ubuntu-24.04`,
`ubuntu-24.04-arm`, and `windows-latest`. Every matrix entry runs the source
checkout policy immediately after checkout and before formatting or compiling,
then executes the same format, Clippy, workspace-test, documentation, and
side-effect-free CLI sequence. The harness-free PTY/drain tests execute their
real behavior on Unix and return explicit non-Unix skips on Windows. The
network, downloadable black-box, and resource Gates remain explicit commands
instead of being repeated on every push.

The workflow passed `actionlint 1.7.12`. The Foundation commit's hosted matrix
passed on both macOS architectures, both Linux architectures, and Windows
x86_64. The local macOS arm64 aggregate Gate also passed. Independently, a
read-only Linux arm64 Colima container ran source policy, version, format,
Clippy, all workspace tests (including real PTY/drain/resync), and docs, then
removed its exact container and target volume. The local Windows platform-only
MSVC cross-check described in Step 3 passed before the hosted Windows run.

For this historical Foundation result, `PHASE_NAME` reported
`phase-one-foundation-gate` and the CLI remained the same side-effect-free
build probe at workspace version 0.1.1; that Gate added no daemon process,
configuration, socket, pairing state, session registry, or UI. M3 later added
the per-user local daemon while retaining the session/network exclusions.

The final aggregate source, format, lint, workspace test, docs, dependency,
secret, shell, action-workflow, task-context, cross-target, and diff checks all
passed. No `deploy/relay/**`, Relay publication workflow, server, OpenResty,
Cloudflare, or firewall state changed in this Gate.

## Verification performed

```text
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
  main workspace and isolated Relay handshake-probe policies passed
sh tests/relay/secret-scan.sh
sh -n tests/foundation/network-gate.sh
dash -n tests/foundation/network-gate.sh
git diff --check
  all passed (cargo-deny emitted only allowed duplicate-version warnings)

cargo test -p zterm-daemon --test iroh_profile_gate
  4 passed; 1 ignored child probe (executed by the staging-environment test)

cargo test -p zterm-daemon --test iroh_network_gate --no-run
  compiled

sh tests/foundation/network-gate.sh
  A complete: relay -> relay, 3/3 streams
  B complete: raw UDP passed, relay -> direct, 3/3 streams
  C complete: non-DNS UDP blocked, relay -> relay over WSS/TCP, 3/3 streams
  raw run aggregate before verdict-only update: non-zero NO_GO_ADDRESS_DISCOVERY
  cleanup: test container, network, links and nftables state absent

cargo test -p zterm-core --test terminal_corpus
  5 passed

cargo test -p zterm-core --test terminal_snapshot_delta
  4 passed

cargo test -p zterm-core
  11 passed total: 2 unit + 9 integration

cargo clippy -p zterm-core --all-targets --all-features -- -D warnings
cargo doc -p zterm-core --no-deps
  passed

cargo test -p zterm-platform --all-features -- --nocapture
  6 unit tests passed
  interactive: input/output, child-observed 47x123 resize, natural exit 23
  bulk: 1,048,576 bytes, independent completion marker
  explicit close: signal Hangup
  PTY_LIFECYCLE_GATE=PASS

cargo clippy -p zterm-platform --all-targets --all-features -- -D warnings
cargo doc -p zterm-platform --no-deps
  passed

cargo test -p zterm-daemon --test terminal_drain -- --nocapture
  zero attachment: >1 MiB drained through capacity 2, independent marker
  simulated transport drop: root child continued and a new attachment synced
  concurrent wait: raw-mode DSR received CSI 0n before natural child exit
  TERMINAL_DRAIN_GATE=PASS

cargo test -p zterm-daemon --test attachment_resync -- --nocapture
  slow attachment: revision 1 -> latest across >190 processed chunks
  capacity 3; maximum pending 3; latest full resync was semantically equal
  ATTACHMENT_RESYNC_GATE=PASS

cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings
cargo doc -p zterm-daemon --no-deps
  passed

sh tests/foundation/terminal-blackbox.sh
  tmux 3.7c: alternate, resize, detached progress, latest resync passed
  Herdr 0.8.2: checksum, alternate, resize, detached progress, latest resync passed
  Codex CLI 0.148.0: isolated main-screen onboarding smoke passed; no prompt
  OpenCode 1.18.20: checksum and isolated alternate-screen smoke passed; no prompt
  TERMINAL_BLACKBOX_GATE=PASS; BLACKBOX_CLEANUP=PASS

cargo bench -p zterm-core --bench terminal_state
  stable 1/3/16 x 120x40/512x256 candidate matrix emitted six records

sh tests/foundation/resource-gate.sh
  ten shallow/saturated cases measured with /usr/bin/time
  saturated cases retained exactly 10,000 or 2,000 configured history rows
  recommended 3/8-session configurations passed structural and RSS limits
  oversized 10k/512x256 and 16-session candidates were rejected
  TERMINAL_RESOURCE_GATE=PASS; RESOURCE_TEMP_CLEANUP=PASS

cargo check -p zterm-platform --all-targets --all-features --target x86_64-pc-windows-msvc
  passed; portable-pty/ConPTY-facing zterm boundary compiled

Linux arm64 read-only Colima CI-equivalent run
  source/version/fmt/Clippy/workspace tests/doc passed
  PTY_LIFECYCLE_GATE, TERMINAL_DRAIN_GATE, ATTACHMENT_RESYNC_GATE passed
  LINUX_ARM64_CI_GATE=PASS; LINUX_CI_CLEANUP=PASS

actionlint 1.7.12 .github/workflows/ci.yml
  passed
```

## Retained boundaries after the Gate

| Area | Status |
| --- | --- |
| PTY lifecycle | Passed locally on macOS arm64, in Linux arm64, and across the hosted Unix matrix; Windows boundary compiled and skipped Unix behavior explicitly |
| Zero-attachment drain and detach/reattach ownership | Passed locally on macOS arm64, in Linux arm64, and across the hosted Unix matrix |
| VT parser/corpus and compatibility gaps | Step 2 passed with the active/inactive-screen boundary documented above |
| Snapshot/checkpoint/delta protocol | Step 2 passed for latest active-screen semantics |
| Resource measurements and recommended defaults | Step 6 passed; 2,000 rows, 240x80 ceiling, eight sessions, 128 MiB fixed-cell admission ceiling recommended |
| tmux/Herdr/Codex/OpenCode black-box checks | Step 5 passed; Codex isolated onboarding truthfully recorded as main screen |
| Real two-network automatic discovery | deferred to parent M10; not treated as passed |
| M2 entry decision | allowed; Foundation has no remaining hard stop |
