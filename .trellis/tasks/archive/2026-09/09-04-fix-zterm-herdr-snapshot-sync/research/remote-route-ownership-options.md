# Remote Route Ownership Options

## 1. Decision Being Made

The product invariant is already clear: a terminal view attaches to one target daemon, and the
target daemon owns the only Session/PTY/VT model/revision/controller state. The unresolved question
is narrower:

> On a desktop controller, which process owns the Iroh Endpoint and the client side of the remote
> Session protocol?

These are two separate ownership decisions:

1. **Network ownership** — device key, Iroh Endpoint, discovery, direct/relay selection,
   authenticated connection, stream admission, and path observation.
2. **Session-client ownership** — attach request, attachment ID, resume view ID, known revision,
   reconnect policy, synchronization fence, and operation retry/ambiguity handling.

The original comparison, “CLI direct” versus “CLI through its daemon,” hides this distinction. Four
coherent designs need to be considered.

### Historical requirement correction

The user's original explicit requirement was that multiple persistent Sessions shown later as
GUI/Android tabs or cards share one device-to-device connection rather than opening one connection
per Session. The prior plan proposed terminal-emulator tabs/windows as a first-version way to view
different Sessions and then derived cross-process daemon pooling from that proposal. The user has
now explicitly withdrawn the tentative statement that A would not run multiple CLIs. The
architecture must therefore tolerate concurrent frontend processes without creating a second A-B
network connection.

Three cardinalities must remain distinct:

- **Daemon:** device B has one authoritative target daemon.
- **Connection:** device A normally has one authenticated transport connection to B; device C has
  its own independent connection to B.
- **Attachment:** each visible Session view is an independent stream/attachment. A and C attaching
  the same SessionId observe the exact same SessionActor/PTY/TerminalModel/Agent process, not
  synchronized copies. They still have separate attachments, and controller-lease policy decides
  which one may input/resize.

One network connection does not imply one Session or one attachment. Several CLI processes and a
future single GUI/App with several tabs are simply different owners of attachment streams over that
same connection; neither may create an additional device-pair connection.

## 2. Non-negotiable Target-side Invariants

Every viable option must preserve all of the following:

- The target daemon owns exactly one SessionService/SessionActor/PTY/TerminalModel for a Session.
- Local and remote ingress authenticate differently, but invoke the same Session contract.
- `AttachmentPrincipal` may enforce authorization, revocation, resume ownership, and controller
  ownership; it cannot select different rendering, resize, snapshot, delta, or acknowledgement
  behavior.
- A connection or attachment disappearing never terminates the target Session.
- The viewer's route and path telemetry do not become target Session state.
- The Herdr failure is fixed in the common viewer acknowledgement transition. No routing option
  makes that client bug cease to exist by itself.

## 3. Decisive Constraint: One Device Key Cannot Be Used by Many Live Endpoints

The project pins Iroh 1.0.3. In that version:

- `Endpoint::builder(...).secret_key(key)` documents that the key's public key is the EndpointId.
- The upstream test `same_endpoint_id_relay` creates two live endpoints from the same key. Once the
  second endpoint connects to the home relay, new traffic is routed to the second endpoint and the
  first receives the warning: `Another endpoint connected with the same endpoint id. No more
  messages will be received`.

This is not merely a code-style preference. If the desktop daemon and one or more CLI processes all
load the current device key and create their own endpoints, a later CLI can take over relay
reachability for the shared EndpointId. It can break the daemon's inbound availability, and
concurrent CLIs can displace one another. The current one-primary-connection-per-device arbitration
also assumes one logical endpoint owner, not several processes racing under the same identity.

Restricting A to one outbound CLI does not remove the collision: A's daemon must normally keep its
own Endpoint alive so other devices can connect to Sessions hosted by A. A direct CLI using that
same key would still be the second live Endpoint. Avoiding this would require forbidding simultaneous
host/controller roles or redesigning transport identity, neither of which follows from a one-CLI UX.

Therefore the naive form of CLI-direct—copy the daemon's key into every CLI and dial—is not a viable
multi-process architecture with the current identity model.

Giving every CLI a different permanent identity avoids the EndpointId collision, but changes the
product identity from “this device” to “this terminal process.” It either requires pairing every
CLI instance or introduces a parent/delegated identity system. The latter is Option C below.

The current `0600` key file is protected at the OS-user boundary, not from arbitrary malicious code
already running as the same user. Keeping the key out of normal CLI code is still useful: it reduces
accidental reads, copies, crash/log exposure, and the number of components that must correctly
handle it. It must not be described as a hard same-UID security sandbox.

## 4. Option A — Daemon-owned Network and Semantic Attachment Broker

Current topology:

```text
CLI
  -> same-UID local Session wire
  -> viewer daemon RemoteAttachmentBridge
  -> shared authenticated Iroh connection / service stream
  -> target daemon SessionWireServer
  -> the target SessionService
```

### What the viewer daemon owns

- The long-term device key and sole Iroh Endpoint.
- Target resolution, route lookup, NAT traversal, relay fallback, pairing, authorization, and
  one shared connection per remote device.
- One `DesiredAttachment` per local view.
- A stable local attachment ID and epoch-local target attachment IDs.
- Resume view ID, frozen SessionId, last known revision, latest viewport, reconnect phase, pending
  controls/history requests, and path/RTT projection.
- Decoding, validating, rewriting, and re-encoding terminal protocol frames across the two streams.

The current implementation is consequently a semantic proxy, not a transparent pipe. This is
visible in `remote_attachment.rs`: `BridgeState`, `DesiredViewPhase`, `RemoteEpoch`,
`remote_attach_request`, attachment-ID rewriting, offline control handling, and reconnect/resume
logic are all daemon-owned.

### Advantages

- **Identity is coherent.** Exactly one process owns the device key and EndpointId. Pairing and
  revocation remain device-scoped.
- **Connections are shared.** If multiple CLI views are allowed, and in any case for future desktop
  GUI tabs, unary operations, and multiple Session attachments, independent QUIC streams can
  multiplex over one authenticated connection.
- **Warm startup can be faster.** A second view can reuse an already-resolved, authenticated,
  direct-or-relay connection instead of binding an endpoint and dialing again.
- **Resource use is bounded centrally.** UDP sockets, relay sessions, dial workers, stream limits,
  backoff, and admission budgets are per device/peer rather than per CLI.
- **Remote recovery is process-independent.** A temporarily disconnected CLI view can retain a
  daemon-owned desired attachment while the broker redials and resumes it.
- **Frontends stay thin.** CLI and a future desktop GUI reuse one implementation of pairing,
  discovery, connection pooling, remote mutation retries, and reconnect.
- **The target has a simple trust model.** TLS EndpointId directly names the authorized controller
  device; no subordinate credential needs to be parsed before normal service admission.

### Costs and risks

- **There are two application-level attachment epochs.** The CLI-to-viewer-daemon stream and the
  viewer-daemon-to-target stream have different attachment IDs and lifetimes.
- **The bridge duplicates Session-client state.** It must interpret revisions, snapshots, deltas,
  controls, resume, and errors. Correctness depends on keeping its state machine aligned with both
  the CLI and target contracts.
- **Reconnect state is split across layers.** The daemon owns remote reconnect and resume, while
  the CLI owns rendering, input gating, and snapshot acknowledgement. Route events and terminal
  events must be normalized correctly at the boundary.
- **The hot path has another hop.** Each input/update traverses a Unix socket plus an Iroh stream,
  with extra scheduling, framing validation, and in several paths decode/re-encode or ID rewrite.
  Its actual latency and throughput cost have not been measured; it should neither be assumed
  material nor dismissed as negligible.
- **The viewer daemon is a failure concentrator.** Its crash or restart drops every remote view on
  that desktop at once. A single CLI crash normally affects only its own view.
- **Version and error translation have more surface.** The bridge must preserve request
  correlation, typed errors, deadlines, and unknown/ambiguous mutation outcomes across two edges.
- **The semantic bridge is desktop-specific plumbing.** Mobile has no companion daemon and must
  own its remote Session client in the app process, so the core remote-client logic is not naturally
  shared unless separately extracted.

### Main architectural question

Option A is justified if the product wants the daemon to provide a durable, shared **remote-view
service**, not merely shared networking. It is overpowered if the desired invariant is only “one
device Endpoint and one connection pool.”

## 5. Option B — Every CLI Directly Uses the Existing Device Key

Naive topology:

```text
CLI with the desktop device key and its own Iroh Endpoint
  -> target daemon
  -> target SessionService
```

### Apparent advantages

- The Session protocol is end-to-end between the renderer and target daemon.
- There is no Unix-socket data hop, attachment-ID translation, or semantic proxy.
- The CLI directly observes connection closure, path, and RTT.
- A local daemon crash does not end an already-running remote CLI.
- Different CLI processes have isolated failure domains.

### Blocking problems

- **EndpointId collision makes the basic design invalid.** Live endpoints using the same key can
  displace one another at the relay. A CLI can also displace the daemon that must accept inbound
  connections for Sessions hosted on the same machine.
- **It violates the existing connection invariant.** Multiple CLI processes create multiple
  connections under one logical device identity instead of multiplexing one connection.
- **Network machinery is repeated per process.** Every CLI needs endpoint bind, discovery, route
  cache access, NAT traversal, relay lifecycle, authorization handshake, reconnect, limits, and
  path observation.
- **Persistent state gets a new concurrency problem.** If the CLI stops asking the daemon to
  resolve devices, the device directory and route cache need multi-process readers/writers and
  locking. If it still asks the daemon, it is not independent; only its data plane is direct.
- **Resource cost scales with terminal count.** Each CLI can add an endpoint, UDP socket, relay
  connection, handshake, peer connection, and its own backoff loop.
- **Pairing and revoke semantics become ambiguous.** The target sees several simultaneous
  connections claiming the same identity, while current admission and primary-generation logic is
  device-scoped.

### Conclusion

Option B should be rejected, not selected as the “simple” architecture. Making it work requires
changing identity, which turns it into Option C.

## 6. Option C — Direct CLI with a Delegated Ephemeral Identity

Possible topology:

```text
CLI generates ephemeral Iroh key
  -> same-UID request to local daemon
  <- target metadata + root-signed, short-lived capability bound to that ephemeral EndpointId
CLI ephemeral Endpoint
  -> target daemon validates delegated capability
  -> target SessionService under the parent device principal
```

The local daemon remains the device root/control-plane authority, but leaves the data path after
credential issuance.

### Advantages

- **The terminal data plane is genuinely direct.** The renderer speaks the Session protocol to the
  target and uses the target attachment ID without translation.
- **The long-term key stays in the daemon.** Each CLI gets only an ephemeral private key and a
  narrowly-scoped authorization.
- **Failure isolation improves.** A viewer-daemon restart need not kill an established remote CLI;
  one CLI connection failing does not disturb another.
- **Capabilities can be least-privilege.** They can theoretically bind target, delegate key,
  service purpose, parent device, authorization generation, and allowed duration.
- **Desktop and mobile remote clients can share more Session-client logic.** Both become endpoint
  owners from the Session protocol's perspective.

### Costs and unresolved security design

- **This adds a new authentication protocol.** The target's TLS peer is no longer the already
  paired device. Before accepting normal services, it must validate a parent-signed delegation and
  map it back to the authorized parent principal.
- The credential must define and test:
  - signature algorithm/domain separation and canonical encoding;
  - binding to delegate EndpointId, exact target, service/purpose, protocol version, and issuer;
  - issue time, expiry, clock-skew policy, connection-lifetime behavior, and renewal;
  - replay policy and whether a stolen public capability is harmless because it is key-bound;
  - authorization generation and immediate revocation of every live delegate;
  - admission limits before and after credential validation to resist connection/handshake DoS;
  - audit/debug redaction and migration/version compatibility.
- **Connections are no longer shared.** Each CLI has a distinct EndpointId and connection. This is
  correct but gives up the original one-connection-per-device-pair optimization.
- **Startup and network cost scale per CLI.** Every process binds an endpoint and may maintain its
  own home-relay connection, NAT probes, direct-path establishment, and target QUIC connection.
- **The daemon is still needed at launch.** Unless device directory and credential issuance move
  elsewhere, `zterm connect` must start/contact it before dialing. The benefit is data-plane
  independence after launch, not a daemon-free desktop.
- **Principal semantics become hierarchical.** Controller ownership, resume checkpoints, revoke,
  logs, and limits must decide whether they bind to the ephemeral delegate, the parent device, or
  both.
- **It is a broad migration.** Pairing, transport admission, protocol negotiation, stores, tests,
  and all clients change; this is much larger than removing a forwarding function.

### When it is justified

Option C is attractive only if direct process-to-process transport and per-CLI failure isolation
are product requirements strong enough to justify a new delegated-identity system and per-view
network resources.

Because concurrent CLI processes are not forbidden and every device pair must have only one active
connection, Option C no longer satisfies the confirmed product model: each direct CLI has a
different Endpoint and connection. Adding a shared process that opens streams for the CLIs turns it
into Option D. C remains useful only as a record of the trade-off if the single-connection invariant
is reconsidered later.

## 7. Option D — Daemon-owned Network, Opaque Stream Tunnel, CLI-owned Session Client

Middle topology:

```text
CLI SessionClient
  -> same-UID OpenAuthenticatedStream(target, purpose) tunnel
  -> viewer daemon's one Endpoint / shared Iroh connection
  -> opaque QUIC service stream
  -> target daemon SessionWireServer
  -> target SessionService
```

The daemon still owns the device identity and shared network connection, but it no longer owns a
`DesiredAttachment` or interprets terminal revisions. After validating the local caller and exact
target, it opens an authenticated service stream and pumps bounded bytes. Route/path observations
travel as tunnel metadata or on a separate local side channel, not as target Session state.

### Advantages

- **It preserves the strongest reasons for the daemon broker.** One key, one Endpoint, one paired
  device identity, shared connection, central discovery, direct/relay selection, and resource
  admission all remain.
- **The Session protocol becomes logically end-to-end.** The CLI uses the target attachment ID and
  owns attach/resume/revision decisions; the viewer daemon does not rewrite terminal frames.
- **Boundary complexity is reduced.** Stable-local-versus-epoch-remote attachment ID mapping,
  revision caches, pending terminal controls, and target error translation can leave the daemon.
- **It does not require delegated authentication.** The target still sees the paired viewer-daemon
  EndpointId and existing authorization generation.
- **Multiple CLIs still share transport.** Each gets an independent QUIC stream on one connection.
- **Protocol compatibility is easier to reason about.** The tunnel protocol answers only “open and
  observe this authenticated stream”; Session semantics remain between SessionClient and target.
- **It gives mobile and desktop a plausible shared SessionClient.** Desktop supplies a tunneled
  byte stream; mobile supplies a directly opened Iroh stream.

### Costs and hidden complexity

- **The extra local hop remains.** If the daemon owns the userspace QUIC connection, it cannot hand
  a live Iroh/QUIC stream to another process as a normal OS file descriptor. QUIC crypto,
  congestion, and stream state live in the daemon. A byte proxy or shared-memory transport remains.
- **Reconnect does not disappear; it moves.** When the remote service stream is lost, the CLI must
  retain ResumeViewId/known revision, request a new tunnel, send a new attach, consume the target's
  new attachment ID, gate input, and resynchronize.
- **A tunnel control protocol is required.** It needs exact target/purpose selection, open failure,
  half-close semantics, byte/frame limits, backpressure, cancellation, daemon shutdown, path/RTT
  side events, and protection against a same-UID client opening arbitrary ALPNs or unbounded
  streams.
- **The viewer daemon is still a shared failure domain.** Its restart closes every tunnel. A CLI can
  restart/autospawn it and resume, but it cannot keep the old network connection alive.
- **Remote Session-client logic must become reusable.** Moving the current state machine into ad
  hoc UI code would only relocate complexity. It should be a transport-agnostic SessionClient used
  by CLI, desktop GUI, and mobile.
- **Mutation ambiguity needs a single owner.** The current daemon owns byte-identical retries and
  outcome-unknown classification for remote unary mutations. A fully generic tunnel moves that
  policy to SessionClient. Keeping unary forwarding daemon-owned while only attachments use the
  tunnel is a possible migration step, but temporarily leaves two ownership models.
- **This is not a small refactor.** Much of the well-tested `RemoteAttachmentBridge` behavior would
  be replaced. The benefit must be proven by a prototype and fault tests, not inferred from fewer
  boxes in a diagram.

### Main architectural question

Option D is the cleanest candidate if the desired boundary is:

> daemon owns authenticated connectivity; the active frontend owns Session-client semantics.

It is not superior if the product intentionally wants remote views to outlive or be managed
independently of a particular frontend process through a daemon-owned desired-view abstraction.

## 8. Comparative Matrix

| Dimension | A. Semantic broker | B. Shared-key direct | C. Delegated direct | D. Opaque tunnel |
| --- | --- | --- | --- | --- |
| Current identity model | Native fit | **Invalid for concurrent live endpoints** | Requires delegation protocol | Native fit |
| One connection per device pair | Yes | No | No, normally one per CLI | Yes |
| Multiple CLI resource efficiency | Best | Poor/conflicting | Lowest of viable choices | Best |
| Session protocol end-to-end | No; semantic proxy | Yes | Yes | Yes, through byte tunnel |
| Attachment ID/revision rewrite | Yes | No | No | No |
| New target auth surface | None | Breaks current assumptions | **Large** | None |
| Local data hop | IPC + semantic bridge | None | None after issuance | IPC byte proxy |
| Warm second-view startup | Reuses connection | New/conflicting endpoint | New endpoint/connection | Reuses connection |
| Viewer-daemon crash impact | All remote views drop | Running CLI unaffected | Running CLI unaffected | All tunnels drop, CLI may resume |
| Per-CLI crash impact | Its desired view closes | Its connection closes | Its connection closes | Its tunnel closes |
| Remote reconnect owner | Viewer daemon | CLI | CLI | CLI SessionClient + daemon network pool |
| Long-term key in CLI path | No | Yes | No | No |
| Implementation risk from current code | Lowest | Not acceptable | Highest/security-sensitive | Medium-high/rearchitecture |
| Mobile/client reuse | Needs extraction/parallel client | Direct model, identity wrong | Strong | Strong with shared SessionClient |

## 9. Failure-domain Comparison

### Network changes and relay migration

- A and D keep one daemon-owned connection actor; all views benefit from one path migration and
  central backoff. Independent streams still isolate Session-level slowness, although they share
  connection congestion control.
- C lets each CLI recover independently, but several CLIs can simultaneously rediscover, reconnect,
  and use relay capacity.
- B can actively damage reachability because identical EndpointIds race at the relay.

### Viewer daemon restart

- A loses broker-owned attachment state unless it was explicitly made persistent; each target
  Session survives and views must resume/reopen after daemon recovery.
- D loses tunnels, but the CLI is already the desired-view owner and can re-establish them after
  daemon autospawn. This is conceptually cleaner, though it still needs implementation.
- C's established data plane survives the root daemon restart, subject to capability lifetime and
  revoke semantics.

### CLI exit or terminal hang

- In all designs, the target Session must survive.
- A must promptly translate local EOF/cancel into remote detach and release its demand.
- C and D naturally bind the remote/tunneled stream lifetime to the frontend, but still need
  bounded detach and controller-lease cleanup.

### Target daemon restart

- No option preserves PTY state because the current product does not promise Session survival
  across target-daemon restart.
- Reconnect code must distinguish target restart/session disappearance from a transient route loss;
  changing route ownership does not change that product boundary.

## 10. Performance Questions That Must Be Measured

No current benchmark establishes that the local IPC/semantic bridge is either expensive or free.
The relevant comparisons are:

- first remote view from a cold endpoint;
- second and tenth views with a warm shared connection;
- keystroke-to-target and target-delta-to-render latency on LAN direct, WAN direct, and relay;
- sustained full-screen TUI updates and large snapshot throughput;
- CPU copies, allocations, wakeups, and RSS per view;
- behavior under one slow view plus several active views;
- viewer-daemon restart and network-loss time to a usable resumed frame.

Option C should not be called faster overall merely because its steady-state data path is shorter;
per-process endpoint startup and connection establishment may dominate. Option A should not be
called effectively free merely because Unix IPC is local; the current semantic decode/rewrite and
task scheduling are real work.

## 11. Selected Decision and Consequences

The user selected **D: daemon-owned network plus an opaque per-frontend stream tunnel**.

The selection fixes both ownership axes explicitly:

1. the viewer daemon remains the one shared owner of device identity, Endpoint, discovery,
   authentication, direct/relay migration, the device-pair connection, stream admission, and path
   observation;
2. each active CLI/GUI process owns its own transport-independent SessionClient, target attachment
   identity, ResumeViewId, known revision, viewport, reconnect/resume sequence, and mutation
   ambiguity;
3. each frontend has its own same-UID IPC connection; a remote IPC tunnel maps 1:1 to one service
   stream on the shared device-pair connection, rather than several frontends sharing one IPC byte
   stream;
4. the viewer daemon interprets only a bounded local tunnel envelope and path side channel. Inner
   Session frames are opaque and are never decoded, rewritten, acknowledged, or retried there.

B remains rejected because duplicate live Endpoints with one long-term EndpointId conflict. C
remains rejected under the confirmed concurrent-frontend plus one-connection-per-device-pair
invariant. A remains useful as historical/current-code comparison but is not a production fallback
to retain alongside D.

## 12. Final Assessment

- The daemon network broker is necessary under the chosen identity, multi-frontend, and shared-
  connection model; the semantic attachment broker is not.
- D retains the network-level benefits of the current design while giving Local and Remote one
  frontend Session-client implementation. The design must still prove bounded tunnel behavior,
  reconnect/resume correctness, mutation ambiguity, daemon-restart recovery, and sibling-stream
  isolation before the old bridge is removed.
- D does **not** fix Herdr by implication. Herdr is a separate event-entry acknowledgement bug in
  the common viewer delta/resize transition. Migration can preserve it on both routes or hide it by
  timing; only the application-neutral regression and explicit state-machine correction establish
  correctness.
- The implementation order is therefore: reproduce and repair the acknowledgement invariant,
  implement/prove the D tunnel and common SessionClient, remove the semantic bridge, then unify
  chrome and run both generic and Herdr end-to-end gates.
