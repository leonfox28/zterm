# Ordinary terminal notifications

## 1. Scope / Trigger

Apply when changing OSC 9/777 ingress, transient host effects, kind 325, desktop
notification output, or the Android notification bridge/platform consumer.

## 2. Signatures

```rust
TerminalNotification::osc9(message: String) -> Result<Self, TerminalNotificationError>
TerminalNotification::osc777(title: String, body: String) -> Result<Self, TerminalNotificationError>
TerminalNotification::title(&self) -> Option<&str> // None identifies OSC 9
TerminalNotification::body(&self) -> &str
TerminalHostEffects::{push, pop, clear, is_empty}
TerminalNotificationQueue::{push, pop, clear, is_empty}
NativeTerminal::next_notification(&self) -> Result<NativeNotification, NativeError>
NativeTerminal::notification_is_current(&self, generation: u64) -> bool
```

`TerminalUpdate.host_effects` replaces the former singular optional effect.
`TerminalHostEffect` contains ClipboardWrite and Notification. Kind 325 carries
`TerminalNotification { attachment_id, oneof content { string osc9,
TerminalNotificationOsc777 osc777 { title, body } } }`; request/deadline are zero.

## 3. Contracts

- One ingress framer recognizes OSC 9 and exact `777;notify;title;body`, with
  BEL, ESC-ST and standalone C1 ST termination. Track UTF-8 continuation state
  across chunks before classifying raw `0x9c`: `果` contains this byte. Do not
  add a second parser or feed raw notification controls to the terminal engine.
- Domain construction preserves valid text. The entire canonical OSC body
  (command/separators included, framing excluded) fits 1,024 bytes. Text contains
  no Unicode control scalars; the OSC 777 title has no semicolon, while its body
  may. OSC 9 must be nonempty; OSC 777 needs at least one nonempty field. Reserve
  an ASCII numeric prefix followed by `;`, and bare `4`, for non-notification
  OSC 9 subcommands. Never sanitize an excluded command into an admitted one.
- Per-ingest accumulation, broker, shared view and Android bridge bound pending
  notifications to 32, evicting oldest on overflow. Repeated equal requests are
  distinct. Clipboard remains an independent latest-only slot. There is no new
  cross-kind ordering guarantee and no network backpressure on PTY drain.
- Session installs the exact eligible controller target using its existing
  current-generation Active/previously-Active rule. No controller means drop;
  target changes clear pending content under the broker mutex. Real reconnect,
  detach, lease loss, end/error and reader closure discard pending effects.
  Healthy same-attachment screen synchronization is not disconnection.
- Payload-free watches are wakeups, not queues. Observe the watch, take pending
  content, then await only if empty. Prioritize lifecycle/control handling and
  use bounded writer deadlines. Never strand a second entry after a coalesced
  wake, prefetch an effect outside the lifecycle-select boundary, or replay
  effects from snapshots/history/operation state.
- Core and protobuf Debug implementations redact all text, including nested
  oneof/OSC 777 DTOs. Protocol conversion validates form, fields, size and target
  shape; the shared Session client checks exact attachment identity and zero
  request/deadline. The direct Iroh allowlist includes kind 325. Matching updated
  client/daemon builds are required; there is no negotiation/downgrade path.
- DesktopPresenter alone emits one canonical 7-bit OSC 9/777 with ESC-ST and one
  flush. This does not advance the visual baseline or wait for a screen frame.
  An ambiguous output failure is not retried. Android exports semantic text
  through a separate bounded NativeTerminal bridge, never NativeFrame/watch state.
- AppRepository owns one Android consumer per attachment. Immediately before
  posting, check its attachment epoch/object and the native notification
  generation; no intervening coroutine hop. Native invalidation also fences
  reconnect within the same object. Cancel/join the consumer before closing it.
- Kotlin posts ordinary, dismissible notifications through stable channel
  `terminal_notifications` with default importance and private lock-screen
  visibility. OSC 9 uses app title + complete message; OSC 777 preserves title
  and body (empty title uses app name). IDs preserve distinct requests. Tap only
  opens the existing app, never reconnects/takes over/executes input.
- The Settings action owns Android 13+ POST_NOTIFICATIONS requests and system
  settings access. OS state is the permission authority, refreshed on resume.
  Denied/disabled events are consumed; grants do not replay them. Both foreground
  and background receive while the existing connection remains alive. No
  foreground service, push, wake lock or process-death delivery is included.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Malformed, oversized, cancelled, unsupported OSC | Consume safely; no request, screen residue or content-bearing diagnostic |
| Missing wire form/ID, invalid fields or mismatched attachment | Typed connection-local malformed frame |
| Pending FIFO exceeds 32 | Discard oldest pending request, admit newest; PTY continues |
| No eligible controller / lifetime retired | Discard; no replay or transfer to another device |
| Coalesced wake with several pending requests | Drain each request without needing another publication |
| Android permission/channel disabled or posting fails | Skip; terminal continues, no later retry |
| Activity recreation / ordinary repaint | No duplicate consumer or notification |

## 5. Good / Base / Bad Cases

Good: two equal OSC 9 messages become two events; clipboard replacement in the
same chunk stays independent. Base: a connected Ghostty or Android controller
receives one ordinary request. Bad: retaining notification text in a frame,
forwarding a numeric progress command, or relabeling a retired Android event with
the latest connection generation.

## 6. Tests Required

- Core/proto tests assert form/size/control/subcommand validation, FIFO overflow,
  independent clipboard replacement, exact round trip and nested redaction.
- `terminal/tests/notifications.rs` checks all chunk widths, Chinese C1-shaped
  continuation bytes, main/alternate and DEC 2026 independence, invalid input,
  title parsing and OSC 52 coexistence. Keep the original security/corpus tests.
- Broker and shared-client tests assert target retirement, observer exclusion,
  reconnect clearing, coalesced wakes, lifecycle priority and wire identity.
- Presenter tests own exact bytes, one write/flush, unchanged baseline and output
  failures. `local_session_ipc` and the real outer-PTY `daemon_autospawn` fixture
  verify composed delivery through the actual model, daemon, wire and CLI.
- Native bridge tests verify bounds, same-handle generations and waiter lifetime.
  `TerminalNotificationsTest` covers system fields, permission/channel policy,
  live foreground/background receipt, Activity recreation and detached output.
  Its real-network case requires an explicitly disposable host and private ticket.
  Count actual notifications, excluding `Notification.FLAG_GROUP_SUMMARY`:
  Android can asynchronously add a system-generated summary to active notifications.
  Record devices/API levels; APK compilation is not notification runtime proof.

## 7. Wrong vs Correct

Wrong: `wake.changed().await; take_one()` for a FIFO; further pending entries can
remain forever if wakeups coalesce. Correct: mark the watch observed, take one
pending entry inside the selected delivery branch, and await only if empty.

Wrong: store the last notification in NativeFrame and post on each Activity
subscription. Correct: consume once from the application-owned transient bridge
and validate the original lifetime token immediately before system delivery.
