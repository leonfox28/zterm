# Target-Daemon Connection Unification Research

## Question

Can local and remote terminal connections be modeled as the same operation—connect to a target
daemon—while retaining the necessary transport differences?

## Conclusion

Yes. This is already the intended core architecture and is substantially implemented below the
command layer. Target-daemon unification does not require collapsing Unix IPC and Iroh into one
physical transport. Under the current semantic-broker baseline, the smallest safe change is to
carry the existing target-daemon abstraction through the operations/prepared-view/UI boundary,
where a remote-only presentation shortcut currently leaks. Whether the remote Session client
should remain in that broker is a separate open decision analyzed in
`remote-route-ownership-options.md`.

The stable invariant is:

```text
viewer resolves one target daemon
  -> local same-UID route OR viewer-daemon/Iroh route
  -> target ingress authenticates the caller
  -> one target SessionService / SessionActor / attachment contract
  -> one terminal UI and chrome composition at the viewer
```

The route is not target-Session state. The target daemon may retain a same-UID or authenticated
device principal for authorization, revoke cleanup, and remote-resume identity, but principal kind
must not select terminal rendering, resize, synchronization, acknowledgement, or controller
priority behavior.

## Existing Architecture Evidence

- `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/design.md:20-44` makes every desktop CLI
  connect to the local daemon/broker. The broker routes a self target to the local SessionRegistry
  and a remote target to the shared Iroh connection pool. It explicitly forbids CLI-owned Iroh
  endpoints and self-dial.
- The same design at `:341-359` says both route adapters call one SessionService contract; only the
  remote route owns `DesiredAttachment`, reconnect, and direct/relay path observation.
- `.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:170-178` requires one local IPC
  entry, one internal Session service/model/revision/lease, and distinct same-UID versus remote
  authenticated adapters.
- `.trellis/spec/backend/session-service.md:3-8` defines SessionService as the single
  transport-independent owner used by both adapters.
- `crates/daemon/src/device_directory.rs:57-93` already represents an exact target as
  `ResolvedSessionTarget::{Local, Device(DeviceId)}` while deliberately keeping aliases out of the
  routing token so a rename cannot retarget an operation.
- `crates/daemon/src/local_ipc.rs:648-770` encodes the same TerminalAttachRequest for either exact
  target. `:2033-2078` similarly encodes one Session unary message, then chooses only the local or
  remote forwarding adapter.
- `crates/daemon/src/local_ipc.rs:382-421` is the intended adapter split: a remote target enters
  `RemoteSessionService::bridge_attachment`; a self target enters `SessionWireServer` over the same
  SessionService contract.

## Boundary Leak

- `crates/daemon/src/operations.rs:392-475` exposes `remote_alias: Option<String>` in
  `PreparedTerminalView`. Alias presence then selects the initial transport state and is handed to
  the terminal driver as a proxy for route class.
- `crates/cli/src/terminal_ui.rs:239-317` predicts route class from the raw selector before daemon
  resolution, gives local and remote different geometry, and verifies route again by checking
  whether an alias exists.
- `crates/cli/src/terminal_ui.rs:2309-2376` uses `device: Option<String>` to decide status-row
  existence, remote class, display text, and whether connection samples are meaningful.
- `crates/cli/src/terminal_ui/composition.rs:21-38` takes a `remote: bool` solely to decide whether
  host chrome owns the bottom row. Layout and status rendering therefore have two related but
  separate enablement decisions.

This was sufficient for the prior explicit remote-only status-bar requirement, but it cannot express
the new product invariant: both routes have a target display name and status row, while only the
remote route has network path samples and reconnect.

## Recommended Boundary

Keep `ResolvedSessionTarget` as the private exact routing token. Replace the presentation shortcut
with immutable view metadata:

```rust
enum TerminalViewRoute {
    Local,
    Remote,
}

struct TerminalViewTarget {
    display_name: String,       // redacted in Debug; frozen for this view
    route: TerminalViewRoute,
}
```

`PreparedTerminalView` carries `TerminalViewTarget` plus its private `LocalAttachmentClient`.
Only the viewer-side connection-status projection matches the explicit route; the common
snapshot/delta/resize driver neither receives nor infers it from the display name. Remote
connection events carry only viewer-broker path/RTT observations and are rejected for a local
route.

Chrome layout independently owns the universal rule “reserve the bottom row when physical rows are
at least two.” Status text is route-specific data inside that common row:

- local: `<configured device name> | local`
- remote: `<frozen alias> | <direct|relay|--> | <integer ms|-->`

## Rejected Alternatives

1. **Set `remote_alias = Some(local_name)`.** This would classify local views as remote, alter their
   initial synchronization lifecycle, admit remote connection events, and couple identity to route.
2. **Always pass `remote = true` only in the UI.** This adds the row but preserves raw-selector and
   optional-alias inference, so future clients and lifecycle consumers still see two concepts.
3. **Emit a fake local `TerminalConnectionStatusEvent`.** The event represents Iroh selected-path
   telemetry. Fabricating one for Unix IPC corrupts its semantics and needlessly changes wire data.
4. **Make every CLI dial with the daemon's existing device key.** Iroh derives EndpointId from that
   key, and its `same_endpoint_id_relay` test demonstrates that the later live endpoint displaces
   the earlier endpoint's relay reachability. This also defeats the shared per-device connection
   pool and expands ordinary key handling. Direct CLI is coherent only with distinct identities,
   most plausibly a new root-signed delegated-capability protocol; that is a blocking product and
   security decision rather than a drop-in simplification.
5. **Self-dial the local daemon through Iroh.** This adds pairing, discovery, latency, and network
   failure to a same-UID operation without adding product value.
6. **Force local IPC and Iroh to share authentication/retry/reconnect code.** Their physical and
   security contracts are intentionally different. Unification belongs above the route adapters.

## Relationship to the Snapshot Bug

The target daemon does not have a local Herdr path and a remote Herdr path. The reported failure is
in the viewer's common delta/resize/snapshot acknowledgement decision. Current alias/route leakage
and different transport timing can make one route expose it first, but that difference is not an
allowed behavior. The universal status row reduces local child height by one, while Main/Alternate
still changes width due to the main-screen gutter; both routes must pass the same generic transition
contract, with route data absent from the acknowledgement decision.
