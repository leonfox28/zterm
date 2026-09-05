# Technical Design: One Session Client over Local IPC or a Daemon-Owned Remote Tunnel

## 1. Selected Architecture

The selected model is **D: daemon-owned network, frontend-owned Session client**.

Every terminal view is a client attachment to exactly one target daemon. Local and Remote name the
route used to reach that daemon; they do not create two target-side terminal modes.

```text
Local CLI 1 -- own IPC connection 1 -----------------> local daemon SessionWireServer
Local CLI 2 -- own IPC connection 2 -----------------> local daemon SessionWireServer

Remote CLI 1 -- own IPC tunnel 1 -> viewer daemon -- QUIC service stream 1 --┐
Remote CLI 2 -- own IPC tunnel 2 -> viewer daemon -- QUIC service stream 2 --┼-> target daemon
                                      one shared Iroh peer connection         ┘
```

The important cardinalities are:

- one same-UID IPC connection per active CLI/GUI view, never one IPC byte stream shared by several
  frontend processes;
- one tunnel and one current QUIC service-stream epoch per remote view;
- at most one active authenticated Iroh connection per device pair, shared by all of those service
  streams;
- one target `SessionActor`/PTY/terminal model/Agent process for one `SessionId`, even when several
  devices or several frontend views attach to it;
- one independent target-issued attachment identity per view.

The viewer daemon owns the long-lived device key, Iroh `Endpoint`, discovery, direct/relay path,
authentication, connection arbitration, connection pool, stream admission, and path observation.
The active frontend process owns Session attach/resume state, target attachment IDs, applied
revision, viewport, synchronization, request correlation, operation leases, and mutation ambiguity.

The current daemon-owned semantic bridge (`DesiredAttachment`, local/remote attachment-ID mapping,
revision/viewport cache, queued controls, and attachment reconnect state) is an old intentional
architecture that D supersedes. It is not retained as a second implementation of Session-client
semantics.

## 2. Two Independent Root Causes

This task deliberately keeps two findings separate.

1. **Architecture boundary:** Local currently reaches `SessionWireServer` directly, while Remote is
   interpreted and rewritten by a daemon-owned semantic bridge. That was an intentional earlier
   design, not an accidental missing branch. D replaces it with an opaque remote stream tunnel so
   both routes terminate in the same frontend-owned Session client.
2. **Herdr failure:** the common terminal-view delta handler observes `Active`, renders a delta that
   changes Main/Alternate layout, sends a resize, mutates itself to `Synchronizing`, and then uses
   that mutated state to acknowledge the old delta. The target correctly rejects that stale
   acknowledgement with `not_synchronized: attachment is not awaiting a snapshot`.

Current Remote appears correct because the old semantic bridge is also an acknowledgement filter:
`remote_attachment.rs` forwards a local `TerminalSnapshotApplied` only while its own epoch is
`Synchronizing` and the revision equals its expected revision; otherwise it returns without sending
that command to the target. Local has no such intermediary, so the target sees and rejects the UI's
invalid command. Remote success today is therefore evidence of bridge masking, not evidence that the
shared UI transition is correct.

D removes both this semantic filter and a source of future Local/Remote drift, but it does not
logically repair the producer of the invalid command. Copying the current handler into the new
common client makes the bug reachable on Remote as well as Local. Once D and the explicit transition
fix are complete, the same event sequence must succeed on both routes; a Local-only or Remote-only
failure is an implementation defect, not an allowed architectural difference.

## 3. Component Ownership

### 3.1 Target daemon

The target daemon remains the only owner of:

- `SessionService`, `SessionActor`, PTY, terminal model, authoritative revision and lifecycle;
- controller lease/takeover policy and independent attachment records;
- strict replacement-snapshot acknowledgement state;
- authorization of the principal established by the ingress adapter.

`SessionWireServer` remains the one adapter around that contract. A same-UID local stream creates a
local principal; an authenticated Iroh stream creates a remote-device principal. Principal kind may
affect authorization, revoke cleanup, and remote-resume eligibility, but never rendering, resize,
delta interpretation, snapshot acknowledgement, or terminal contents.

### 3.2 Viewer daemon

The viewer daemon owns only shared network concerns for a remote view:

- resolve and authorize the exact `DeviceId`;
- acquire one demand on the existing `ConnectionBroker` peer slot;
- open one `StreamPurpose::Service` bidirectional stream on the promoted shared connection;
- pump bounded opaque Session bytes between one same-UID IPC tunnel and that service stream;
- project opened/closed and direct/relay/RTT observations through a tunnel side channel;
- release stream and demand permits on close.

It must not decode an inner `TerminalAttachRequest`, allocate or replace a Session attachment ID,
manufacture Session transport-state events, track an applied revision or viewport, retry a Session
mutation, or decide when to acknowledge a snapshot.

### 3.3 Frontend process

A transport-independent Session client runs inside each CLI/GUI process. The implementation may
remain in a reusable Rust library crate; “frontend-owned” describes runtime/process ownership, not
necessarily the Cargo package containing the type.

It owns the exact inner Session frames and the complete live-view state across stream epochs. The
terminal UI consumes one normalized event/command API produced by this client. Route observation is
a separate presentation input and is not passed into the snapshot/delta/resize state machine.

## 4. Transport Abstraction and Establishment

The frontend client receives a connected byte transport plus immutable endpoint metadata. The
transport boundary must support bounded asynchronous read/write, half-close, typed establishment
failure, and optional route observations. It must not expose the viewer daemon's Iroh key or
`Endpoint`.

### 4.1 Local

For Local, each frontend opens its own Unix socket to the local daemon and sends ordinary Session
wire frames directly. The inner target selector is `local=true`, as required by the same-UID
`SessionRequestContext`. The daemon passes the attachment stream directly to its local
`SessionWireServer`.

Local establishment never performs device discovery, pairing, self-dial, Iroh connection setup, or
relay access. A failure of Internet/DNS/Pkarr/relay therefore cannot prevent Local attach.

### 4.2 Remote

For Remote, each frontend opens its own Unix socket to its local viewer daemon and first opens one
remote Session tunnel for an already-resolved exact `DeviceId`. Once the tunnel is open, the
frontend sends the ordinary target Session wire stream inside bounded opaque data envelopes. The
inner target selector names the target daemon's own `DeviceId`, as already required by the remote
authenticated `SessionRequestContext`.

The viewer daemon maps that IPC tunnel 1:1 to a broker-owned QUIC service stream. It does not
rewrite `target`, request IDs, attachment IDs, revisions, unknown fields, or frame boundaries. The
target therefore sees the same Session protocol it already accepts on authenticated remote service
streams, and the frontend sees target responses unchanged after the tunnel envelope is removed.

Existing bounded single-request `LocalSessionUnaryRequest` forwarding may remain for stateless
list/create/rename/close operations during migration. It already forwards a validated inner Session
frame without attachment-ID translation. Live terminal streams must use the new tunnel boundary;
the old semantic attachment bridge is removed after parity gates pass.

## 5. Same-UID Tunnel Protocol

The tunnel is a versioned **local IPC control protocol**, not a new target Session protocol and not
a new network ALPN. New local-only wire kinds/messages represent:

- `Open`: exact target `DeviceId` plus tunnel protocol version;
- `Opened`: correlated confirmation after remote authorization and service-stream admission;
- `Data`: a bounded opaque byte chunk, with direction implied by the socket endpoint;
- `Path`: latest `unknown`/`direct`/`relay` plus optional bounded RTT;
- `HalfClose`: no more data will be written in that direction;
- `Closed`: one terminal reason/category for this tunnel epoch.

After `Open`, all frames on that Unix socket belong to one tunnel. There is no tunnel ID or
multiplexing inside the socket because the socket itself is the isolation boundary. Multiple CLI
processes never coordinate writes to one IPC stream.

Inner bytes may be split at arbitrary chunk boundaries. The frontend's ordinary Session
`FrameDecoder` reassembles target frames; the viewer daemon never runs that decoder on `Data`.
Tunnel chunks use a fixed upper bound well below the global frame ceiling, bounded channels, and
backpressure from the slower side. A complete inner snapshot may span many tunnel chunks without
being buffered as one daemon-owned semantic object.

Only one task owns each physical writer. Data, path, and close producers feed a bounded writer
queue so envelope bytes cannot interleave. EOF and half-close are propagated once; cancellation
closes the corresponding QUIC half, releases RAII permits, and reaps both pumps. A slow or malformed
tunnel consumes only its own connection/stream permits and cannot grow an unbounded queue.

Before `Opened`, failures are returned as a correlated typed local service error. After `Opened`, a
network or peer failure produces `Closed` when the local socket is still writable, then EOF. A
malformed outer envelope closes only that same-UID tunnel. Target-generated Session errors remain
opaque inner data and are interpreted only by the frontend Session client.

Path/RTT is sideband envelope data. It is never inserted into the inner Session byte stream and is
never sent to the target daemon. A new tunnel epoch begins with unknown path/RTT and cannot reuse a
stale sample from the previous connection candidate.

## 6. Frontend Session Client State

One Session client instance belongs to one active view. It retains across remote tunnel epochs:

- immutable exact target and safe frozen display metadata;
- one stable `ResumeViewId` generated by the frontend for that view;
- frozen `SessionId` after the first successful attach;
- the target-issued current `AttachmentId` for the active stream epoch;
- last atomically applied revision, latest requested viewport, takeover intent, and force-full flag;
- request IDs, operation lease/sequence, pending query/control correlation, and mutation ambiguity.

On the first attach it sends the caller's selector/create/takeover/viewport. On a retryable remote
tunnel loss it opens a new tunnel and sends a resume attach with the same `ResumeViewId`, frozen
`SessionId`, last applied known revision, and latest viewport according to the target wire contract.
The target may issue a new attachment ID for the new epoch; no viewer-daemon-local attachment ID
exists and no mapping is required.

Only a successfully applied snapshot/delta advances the client's known revision. A Session
mutation whose request may have been committed before transport loss follows the existing
operation-lease/replay rules; the tunnel daemon never guesses or retries it. Non-retryable target
errors are surfaced unchanged. A Local socket loss does not attempt network recovery; if the local
daemon died, its in-memory Session/PTY died with it.

The reconnect strategy may decide whether and how to reopen a transport, but the code interpreting
inner snapshot/delta/control frames is route-neutral. In particular it cannot branch on Local,
Remote, alias presence, path kind, RTT, or tunnel epoch when deciding acknowledgement correctness.

## 7. Multiple Views and Failure Isolation

Attaching A and C to the same `SessionId` on B reaches B's exact same `SessionActor`, PTY, terminal
model, and Agent process. Each view has its own attachment state, viewport observation, and
controller/takeover relationship under the existing target policy. “Same Session” does not mean
sharing an attachment ID or a byte stream.

Failure boundaries are:

| Failure | Required result |
| --- | --- |
| One frontend exits or closes IPC | only its attachment/tunnel/service stream closes |
| One tunnel envelope is malformed | only that tunnel closes; peer connection and sibling streams remain |
| One QUIC service stream resets | only its Session client reconnects/resumes |
| Shared peer connection is lost | all affected clients independently reopen tunnels; broker establishes one replacement connection |
| Viewer daemon restarts | remote frontends reconnect/autospawn it and independently resume; target Sessions remain |
| Target daemon/session ends | every attached view receives the authoritative target outcome |
| Local daemon exits | its local Sessions and local attachments end; no network fallback/self-dial |

The broker's existing per-peer open queue, per-connection stream admission, global handler limits,
and RAII accounting stay authoritative. Tunnel-specific limits are additive, not replacements.

## 8. View Metadata and Universal Chrome

Viewer-side metadata has two independent values:

- an exact resolved target used for connection establishment;
- a redaction-safe immutable presentation record with display name and route.

The Local name comes from the local daemon's committed device name. The Remote name is the safe
alias frozen when the exact target is resolved. Display-name presence or spelling never infers a
route and never changes Session synchronization.

Every physical terminal with at least two rows reserves the bottom row. Main and Alternate use the
same layout code; the existing Main-screen gutter remains. At physical 24x80 this yields Main
23x79 and Alternate 23x80 for both routes. A one-row terminal gives its sole row to the child.

The status strings are exactly:

```text
Local:  <device> | local
Remote: <device> | <direct|relay|--> | <integer ms|-->
```

Local never displays latency or a trailing third field. Both use the same reverse-video whole-row
rendering, Unicode display-cell clipping, atomic presentation, cursor/style restoration, and
exclusion from child/gutter/mouse/selection coordinates.

## 9. Herdr Acknowledgement Correction

At entry to a contiguous `TerminalViewEvent::Delta`, capture whether the view was already waiting
for a snapshot activation:

```rust
let acknowledges_existing_sync =
    transport_state == TerminalViewTransportState::Synchronizing;
```

Then keep the normal order:

1. validate and apply the contiguous delta;
2. present it atomically when live;
3. derive the Main/Alternate layout;
4. if an entry-`Active` delta changed geometry, send one resize and enter `Synchronizing`;
5. send `snapshot_applied(delta.to_revision)` only when
   `acknowledges_existing_sync` was true at event entry.

Thus an Active delta that itself starts a new resize/snapshot epoch cannot acknowledge that new
epoch with its old revision. A contiguous delta received while already `Synchronizing` still
acknowledges its exact `to_revision`. Pending mode geometry observed while synchronizing is sent only
after the current activation completes.

The target's strict `AttachmentSync::Awaiting` check remains unchanged. Duplicate/stale
acknowledgements must not be ignored, because that would conceal client state corruption.

## 10. Compatibility and Security

- CLI syntax and the reserved `local` selector do not change.
- The target Session wire major, target message meanings, persistent schema, Session IDs, operation
  leases, acknowledgement contract, and normal Iroh ALPN do not change.
- The new tunnel envelope is accepted only on an authorized same-UID local IPC connection.
- CLI/GUI processes never read the long-lived device private key or create another Endpoint with
  the same identity.
- Remote target resolution remains exact and alias-renaming cannot retarget an established view.
- Debug output redacts display strings and all tunnel payload; it may report only bounded lengths,
  route category, and non-secret lifecycle state.
- Local intentionally gains the universal status row, reducing its child height by one where the
  terminal has at least two rows.
- Removing the semantic bridge changes implementation ownership, not target authorization or the
  fact that multiple views attach independently to the same Session.

## 11. Validation and Rollout

Implementation is staged so the Herdr fix remains independently reviewable:

1. add an application-neutral Main/Alternate pseudo-TTY regression and fix the event-entry
   acknowledgement transition;
2. add and exhaustively test the local-only tunnel envelope and bounded pump;
3. extract the current frontend-side logic into one transport-independent Session client;
4. switch Remote to the daemon tunnel while retaining the old bridge only as a temporary test
   oracle/fallback inside the development branch;
5. prove parity and fault isolation, then delete the old bridge and fallback in the same change;
6. introduce explicit presentation metadata and universal local/remote chrome;
7. run focused, real-Iroh, pseudo-TTY, Herdr, workspace, and durable-spec gates.

Required named evidence includes:

- one generic DECSET/DECRST 1049 test that fails before the acknowledgement fix and continues after
  Main→Alternate→Main afterward;
- the same synthetic target attachment trace through Local direct IPC and Remote tunnel fixtures,
  producing identical Session events, revisions, acknowledgement commands, and viewports;
- two or more simultaneous remote tunnel streams sharing one broker peer connection;
- reset/backpressure/malformed-envelope tests proving sibling-stream isolation and permit cleanup;
- connection loss followed by one broker redial and independent resume from every live frontend;
- exact Local two-field and Remote three-field status rendering, including narrow/Unicode and
  one-row cases;
- real local-daemon `zterm` → Herdr 0.8.2 smoke with normal return to the Session shell;
- existing target Session, authorization/revoke, connection-broker, and two-daemon direct tests.

No public-relay availability is introduced as a deterministic CI prerequisite. Existing simulated
relay/path migration tests validate the affected side channel; a manual/public-relay smoke remains
environmental evidence.
