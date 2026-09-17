# Terminal notification forwarding design

Status: approved on 2026-09-17; implemented, automated validation and user-reported
Ghostty popup verification passed. Commit, PR creation and merge are authorized.

## 1. Change boundary

The gap is between recognized PTY output and transient client notification effects.
The host's existing ingress policy owns notification parsing, Session owns the
current recipient, and each platform owns presentation. `DesktopPresenter` owns
physical terminal output; Kotlin owns Android system notifications. Extend these
owners without introducing another parser, raw-output tunnel, or application
detection in zterm.

```text
Child PTY bytes
  -> TerminalIngressPolicy: frame, validate, normalize notification
  -> TerminalUpdate: bounded transient effects
  -> TerminalEffectBroker: current controller + bounded notification FIFO
  -> Session wire: typed attachment-scoped notification
  -> shared SessionClient / terminal view: validate + bounded pending FIFO
  -> desktop: DesktopPresenter -> canonical OSC 9/777 + flush -> Ghostty
  -> Android: NativeTerminal transient event -> Kotlin -> system notification
```

The viewer-daemon remote tunnel remains an opaque carrier. Notifications never
enter snapshots, deltas, checkpoints, retained history, operation replay, or
persistent session state.

## 2. Input and domain contract

Add one validated `TerminalNotification` domain value in `zterm-core`, with two
forms: an OSC 9 message and an OSC 777 notification title/body. Preserve the form
instead of sending both variants or translating between them. Construction
validates text and canonical encoded length; Debug exposes only form and lengths.

| Input | Result |
| --- | --- |
| `OSC 9 ; message ST` | One OSC 9 message, requiring non-empty message text. |
| `OSC 777 ; notify ; title ; body ST` | One OSC 777 title/body notification; permit an empty title or body, but require at least one non-empty field. Split the header/title delimiters once and retain the remaining body, including semicolons. |
| Numeric ConEmu subcommand prefix in OSC 9, including progress `4;...` and abbreviated `4` | Consume as unsupported; never forward as a desktop notification or progress action. |
| Other OSC 777 extension, OSC 99, incomplete/cancelled/invalid request | No notification; preserve the existing bounded rejection/consumption policy. |

The numeric-subcommand rule reserves a leading ASCII decimal field followed by
`;`, plus the standalone `4` progress shorthand. This follows the documented
OSC 9 ambiguity rather than treating all OSC 9 commands as notification content.
Do not turn an excluded prefix into a valid request by trimming/sanitizing it.

Keep the existing 1,024-byte control-string bound, counted from the OSC command
through payload and excluding introducer/terminator. Domain validation applies
the same bound to the canonical body: `9;message` or `777;notify;title;body`.
Put the notification bound in the transport-neutral core and keep it consistent
with the existing terminal string limit; do not add a reverse dependency from
core to the terminal engine. No larger allocation exception like OSC 52 is
needed. Reject invalid UTF-8 and
control scalars in text; do not re-emit embedded ESC, NUL, C0/C1 or DEL controls.
Valid text is preserved rather than truncated. BEL used to terminate an OSC is
framing, not a separate audible-bell request.

Reuse the existing streaming framer and its ESC/ST, BEL, C1, CAN/SUB and overflow
handling. One necessary correctness detail is UTF-8 inside control strings:
`ingress.rs:302` currently tests raw `0x9c` as ST without accounting for a UTF-8
continuation byte. Make the same framer distinguish a standalone C1 ST from a
continuation within a valid scalar, across chunks. For example, the Chinese
character `果` contains a `0x9c` continuation. Keep raw C1 termination functional
and regress existing OSC 52/title/color framing around this change.

## 3. Transient effect storage and routing

Clipboard replacement and notification delivery have different retention rules.
Represent the output of one ingest as a bounded host-effect aggregate: one
latest clipboard value and up to 32 ordered notifications. Extend the existing
effect enum with a notification form for delivery, and update the model's
empty/resize/base-color results to emit an empty aggregate. This is a scoped
representation refactor; existing clipboard validation and latest-only behavior
must remain unchanged. Notification delivery does not impose new cross-kind
ordering guarantees on clipboard replacement; each notification FIFO preserves
the relative order of its own admitted requests.

The broker keeps its existing controller target and clipboard slot, plus a
separate notification FIFO capped at 32 entries. The shared client has the same
notification cap outside its lifecycle/control queue. A full notification FIFO
evicts the oldest pending notification and admits the newest; it never blocks
PTY ingestion, grows indefinitely, or overwrites the clipboard slot. Below the
cap, distinct requests, including identical text sent twice, remain distinct and
ordered. Per-ingest accumulation follows the same bounded rule.

Recipient eligibility follows `reconcile_effect_target` in
`crates/daemon/src/session.rs:4222`: the exact controller must have become Active
in its current generation. Ordinary same-attachment synchronization after Active
does not count as a disconnect. Initial/replacement attachments receive no effect
until eligible. Target assignment and enqueue are serialized by the existing
broker mutex; no network write runs while holding it.

No target means immediate discard. Target loss/change clears pending content.
The client also clears its queue on real reconnect, detach, lease loss, end,
error and event-reader closure. Notifications removed by these transitions are
not reconstituted from snapshots or retry state. An action already written to
the old outer terminal or committed to Android's notification service is already
delivered; no remaining item is retargeted, and existing OS notifications are not
reposted or withdrawn merely because the connection later closes.

Watch notifications are only wakeups. A consumer must check/drain remaining
pending entries before waiting again, so one coalesced wake cannot strand the
second notification. Keep lifecycle/cancellation handling ahead of stale
pending effects, use the existing bounded attachment-write deadlines, and check
the current target as entries are taken. Drain in bounded work units so a flood
cannot monopolize the Session writer or desktop event loop. The writer gives
visual updates a turn after each effect. The shared reader may defer one ordinary
visual event while returning a pending effect, then returns that visual event
next; lifecycle transitions retain priority. This prevents continuous repaint
from starving notifications without growing another event queue.

## 4. Wire and shared clients

Add a dedicated `TerminalNotification` control message in
`proto/zterm/v2/terminal.proto`, with the exact target attachment ID and a oneof
for OSC 9 message versus OSC 777 title/body. Reserve new kind **325** in
`wire.proto` and the Rust `WireKind` registry; 324 is the current last terminal
kind. Do not reuse a retired kind or overload the clipboard message.

The message is unsolicited (`request_id = 0`, `deadline_ms = 0`) and carries
validated semantic text, never raw escape bytes. Proto conversion rejects an
absent/unknown form, invalid target ID, invalid fields or oversize content. The
Session client additionally checks the target equals its current attachment.
Add generated-message Debug suppression and a content-redacted implementation.

Update the Session writer, shared client event enum/conversion, direct Iroh
Session-kind allowlist, and shared view notification queue. No target-issued ID
translation is introduced. Export the same typed event through an independent
transient UniFFI path for Kotlin notification posting. Do not store notifications
in NativeFrame or its conflated screen-state channel.

Follow the existing non-negotiated wire-v2 policy: no feature bit, downgrade,
second decoder or compatibility adapter. Older receivers reject unknown kinds,
so interoperability validation uses matching updated client/daemon builds;
document this rollout requirement. No release or deployment is part of this task.

## 5. Desktop presentation

Add a presenter operation accepting only the validated domain value. It emits
one complete sequence in the original form using 7-bit OSC and canonical ESC-ST,
followed by flush. It does not paint a frame or advance/change the presenter's
visual baseline, selection, cursor or host input modes. It remains independent
of screen-presentation cadence and DEC 2026 holds in the child model.

Use the same stdout owner as clipboard effects. Propagate actual output failures
through the existing terminal-I/O path; do not retry an ambiguous write, invent
an acknowledgement, or synthesize a bell/native notification fallback.

## 6. Android delivery

### Attachment-owned transient bridge

Extend the existing NativeTerminal actor with a bounded notification queue (32
entries, evict oldest on overflow) and a single-consumer asynchronous UniFFI
read API. Export semantic form/text and an opaque connection-generation token;
do not expose raw OSC bytes. Reuse the existing cancellation/closed-result
patterns. Waiting for an event must not block the actor, input, or frame delivery.

The application-owned AppRepository starts one notification consumer alongside
its existing frame observer after a successful attachment. Activity recreation
or background visibility changes do not create another consumer. Retirement
cancels and joins the consumer before detaching/closing the old NativeTerminal.
Native lifecycle transitions clear the queue and invalidate its generation on
connection loss, detach, lease loss, and session end. Same-attachment screen
synchronization does not invalidate a healthy recipient.

Fence delivery with both the repository's current attachment epoch/object and
the native notification generation; reconnect within one NativeTerminal must
also reject old events. Provide a small atomic generation-validity check at the
delivery boundary rather than inferring eligibility from a potentially stale
NativeFrame. Kotlin posts immediately after this check without another queued
coroutine hop or suspension. This is the delivery commitment boundary: later
connection loss cannot recall a notification already submitted to Android.
Do not retain a second unbounded Kotlin queue or replay the last event to a new
subscriber.

### System presentation and permissions

Use a focused Kotlin notification helper owned by the application repository.
Create one stable, localized `terminal_notifications` channel with default
importance and respect subsequent user/OS channel settings. Declare
`POST_NOTIFICATIONS`; request it on Android 13+ from a foreground, user-initiated
"Terminal notifications" action in Settings. That row reflects actual OS
permission/channel availability and provides access to system notification
settings when needed. Refresh its state when returning from system settings.
No notification event launches a permission prompt, and denied/disabled events
are consumed without delaying or failing the terminal connection. No separate
persisted app opt-in or settings migration is needed.

Map OSC 9's complete message to notification text with the localized app name as
the title; do not infer title/body from a colon in the message. Map OSC 777's title
and body directly, using the app name only when its title is empty. Use ordinary
text/expanded text, an appropriate small icon, default channel behavior, and
distinct notification IDs so repeated requests are not accidentally conflated.
Use private lock-screen visibility and avoid payload-bearing debug output.

An immutable content intent opens the existing app without automatically
reconnecting, switching sessions, taking over, or executing a terminal command.
Notifications are ordinary, dismissible, and auto-cancel on tap; add no custom
buttons, acknowledgement, or durable notification model.

Post in both foreground and background when the application still owns the
eligible live connection. Retain the current application's runtime/connection
lifetime. No foreground service, wake lock, keepalive, battery exemption or push
infrastructure is included; process termination and disconnection yield no
events and no later catch-up. Kotlin's OS permission/channel policy can suppress
display even when event transport succeeded.

## 7. Herdr and outer-terminal compatibility

Herdr 0.9.1's Ghostty backend is an OSC 9 producer; its `title: body` form needs
no special implementation. Use synthetic sequences as deterministic acceptance
fixtures, then an isolated Herdr/Ghostty smoke when available.

Herdr emits nothing if its own environment-based terminal detector finds no
supported backend. Zterm's hosted profile remains `TERM=xterm-256color`; it does
not impersonate Ghostty or alter the upstream application. Record the observed
backend inputs during an isolated smoke and distinguish a producer-selection
limitation from forwarding failure. Transparent Herdr detection in every remote
environment is deferred; the MVP guarantee begins when a supported request is
actually emitted. Likewise, an outer terminal or OS that suppresses notifications
may receive a correct request without showing a popup.

## 8. Validation, risks, and rollback

Validation maps to PRD A1-A11 in `implement.md`. Ingress owns framing/text cases,
core/proto own semantic and wire invariants, broker/client tests own lifecycle
and bounded queue behavior, and presenter/outer-PTY tests own emitted bytes and
visual-baseline preservation. Native bridge and Android tests own event lifetime,
permission/channel handling, Activity recreation and system-notification fields.
Real Ghostty display and Android foreground/background notification delivery are
separate platform evidence; building an APK does not prove either runtime path.

The highest-risk areas are UTF-8/C1 framing, preserving clipboard semantics,
coalesced wakeups with multiple pending notifications, attachment retirement,
and stale Android events crossing the native/Kotlin boundary.
Use focused deterministic regressions at those owners instead of adding a second
parser or an unrelated global scheduling/recovery framework.

There is no persistent-data migration. Revert the feature as one coherent
core/ingress/wire/client/desktop/Android change if needed; an old client with a new
sender is not a safe partial rollback. Reverting does not recover missed
notifications, consistent with the selected no-replay behavior. Android may
retain its harmless OS-managed channel; do not delete/recreate channels to reset
the user's choices.
