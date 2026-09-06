# Research: Color state architecture and end-to-end ownership

- Query: Design an implementable single-owner architecture for the confirmed C05–C18 color compatibility scope, including controller observations, child writes/queries/resets/stacks, semantic transport, attachment lifetime, rendering and pinned-engine limitations.
- Scope: mixed; repository and locally installed pinned dependencies are the implementation evidence. Primary protocol documents were consulted for state semantics.
- Date: 2026-09-06
- Status: planning research, not implementation or runtime conformance evidence. Only this research file was written.

## Findings

### Recommendation

Add **one `ZtermColorState` owned by `AlacrittyEngine` inside the Session's existing `TerminalModel`**. It owns base observations, application overrides, effective color resolution, bounded color stacks, appearance subscription and whole-screen reverse-video state. All supported child OSC palette/default-color actions remain intercepted by the existing ingress policy and are dispatched to this owner; they must never also mutate Alacritty's palette. Alacritty continues to own the sole grid, scrollback, cursor templates, per-cell SGR state and screen switching.

This is bounded metadata orthogonal to the grid, not another terminal parser or a replacement grid. It avoids having one app palette in Alacritty plus a competing palette in the daemon or renderer. Core/protobuf values expose snapshots of effective state; they do not become authorities. The frontend's outer observations are proposals authorized by the Session's current controller lease.

Known effective colors resolve to RGB only at presentation. Unknown base colors retain inherited default/indexed rendering and never produce fabricated RGB replies. Application RGB cell colors remain literal. State-only changes advance the same checked terminal revision and reach visible and retained-history rendering even if every cell is unchanged.

### Files found and existing integration points

| File | Existing owner / concrete change point |
| --- | --- |
| `crates/core/src/terminal.rs:137` | `TerminalColor::{Default, Indexed(u8), Rgb(u8,u8,u8)}`; retain these per-cell meanings. |
| `crates/core/src/terminal.rs:149` | `TerminalStyle`; replace the underline Boolean with a canonical style enum and add an optional underline color. |
| `crates/core/src/terminal.rs:297` | `TerminalModes`; add screen reverse-video presentation state if it is exposed here rather than the color snapshot. Do not overload per-cell inverse. |
| `crates/core/src/terminal.rs:337` | `TerminalSurface`; add a complete effective color snapshot. |
| `crates/core/src/terminal.rs:406` | `TerminalSurfaceDelta`; carry a complete small color snapshot, even with zero row patches. Extend transactional `candidate`/`apply_to` validation. |
| `crates/core/src/terminal.rs:824` | `TerminalSurfaceHistoryWindowFrame`; add the same revision-bound color snapshot for self-contained history responses. |
| `crates/terminal/src/engine.rs:167` | `AlacrittyEngine`; sole new color-state owner and typed color action methods. |
| `crates/terminal/src/engine.rs:109` | `BoundedEventSink` currently rejects `PtyWrite`, `ColorRequest` and callback closures. Keep upstream callbacks blocked; build bounded canonical replies locally. |
| `crates/terminal/src/engine.rs:211` | `feed_raw`, `feed_screen_transition`, `feed_reset`; distinguish RIS/reset behavior from ordinary main/alternate switching. |
| `crates/terminal/src/ingress.rs:177` | `process` visits bytes in order and drains upstream side events after each byte. This is the existing control-order boundary. |
| `crates/terminal/src/ingress.rs:400` | `dispatch_csi`; add exact inquiry, mode, stack and reverse-video actions before generic forwarding. |
| `crates/terminal/src/ingress.rs:487` | Whole-SGR underline-color filtering is the C13 defect. Remove that rejection when full supported SGR 58/59 reaches the pinned engine. |
| `crates/terminal/src/ingress.rs:499` | `dispatch_string` currently takes only a collector; give it the engine and a fallible reply result. Preserve completed-string framing, cancellation and caps. |
| `crates/terminal/src/ingress.rs:109` | `UpdateCollector` owns one 64 KiB reply buffer; every new reply and asynchronous appearance notification must use this bound. |
| `crates/terminal/src/model.rs:121` | `TerminalModel::new`, `ingest`, `resize`; add typed base-color update with the same preflight/revision contract. |
| `crates/terminal/src/model.rs:239` | `update_from_projection`; include color-only state in delta/capture without inventing row patches. |
| `crates/terminal/src/model.rs:290` | `history_window`; project colors under the same model lock/revision as semantic rows. |
| `crates/terminal/src/projection.rs:16` | `CHECKPOINT_FORMAT_VERSION = 2`; increment for the new projection shape. |
| `crates/terminal/src/projection.rs:117` | Screen/cursor/mode projection; add effective colors and preserve underline detail from upstream cells. |
| `crates/daemon/src/terminal_driver.rs:330` | Model-owner loop currently ingests, releases model lock, writes replies, then publishes processed revision. Extend transaction ordering rather than add an independent reply writer. |
| `crates/daemon/src/terminal_driver.rs:471` | `resize`; use the same ordered commit mechanism as ingest and base-color mutation. |
| `crates/daemon/src/terminal_driver.rs:797` | `SharedTerminal::ingest` / `publish_revision`; one publication order must cover every mutation source. |
| `crates/daemon/src/session.rs:603` | `prepare_attach` service API and internal request: accept controller color observations with initial viewport. |
| `crates/daemon/src/session.rs:1082` | Session creates a model before spawning/starting the driver; optional creation-time base state can be installed before PTY drain begins. |
| `crates/daemon/src/session.rs:516` | `create_until` constructs `OperationFingerprint::Create`; initial base observations must join the fingerprint if supplied to named creation. |
| `crates/daemon/src/operations.rs:810` | `session_create_for_attach` and `attach_created` at `:827`; named interactive creation is currently a create RPC followed by attach, so both need the bootstrap profile. |
| `proto/zterm/v2/session.proto:22` | `SessionCreateRequest`; add optional typed initial base plus the required terminal contract marker for interactive initialization. |
| `crates/daemon/src/session.rs:2447` | `ActorAttachment`; store at most one proposed base observation set for a pending takeover, plus its sequence. |
| `crates/daemon/src/session.rs:2539` | `SessionCommand`; add `UpdateBaseColors`, executed with deadline and current attachment authorization. |
| `crates/daemon/src/session.rs:3348` | `prepare_attach`; normal attach may install base before its initial capture; pending takeover must not change the old controller's effective colors. |
| `crates/daemon/src/session.rs:3546` | `snapshot_applied`; preserve existing initial/replacement acknowledgement barriers after palette changes. |
| `crates/daemon/src/session.rs:3746` | `takeover`; install the candidate base at the ownership handoff, not preparation time. |
| `crates/daemon/src/session.rs:3981` | `require_resize_controller`; already authorizes replaceable controller state during normal first/replacement synchronization. Color observations should reuse the same authority, with explicit pending-takeover staging. |
| `crates/daemon/src/session_wire.rs:738` | Decode/validate attach metadata before stateful preparation and `create_main`. |
| `crates/daemon/src/session_wire.rs:1153` | `process_attachment_frame`; route the new structured color-observation control, validating exact attachment ID before actor mutation. |
| `crates/daemon/src/client/session.rs:300` | `connect_inner` constructs attach request. Add initial base observation and mandatory semantic-contract version. |
| `crates/daemon/src/client/session.rs:510` | `reconnect_remote_once`; resend latest observation set with the new attachment, not bytes or an old attachment ID. |
| `crates/daemon/src/client/view.rs:538` | `TerminalViewCommandWriter::resize` and the command enum at `:636`; corresponding typed color update API belongs here. |
| `crates/daemon/src/local_ipc.rs:478` | Local attach validation/routing; preserve target-issued attachment identity. |
| `crates/daemon/src/remote_tunnel.rs` | Remains bounded opaque tunneling; no color decoding or translation here. |
| `proto/zterm/v2/terminal.proto:7` | Attach request; tag 9 is reserved, so use fresh tags. |
| `proto/zterm/v2/terminal.proto:160` | Style/color DTOs; extend canonical v2 semantics and conversions. |
| `proto/zterm/v2/terminal.proto:189` | Surface/delta/history frames; add complete colors plus a fixed semantic contract marker. |
| `proto/zterm/v2/wire.proto` | New control kind after the current 322 allocation; do not reuse retired terminal kinds. |
| `crates/proto/src/lib.rs:404` | `WireKind`, exact registry mapping and control limits. Color update is structured control, not content or raw child OSC. |
| `crates/proto/src/lib.rs:1224` | Terminal color/style/surface conversions; enforce bounded channels, exact 256-entry palette and legal Dynamic roles. |
| `crates/cli/src/terminal_ui.rs:185` | Raw terminal entry and guard; capture outer mode/subscription state and restore any state changed by ZTerm. |
| `crates/cli/src/terminal_ui.rs:239` | `run_view`; start bounded base-color acquisition using the already sole stdin pump before preparing attachment. |
| `crates/cli/src/terminal_ui.rs:414` | `await_while_inactive`; outer replies must continue to be collected during preparation and acknowledgement, not become inactive keyboard input. |
| `crates/cli/src/terminal_ui.rs:2241` | `HostInputEvent` / `HostInputCodec`; recognize typed outer color/mode replies in the existing host-input decoder before keyboard/prefix forwarding. |
| `crates/cli/src/terminal_ui/session.rs:148` | Active select loop; collect replies/notifications, coalesce probes/updates, and mark presentation dirty for colors alone. |
| `crates/cli/src/terminal_ui/surface.rs:30` | Transactional surface candidate; include color state and keep failed candidates uncommitted. |
| `crates/cli/src/terminal_ui/composition.rs:139` | `ComposedFrame`; carry child colors separately from status/scrollbar chrome. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:129` | Selection overlay; apply session selection roles to selected content, with dynamic reverse-video fallback. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:147` | Equality/dirty-run shortcut; include resolved color identity so unchanged cell semantics still redraw on palette changes. |
| `crates/cli/src/terminal_ui/ansi_presenter.rs:320` | Sole ANSI style encoder; map effective colors and exact underline style/color here. |

### Pinned Alacritty / vte capabilities

`Cargo.toml:21` pins `alacritty_terminal = "=0.26.0"`, default features disabled. `Cargo.lock` pins `vte 0.15.0`. The local dependency sources inspected are under `/Users/huyuanzhe/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`.

| Upstream evidence | Consequence |
| --- | --- |
| `alacritty_terminal-0.26.0/src/term/color.rs:6–25`: 269 `Option<Rgb>` slots; foreground/background/cursor are 256/257/258. | Supports simple overrides but cannot represent Inherit vs explicit Dynamic vs Unknown, selection slots or cursor text. |
| `.../term/mod.rs:952`: public read-only `colors()`. `:1662` and `:1692`: `Handler::set_color`/`reset_color` mutate slots. | Could implement basic OSC with upstream storage, but all approved extensions would require awkward auxiliary metadata. The chosen centralized color owner leaves these upstream palette paths unused. |
| `.../term/mod.rs:1675`: `dynamic_color_sequence` emits `ColorRequest(index, closure)`. | Deferred handling would observe later state if answered after the entire ingest. Do not admit raw upstream callbacks; answer canonical queries at dispatch time. |
| `.../term/mod.rs:1835`: RIS resets grids/modes/title/keyboard stacks but **does not clear `colors`**. | Color reset lifetime must be explicitly implemented/documented; it cannot be inferred from `feed_reset`. |
| `vte-0.15.0/src/ansi.rs:1365`, `:1421`, `:1495`: OSC 4 and 10–12 setters/queries and 104/110–112 resets exist upstream. | No engine dependency upgrade is needed for basic syntax, but the existing policy remains responsible for exact bounded approved protocol coverage. |
| `vte-0.15.0/src/ansi.rs:1832–1924`: handles 4:0–5, SGR 58/59 and semicolon/colon RGB/indexed color forms while continuing supported sibling attributes. | Removing ZTerm's broad filter plus projecting these values fixes C12/C13. No replacement SGR parser is needed. |
| `.../term/mod.rs:1881` stores underline colors and distinct underline flags; `.../term/cell.rs:197` exposes `underline_color()`. | Projection should copy fixed semantic values; default underline color follows text foreground. |
| `.../term/cell.rs:123`: `CellExtra` has a fixed optional color plus combining `Vec<char>` and optional hyperlink. | Colored underline adds at most fixed extra metadata per retained cell/template. Retain combining limits and blocked hyperlinks; do not count all underline-only cells as combining cells or truncate ordinary underlined screens at 4,096 cells. |
| vte private-mode mapping lacks mode 5; modern appearance and color-stack/OSC21 controls are not implemented. | Store those orthogonal modes in `ZtermColorState`, intercept before generic forwarding, and report them from the same state. |

The frontend source-policy check prohibits importing any terminal engine and prohibits brand/application detection. New host reply framing belongs in its existing input codec; no Alacritty/vte dependency, `TERM_PROGRAM` routing, Herdr branch or terminal-brand heuristic should be added.

### Proposed domain and engine APIs

These are design signatures, not final naming requirements:

```rust
struct TerminalRgb { red: u8, green: u8, blue: u8 }
enum TerminalColorValue { Unknown, Rgb(TerminalRgb), Dynamic }
enum TerminalAppearance { Unknown, Dark, Light }

struct TerminalBaseColors {
    palette: [Option<TerminalRgb>; 256],
    foreground: Option<TerminalRgb>,
    background: Option<TerminalRgb>,
    cursor: TerminalColorValue,
    cursor_text: TerminalColorValue,
    selection_foreground: TerminalColorValue,
    selection_background: TerminalColorValue,
    appearance: TerminalAppearance,
}

struct TerminalColorSnapshot {
    changed_at: Revision,
    // complete effective palette + roles + appearance
    // (not the mutable base, overrides, stack or controller identity)
}

enum ColorOverride { Inherit, Rgb(TerminalRgb), Dynamic }
struct ZtermColorState {
    base: TerminalBaseColors,
    // fixed role/palette override slots; Dynamic invalid for palette/fg/bg
    // bounded complete stack entries; one owner for aliases of stack protocols
    // appearance subscription + screen reverse-video flags
}

TerminalModel::update_base_colors(&mut self, base: TerminalBaseColors)
    -> Result<TerminalUpdate, TerminalError>
TerminalDriver::update_base_colors(&self, base: TerminalBaseColors)
    -> Result<Revision, TerminalDriverError>
SessionAttachment::update_base_colors_until(&self, sequence: u64,
    base: TerminalBaseColors, deadline: Instant)
    -> Result<Revision, DaemonError>
TerminalViewCommandWriter::update_base_colors(&self, base: TerminalBaseColors)
    -> Result<(), DaemonError>
```

Use fixed arrays internally and validate the wire list has exactly 256 slots. Index RGB channels are already byte-sized in core; protobuf integers must be checked before conversion. Role Dynamic is legal only where ZTerm actually implements a dynamic rendering rule. Unknown is a knowledge state, not black, zero or the absence of a delta.

`update_base_colors` validates and preflights the next Revision before mutation. Equal observations are a no-op; a changed base may matter even when an override hides it, since a later reset reveals that base. Advance once for a changed base, preserve the effective-color `changed_at` unless effective rendering/query state changed, and notify only for meaningful published changes required by the selected protocol contract.

No global config/SQLite schema is required. This is a daemon-lifetime Session feature; terminal state is currently not persisted across daemon restart.

### Query/mutation/reply ordering

Child stream examples must be interpreted at their exact position:

```text
OSC 11 query ; OSC 11 set red ; OSC 11 query ; OSC 11 reset ; OSC 11 query
          old reply               red reply                      base reply
```

`dispatch_string` should parse a completed bounded control, validate keys/values, then call typed engine color methods and append immediate replies to `UpdateCollector`. A combined OSC with queries and setters must apply its specified left-to-right semantics; an implementation may not queue query indices and read final state at the end of the chunk. Whole-chunk, one-byte, fixed-chunk and deterministic-random chunking must agree on final state and concatenated replies.

Do not send child queries through the network to the current viewer and wait for a response. They are answered from authoritative effective state. Outer acquisition is proactive/asynchronous; a missing value returns the protocol's honest unsupported/unknown behavior. In legacy OSC query families, no invented RGB response is allowed. OSC21 distinguishes an unsupported field (`?`) from a supported dynamic or currently undefined field (empty value), following the exact parent protocol design.

Today a model ingest releases its mutex before reply I/O and revision publication, while actor resize mutates through another path. Adding a base update that emits appearance replies makes this an observable ordering problem. Introduce one driver commit gate for **ingest, resize and base updates**, held from revision preflight through reply write and revision publication. Release the model mutex before blocking reply I/O, keep child interrupt/reaping independent, and document one lock order. Alternatively route all three typed mutations through one bounded owner queue, but that is a larger change; do not add a second uncoordinated reply sender.

The ordered transaction must guarantee:

- a later update's notification cannot precede an earlier color inquiry response;
- revision watermarks cannot regress because an older ingest publishes after a newer base/resize mutation;
- input bytes and reply bytes are written atomically through the same existing PTY I/O owner;
- failure/overflow remains driver-fatal with truthful finalization, never a silently missing suffix of child replies;
- no network or controller response wait holds the gate, model lock or PTY drain.

Review the existing terminal-driver spec's wording about I/O mutex scope when introducing the gate; preserve its actual intent that PTY writes cannot block owner-only child interruption or create a global Session lock.

### Base acquisition and host reply ownership

Use the existing sole `StdinPump` and extend `HostInputCodec` with typed color query replies, appearance reports, and relevant mode reports. Add a bounded outer-probe state machine in a focused CLI module. It tracks requested slots, current probe generation, deadline and one coalesced result; it never owns a grid or decodes child output.

Probe after entering raw mode, using a short overall deadline rather than 256 sequential timeouts. Query all supported slots in bounded batches; use fresh bounded retries only on defined refresh triggers. Ordinary keyboard, mouse, paste and prefix handling must remain live. During startup/sync waits, process outer replies even though ordinary input is fenced. Recognized terminal replies, including stale or unsolicited ones of the supported query families, must never leak into the child's stdin.

Preserve paste framing first: OSC-looking bytes inside bracketed paste are literal paste content. For timeout/cancellation/oversized partial replies, consume only recognized response syntax with a bounded discard-until-terminator state; do not accumulate arbitrary OSC forever, split it into key suffixes or swallow unrelated keystrokes.

On a valid appearance/palette change notification, schedule one bounded requery and send one newest full observation set. Same-dark/light palette changes also trigger requery. Avoid overlapping batches because legacy replies have no request IDs; retire/drain one generation before starting another. No claimed query-generation accuracy is possible for indistinguishable late replies without this constraint.

The outer query subscription is frontend-owned state separate from the child's virtual subscription. Capture any supported original subscription mode before changing it, restore it on every normal/error/panic/signal exit, and do not enable a mode that cannot be restored safely. Unsupported notifications imply refresh only at supported triggers such as attach/reconnect and optional bounded focus refresh; there is no promise to detect an outer theme change that the terminal cannot report.

### Attachment and controller lifetime

| Transition | Recommended required behavior |
| --- | --- |
| New Session with no controller | Empty/Unknown base, no app overrides, no subscription, no saved colors. PTY keeps draining; queries do not block waiting for an attachment. |
| First normal attach / attach to detached Session | Reserve the current controller generation, install its validated base before the initial capture, retain existing app overrides/subscription/stacks, and emit a revision/notification only if effective state changed. |
| `create_main` or named create-and-attach | Carry the completed bounded initial observation profile all the way into model construction, before PTY spawn/driver start. Missing observed slots are Unknown, but known initial values must never lose a race to startup queries. Do not invent a separate configuration palette. |
| Pending takeover | Store the candidate base on that attachment only. The existing controller's base and queried/rendered colors are unchanged until takeover commits. A candidate may receive semantic preview using the current Session palette. |
| Takeover commit | Check/preflight generation and color update, replace the lease and base at the actor's ownership boundary, retain Session overrides/stacks/subscription, invalidate old proposals. If colors changed after the prepared snapshot, send a replacement current snapshot and preserve acknowledgement readiness; do not activate against colors that were never presented. |
| Active refresh | Only exact current attachment/generation may apply it; reject stale/lost controllers before model mutation. Sequence numbers are monotonic within an attachment and do not survive a new attachment ID. Equal/stale duplicate proposals do not notify repeatedly. |
| Replacement visual sync | Color observations are replaceable controller state like resize and can be admitted for the exact current controller. Ordinary first-attach input remains fenced. |
| Detach / network interruption | Retain last-known base and all virtual app state. Drop proposals from released attachments. No viewer callbacks are awaited and no state clears merely because the controller disappeared. |
| Remote resume | Reuse existing revision/checkpoint rules; checkpoint now contains projected color state. Resend latest frontend observations with the new attachment. Any base change appears in the merged delta or full resync. |
| Switch main/alternate screen | Palette/default overrides, color stack and appearance subscription are terminal-global and persist. Per-cell text/rendition templates remain upstream screen-owned. Do not treat alternate exit as an app-exit theme restore. |
| SGR 0 / 39 / 49 / 59 | Reset cell rendition / the selected rendition component, never palette/base state or the color stack. |
| Explicit OSC reset | Clear the targeted override and resolve against **the latest controller base**, not a palette captured at Session creation. |
| RIS | Explicitly reset the ZTerm-owned app color overrides, stacks, reverse-video/subscription state according to the final protocol contract; keep base observations. Upstream RIS alone does not do this. |
| Child process exit within live shell | No guessed process boundary. Colors are restored only by explicit terminal protocols or eventual Session reset/end. |
| Session/root end or daemon restart | Destroy this color state with the existing model; do not persist it into configuration or leak it into another Session. |

The current actor already has exactly one controller and one pending takeover. A new proposal should be bounded to one full base set per attachment, not a per-update queue or a global target map.

**Exact new-session bootstrap:** `run_view` completes one bounded initial outer probe before calling `prepare`; `prepare` receives that profile by value. For `create_main`, extend the typed `prepare_attach_until` / `RemoteAttachmentRequest` metadata, pass the profile through `default_main` (`session.rs:986`) only in `NameReservation::Owner`, through `create_reserved` and `create_reserved_inner`, then install it when constructing `TerminalModel` at `session.rs:1082`, before the spawner at `:1085` and `TerminalDriver::start` at `:1094`. No PTY owner change is needed. A `NameReservation::Existing` or `Waiting` path must not replace any existing model's profile; it proceeds to the actor's lease decision.

Named interactive creation currently takes a different path: `prepare` (`terminal_ui.rs:617`) calls `LocalRuntime::session_create_for_attach` (`operations.rs:810`) and then `attach_created`. Add the same initial profile to this create API, `SessionCreateRequest`, frontend/remote unary conversion and `SessionService::create_until/create_inner/create_reserved`. Include it in `OperationFingerprint::Create` so byte-identical retries do not substitute a new palette under the same operation ID. A noninteractive detached create may omit the profile and intentionally starts Unknown. The subsequent attach carries the same/latest profile but does not reset app overrides created by the startup shell.

**Exact existing-session handoff:** validate observations and generation exhaustion before committing mutations; a normal attach installs its profile only after it owns the no-controller lease, then captures its initial snapshot. A takeover keeps one pending profile on the candidate attachment until the already synchronized takeover operation commits. At takeover commit, apply the profile in the same actor command that replaces the controller, preserve app overrides, and produce a replacement snapshot/`Awaiting { target: Active { generation } }` if the effective colors changed after the prepared snapshot. Publish `Active` only after the current authoritative snapshot has been acknowledged. Failed preparation or a stale/expired command leaves the current controller and colors unchanged.

### Stack semantics

Both xterm and OSC 30001/30101 aliases should target the same bounded color owner. A saved entry records the effective known values and Dynamic/unresolved semantics observed at the push/store operation, not merely a copy of override flags. Otherwise changing the controller base while a temporary app theme is active makes pop restore a different palette from the one saved.

Known saved values restore as explicit values; reset later returns to the current base. Dynamic restores as Dynamic. A saved Unknown can retain inherited semantic rendering, but cannot promise the previous outer terminal's unobserved RGB. The stack must not restore an old controller identity, base observation authority, subscription flag or whole-screen reverse-video mode unless the corresponding protocol explicitly requires it. Use documented bounded push/pop/store/index semantics and tests for underflow, overflow, repeated indices and mixed protocol aliases; parent protocol research owns the exact slot behavior.

### Snapshot, delta and history correctness

Keep Default/Indexed/RGB on cells. Snapshot and checkpoint include a complete `TerminalColorSnapshot`. Delta should carry complete latest colors rather than a palette patch language: a 256-slot table is small, bounded and much easier to validate transactionally than sparse partial state. Equal palette deltas can later be optimized only if measurements justify it.

Color snapshots contain a `changed_at: Revision` (a stamp from the **existing** revision domain), constrained to be at or before the enclosing surface/history revision. This avoids inventing another global counter and lets the frontend reject rollback through late history responses. Missing colors are an invalid semantic frame under the new required contract, not an empty/default palette.

History frames carry the colors at their model anchor revision; cached rows remain semantic. The frontend uses the newest validated color snapshot from its live surface/history event stream to present retained rows. A late history reply with older `changed_at` cannot replace newer colors. A newer history response can carry the latest color state even while the corresponding live delta is in flight. Preserve the last successfully flushed presentation until both complete candidate state and output succeed.

Palette changes do not invalidate history coordinates or force the model history epoch forward. They mark the visible frame dirty, including pinned history, but need no history refetch because cell indices/defaults are retained. Never bake the old palette into cached row RGB or silently retain the previous composed RGB frame on the history path.

### Presenter and isolation

Resolve colors in the sole presenter after composing semantic content and before dirty comparison/ANSI output. The committed baseline must represent both the semantic row content and the color interpretation that actually reached stdout. Either compare resolved rows or force content rows dirty when color identity changes. Equality on raw `TerminalCell` alone is insufficient.

Do not forward child OSC 4/10/11/17/19 setters to the physical terminal. Convert known defaults/indices to RGB for content painting. Unknown defaults/indices keep legacy semantic SGR. This isolates sessions and prevents changing the terminal's global palette/defaults on detach. Fill empty child-content cells using the effective background; physical padding/status/gutter have their own outer/chrome meaning and must not accidentally receive child overrides.

Selection uses the effective Session selection foreground/background when explicit, otherwise its implemented Dynamic reverse-video rule. It applies only to selected child cells, not status chrome. Apply cell inverse, screen reverse-video, selection and cursor in one documented precedence. Underline default follows the resolved text foreground; explicit underline index resolves through the same effective palette.

The parent design chose a software block cursor whenever explicit cursor or cursor-text customization requires it. Hide native cursor in that path, paint the underlying cell as an overlay and restore it through ordinary frame diffs, while still emitting the authoritative CUP for IME position. With inherited normal cursor state, keep the native cursor. Do not mutate the outer cursor's configured color. The current DTO does not carry cursor shape, so this color task does not add a cursor-shape protocol family.

OSC21 supported keys are 0–255, foreground/background, cursor/cursor_text, selection_foreground/selection_background. Unknown visual_bell/opacity/transparency keys are reported honestly as unsupported; they are not silently accepted state that cannot be rendered. No native window/background-image/opacity feature is implied by C16.

### Wire and version compatibility strategy

The current spec requires wire major 2, ALPN `zterm/2`, a single semantic representation and no presentation capability fallback. New required palette semantics cannot safely rely only on protobuf unknown-field compatibility: an old client would silently ignore colors, and an old server would silently ignore base proposals.

**Corrected final recommendation after checking old-server side effects:** retain `zterm.v2` and wire major 2, but allocate fresh canonical request kinds for **both `TerminalAttachRequest` and `SessionCreateRequest`**, retire old 300 and 202, and provide no fallback. Current free numbers are 323 for attach and 209 for create; the parent owns final allocation and could then use 324 for the color-update control. Keep the same typed message names and existing response kinds, adding required complete color semantics to content responses. A fixed marker may remain as structural defense, but it is not the side-effect boundary; the fresh kind is.

Evidence: `FrameDecoder` validates the numeric kind while `session_wire.rs:408` `read_first` builds the first frame, before `local_ipc.rs:379` can select attach or `session_wire.rs:488` can dispatch Session unary work. An old server rejects an unknown fresh request kind before decoding its create/attach payload. A new server removes retired 202/300 from the canonical kind mapping and rejects old requests at that same boundary. This matches the existing explicit unknown-kind and retired-kind policy rather than adding version negotiation.

- New server rejects old request kinds before `create_main`, controller lease, terminal-model or PTY effects.
- New client uses only fresh attach/create kinds. An old local daemon or remote target rejects them before Session effects; there is no retry with 300/202.
- New client still validates required complete color state and any chosen contract marker on an initial snapshot/resume delta before acknowledgement/input. Unknown fields alone cannot establish semantic compatibility.
- Color snapshots in all content messages are required by domain conversion; no defaulting missing messages to Unknown.
- Old and new endpoints fail explicitly rather than silently display wrong colors. Ship CLI/daemon/remote-host changes together; do not claim a live migration of retained old-process Session state.
- Use fresh protobuf tags; keep attach tag 9 reserved. Retire 202/300 alongside 312/313/315/316/319/320/321; update both `WireKind` conversion/registry tests and `tests/source-policy.sh` retirement assertions. Do not leave an active alias mapping for old numbers.
- Keep 8 MiB frame / 1 MiB control caps, validate repeated palettes before expensive allocation, retain exact target-issued attachment IDs and route authorization.
- The viewer-daemon remote tunnel remains opaque. Do not add per-terminal-feature logic there.
- Increment `CHECKPOINT_FORMAT_VERSION` from 2. A stale projection forces a full snapshot; there is no checkpoint persistence migration.

Changing this to a full wire-major/ALPN bump is broader release policy and would affect pairing/distribution tests. Existing repository single-v2 contracts favor fresh non-negotiated request kinds unless the parent explicitly chooses the broader rollout.

**Why the earlier marker-only proposal was insufficient:** on a new client talking to an old server, unknown attach fields are ignored. Old kind 300 with `create_main=true` can reserve/create `main`, spawn its login shell, start the PTY/model driver, capture the initial snapshot and reserve an attachment/controller before the client rejects the response's missing marker. An attach to an existing detached Session can similarly reserve a controller and consume/replace the resume checkpoint. Client disconnect releases the attachment eventually; it does not undo an already created Session or startup-shell effects. Named interactive creation is even earlier: old kind 202 creates and runs the named Session, and its unchanged `SessionMutateResponse` can be accepted before the following attach discovers incompatibility. Retaining only a response marker therefore permits orphaned live Sessions and startup commands on old servers; this must not be described as side-effect-free compatibility rejection.

**Why fresh attach plus preflight is not the minimum:** an actual new read-only contract request and positive response before named creation could reject old endpoints, but costs an additional protocol operation/round trip and must bind the later create to the same verified server incarnation/connection. A probe followed by legacy kind 202 without that binding has a check/use gap across reconnect, daemon restart or rollback. Misusing an invalid/nonexistent attach and interpreting its error as feature support is not a reliable positive contract. Moving create to fresh kind 209 requires only the registry/dispatch plumbing already touched by its new bootstrap profile and closes the gap intrinsically, without cached capabilities or preflight state.

Before rejecting a fresh create, a new client can still have performed normal daemon discovery/startup, target resolution, transport establishment or operation-lease issuance using unchanged safe control requests. An old server may allocate an empty operation lease. The guarantee is specifically **no Session creation, PTY spawn, controller takeover or child-input effect from an incompatible color-aware request**; it is not zero network/control-plane activity. Keep any issued empty lease under existing bounded retirement rules.

### Meaningful validation

No tests were executed for this research-only task. The implementation should add focused conformance evidence and run the existing checks below after the relevant layers are complete.

```sh
cargo test -p zterm-core
cargo test -p zterm-terminal
cargo test -p zterm-proto
cargo test -p zterm-daemon --test controller_lease
cargo test -p zterm-daemon --test attachment_resync
cargo test -p zterm-daemon --test local_session_ipc
cargo test -p zterm-daemon --test terminal_drain
cargo test -p zterm-daemon --test terminal_recovery
cargo test -p zterm-daemon --test two_daemon_transport
cargo test -p zterm-cli --lib
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
sh tests/source-policy.sh
```

The final implementation runner may broaden to `cargo test --workspace --all-targets --all-features` once, as required by CI. Native physical-terminal evidence remains separate from pure model/duplex tests.

Required new assertions:

1. Set/query/set/query/reset inside one input; same result for every chunking regime, C1/7-bit and BEL/ST terminations, malformed/overflow/cancelled inputs.
2. Palette-only/default-only changes produce a zero-row-patch advancing delta; replay equals a fresh snapshot; old Default/Indexed history recolors; explicit RGB does not.
3. Underline 4:0–5 and 58/59 preserve adjacent foreground/background/reset attributes, wide cells and combining text through snapshot/delta/history/wire/presenter.
4. Unknown base answers no fabricated legacy RGB, known app overrides answer correctly while disconnected, OSC21 distinguishes unsupported and Dynamic/unresolved state.
5. Base update raced against an earlier query/set chunk cannot invert replies or regress revision watch; blocked I/O in Session A does not prevent child interrupt or Session B progress.
6. Pending takeover cannot mutate colors; lease handoff replaces only base and keeps overrides/stacks; stale proposal and remote generation revocation are no-ops on the model.
7. Detach/reconnect/resume preserves virtual colors, subscription and stack while rebasing inherited slots to the new controller; old history replies cannot roll color state back.
8. Outer reply bytes never become keystrokes through Active/inactive/reconnect/prefix paths; bracketed-paste lookalikes remain paste; delayed/partial/overflow notifications have bounded state.
9. Probe/subscription entry and exit restore outer state after cancellation, signal, panic and output failure; palette/default/cursor OSC setters are absent from physical output.
10. Software cursor/selection overlays correctly restore underlying cells, honor current palette, remain scoped to content, and maintain physical CUP/IME position.
11. Old/new request-kind incompatibility is explicit in both directions before Session/PTTY effects: old 202/300 are rejected by the new registry, fresh create/attach are rejected by a frozen old-server decoder, no fallback is sent, and Session count/creation fixture remain unchanged. Required initial color semantics also fail before acknowledgement/input; viewer tunnel remains opaque and local/remote routes produce equivalent semantic state.
12. Stack save/pop after a controller-base change restores saved effective values; targeted reset returns latest base; main/alternate and RIS cases obey the chosen lifetime contract.

### Related specs

- `.trellis/spec/backend/terminal-model.md`: host-only engine, bounded ingress, reply order, cell extras, semantic projection, checkpoint/history. Several current exclusions must be replaced with the newly approved explicit contracts.
- `.trellis/spec/backend/terminal-driver.md`: bounded PTY drain, reply I/O, latest-only watermarks, child ownership and mutation lock ordering.
- `.trellis/spec/backend/session-service.md`: one controller, pending takeover, acknowledgement and replacement sync, remote resume, no Session shutdown on detach.
- `.trellis/spec/backend/core-wire-domain.md`: transport-neutral types, one v2 representation, transactional validation, exact IDs, frame limits and retired kinds.
- `.trellis/spec/backend/local-daemon-ipc.md`: sole stdin/presenter ownership, raw-mode lifecycle, viewport/history cache, opaque remote tunneling and reconnect.
- `.trellis/spec/guides/cross-layer-thinking-guide.md`: trace each boundary and keep a single decoding/validation owner.
- `.trellis/spec/guides/root-cause-and-architecture-thinking-guide.md`: fix the missing owner rather than introduce application-specific rendering exceptions.
- `.trellis/spec/frontend/state-management.md` is still a placeholder; active CLI state/presentation rules are in the backend local-IPC contract.

### External references

- [Alacritty terminal 0.26.0 `Term` API](https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/term/struct.Term.html), verified with exact locally installed source as cited above.
- [vte 0.15.0 source](https://docs.rs/crate/vte/0.15.0/source/src/ansi.rs), exact local copy used for parser behavior. The Handler HTML endpoint returned an error during browsing, so no claim depends on that page.
- [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html), inspected 2026-09-06 for XTPUSHCOLORS/XTPOPCOLORS/XTREPORTCOLORS and DECSCNM. Parent protocol research owns byte-exact coverage details.
- [Kitty color control](https://sw.kovidgoyal.net/kitty/color-stack/), inspected 2026-09-06 for dynamic/undefined vs unsupported keys and saved-color scope. The protocol explicitly distinguishes these states, which is why a plain optional-RGB override table is insufficient.

## Caveats / Not Found

- This is source/design research. No actual outer terminal, remote host, Herdr or Pi setting was inspected or changed; this does not prove the screenshot's root cause.
- Ordinary ANSI text can retain inherited colors when the outer terminal cannot answer, but exact inherited RGB, missing native notifications and unobserved theme changes cannot be manufactured. Document this as capability-dependent observation, not a failed query answered with a guessed default.
- Screen reverse-video details and RIS color-stack/reset semantics must follow the final parent protocol matrix; the architecture supports them without assuming alternate-screen switches are resets.
- No engine upgrade, third-party terminal fork, second CLI terminal engine or database migration is needed for the recommended design.
- No user-owned functional decision remains within the confirmed protocol scope after the parent selected supported OSC21 roles and the software custom-color cursor. Release policy requiring mixed old/new versions to remain usable would be a separate user-owned compatibility requirement; current repository policy supports explicit incompatibility instead.
