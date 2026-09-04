# Terminal selection and clipboard design

## 1. Outcome and architectural correction

This change makes text selection and clipboard writes first-class terminal-boundary concepts without
moving either into the shared Session screen model.

The reported behavior is one cross-layer architecture gap, not a Herdr compatibility case:

- Zterm owns physical mouse capture but has no local selection owner when the child declines mouse
  reporting;
- the terminal ingress recognizes OSC 52 but has no controller-scoped transient host-effect path;
- the outer terminal cannot see a Zterm-local selection, while Zterm currently disables the child
  keyboard protocol needed to receive Cmd+C reliably;
- a pinned history viewport suppresses visual repaint while hidden deltas still advance the
  authoritative child cursor/keypad, bracketed-paste, focus, and keyboard modes, so treating those
  physical input effects as part of visual presentation alone can leave outer encoding behind the
  child state.

The correction adds four explicit owners:

1. one application-independent interaction router chooses exactly one owner per pointer event;
2. one CLI attachment-local selection controller owns range, source identity, extraction, and overlay;
3. one Session-scoped latest-only effect broker binds a validated child clipboard write to the
   controller that exists when the effect is published;
4. one host keyboard gateway mirrors child protocol state and consumes only Zterm's own copy action;
   its sole `DesktopPresenter` owner can commit the complete host-input projection independently of
   retained history pixels.

No process name, theme, terminal identity, or Herdr-specific branch is introduced. Zterm-owned Rust
remains `unsafe_code = "forbid"`.

## 2. End-to-end data flow

```text
PTY output
  -> TerminalIngressPolicy
       -> semantic bytes -> Alacritty grid + child keyboard-mode state
       -> safe replies -----------------------------------------------> PTY
       -> invalid/read OSC 52 -> payload-free rejection
       -> valid OSC 52 -> TerminalHostEffect::ClipboardWrite(text)
                              |
                              v
                    controller-targeted latest slot
                              |
                 kind 322, decoded structured text
                              |
             remote bridge / same-UID IPC validation
                              |
                 CLI latest transient effect slot
                              |
                              v
selection copy ----------> DesktopPresenter::write_clipboard
                              |
                    canonical OSC 52 to outer terminal
                              v
                  user's desktop system clipboard
```

Semantic screen state continues through snapshots, deltas, and history windows. Clipboard text never
enters those paths, a checkpoint, a reconnect baseline, persistence, or a child reply.

Physical input follows a separate path:

```text
outer mouse/key bytes -> bounded HostInputCodec -> interaction/key owner
  -> gutter/history/selection action, or
  -> child-mode-aware byte forwarding
```

## 3. Shared clipboard domain value and OSC 52 ingress

### 3.1 Domain type

`zterm-core` gains one validated `TerminalClipboardWrite` newtype and
`TerminalHostEffect::ClipboardWrite` variant. Construction requires:

- non-empty UTF-8;
- at most `MAX_TERMINAL_CLIPBOARD_BYTES = 524_288` encoded UTF-8 bytes;
- no NUL.

The value preserves all other bytes exactly, including tabs, newlines, carriage returns, and Unicode.
Its `Debug` prints only a redaction marker and byte length. It exposes no implicit `Display`.

`TerminalUpdate` gains `host_effect: Option<TerminalHostEffect>`. This is deliberately separate from
`TerminalSideEvent`: the latter remains a small bounded diagnostic/event collection and must not grow
into a 512 KiB sensitive-data queue. Resize and empty updates have no host effect.

### 3.2 Dedicated parser state

The ordinary OSC/DCS/APC/PM/SOS retention limit stays 1,024 bytes. After the ingress state machine has
unambiguously recognized `OSC 52;`, it switches to a dedicated bounded clipboard state. That state:

- accepts BEL, 7-bit ST, and existing supported C1 framing consistently across chunk boundaries;
- retains no more than the selector delimiter plus 699,052 Base64 bytes;
- on overflow/cancel consumes through the logical terminator and emits only a payload-free rejection;
- never feeds rejected residue into Alacritty as visible text.

Dispatch accepts exactly `52;c;<data>`, where `<data>` is non-empty canonical RFC 4648 standard padded
Base64. Length, alphabet, padding position, and unused trailing bits are strict. Decoding must produce
one valid `TerminalClipboardWrite`; `?`, an empty value, another selector, malformed Base64, invalid
UTF-8, NUL, or decoded overflow is atomically rejected. The decoded vector is moved into the domain
value, and the encoded buffer is dropped before publication.

One `UpdateCollector` retains only the latest valid clipboard effect in an ingest call. Invalid inputs
cannot erase a prior valid effect from that call and cannot expose either representation in diagnostics.
Alacritty keeps `Osc52::Disabled` as defense in depth.

There is no OSC 52 policy setting: bounded writes are always eligible for controller routing and reads
are always denied without a response.

## 4. Controller-bound transient effect broker

### 4.1 Broker state

`TerminalDriver` owns a shared transient broker beside, not inside, the terminal model:

```text
EffectBrokerState {
    target: Option<AttachmentId>,
    pending: Option<TargetedHostEffect>,
}
```

A Tokio `watch<()>` is only a wakeup/version primitive; it never clones or retains clipboard text.
The model thread publishes a completed `TerminalUpdate.host_effect` after releasing the model lock.
Under one broker mutex it snapshots the current target and replaces `pending`; with no target it drops
the effect. A slow writer therefore causes latest-wins replacement rather than PTY backpressure.

Each attachment has a wake receiver but `take_for(attachment_id)` removes a value only when the saved
target matches. Observer wakeups are harmless and cannot consume controller data.
The attachment subscribes before the Session actor installs a new eligible target, so a newly resumed
controller cannot miss a post-install wakeup.

### 4.2 Target linearization

The Session actor remains the only controller authority. One helper derives and publishes the eligible
effect target after every controller/synchronization transition:

- a first attachment is not eligible until its initial snapshot is acknowledged;
- an already-active controller remains eligible during its later visual resynchronization;
- while takeover is only prepared, the old controller remains the target;
- takeover commit changes the target under the broker mutex and clears any old pending value;
- detach, lease loss, Session end, or a controller-less interval clears target and pending;
- a resumed controller may receive only effects produced after its new target is installed; the old
  slot is cleared, so reconnect cannot replay content.

The broker lock establishes the event-time ordering between a Session target change and model-thread
publication. Random attachment IDs plus target replacement prevent a later attachment from claiming an
earlier request. The effect is best-effort: once taken for a write, failure drops it and never retries.

### 4.3 Bounded delivery

`attachment_writer` selects the effect wakeup alongside outbound control, lifecycle, and revision
watches. It takes at most one value, encodes one frame, writes it under the existing absolute deadline,
and then checks for a newer replacement. There is at most one in-flight transport write and one pending
broker value.

At the operations boundary clipboard effects do not enter the existing capacity-eight semantic event
queue. A second mutex-plus-wakeup latest slot feeds `TerminalViewEventReader`, which multiplexes normal
events and one transient clipboard value. Thus a slow CLI can retain one in-flight value and one
replaceable pending value, not eight 512 KiB event payloads. Existing frame/socket bounds remain the
transport backstop.

## 5. Wire, bridge, and direct cutover

The v2 protocol adds:

```proto
TERMINAL_CLIPBOARD_WRITE = 322;

message TerminalClipboardWrite {
  AttachmentId attachment_id = 1;
  string text = 2;
}
```

Kinds 319-321 stay retired. Kind 322 always uses `request_id = 0`, carries decoded text rather than
child Base64/control bytes, and is validated through the core domain value at every trust boundary.
The 512 KiB text plus protobuf overhead stays below the existing 1 MiB control payload cap.

The remote bridge accepts the event only from the exact remote attachment in `Active`, or in a
Synchronizing phase whose same controller was previously active. It validates the remote ID/value,
rewrites only the attachment ID to the frozen local view ID, and forwards it. Initial/prepared-takeover
phases, unsolicited IDs, and malformed values are protocol errors. Stream loss drops the value; it is
not placed in reconnect state.

Same-UID IPC decodes kind 322 only on an attachment event stream, revalidates the local attachment ID
and domain value, and produces a redacted `LocalAttachmentEvent`. Operations projects the same value to
the CLI transient slot.

This release is an all-node v2 direct cutover. It adds no capability bit, fallback kind, dual decoder,
old-version adapter, or mixed-version branch.

## 6. Attachment-local selection and pointer ownership

### 6.1 Selection state

A small core helper owns only renderer-neutral `TerminalTextRange` normalization and extraction from
validated `TerminalSurfaceRow` values. It contains no gesture, clipboard backend, ANSI, daemon state,
or engine type, so Android can reuse the exact wide/combining/wrap rules later.

A new CLI `selection` module owns the desktop attachment-local interaction data:

- zero-based content-cell anchor and focus;
- `Idle`, `Dragging`, `Finalized`, and `CancelledUntilRelease` gesture state;
- the exact successfully presented source identity;
- pure normalized range, glyph expansion, extraction, and overlay queries.

The state never enters core terminal state, daemon, wire, history storage, or another attachment.
`CancelledUntilRelease` preserves input ownership if content/mode changes in the middle of a drag, so a
release is swallowed instead of reaching a child that never received the press.

The source identity is one of:

- live: active screen, viewport size, and the complete surface revision successfully presented;
- history: immutable cached-window epoch/revision plus the translated first visible live-top row and
  viewport size.

`ViewportCache` exposes only the latter renderer-independent slice identity. Its translated coordinate
stays stable when monotonic live append shifts the absolute offset but the same immutable rows remain
visible. `ViewportController` commits the identity only after `DesktopPresenter` successfully flushes;
selection cannot begin against an unpresented desired window.

Snapshot replacement, incompatible delta/source identity, resize/reflow, screen switch, reconnect,
history gap/rebase, viewport navigation, or child mouse activation invalidates selection. A normal
non-copy key or a new ordinary click clears a finalized selection. A successful/unacknowledged copy
keeps the highlight.

### 6.2 One pointer router

The current scattered mouse conditions are replaced by one exhaustive routing decision. Existing
capture wins first, then hit-testing/mode ownership:

1. active gutter drag;
2. active or cancelled selection drag;
3. gutter hit;
4. host-owned cached-history wheel/navigation;
5. child mouse reporting;
6. alternate-screen alternate-scroll emulation;
7. live main-history wheel;
8. unmodified left-button local selection in the content rectangle when the child has no mouse mode;
9. ignore/byte-preserving fallback as defined for that event class.

Each input produces one typed outcome and only that owner mutates state or emits child bytes. Gutter,
status row, and out-of-layout coordinates cannot become selection points. In a history viewport the
displayed cache remains host-owned even if the live child later changes modes. Shift-drag remains the
outer terminal's native-selection escape hatch; if an outer terminal reports a modified press anyway,
Zterm does not reinterpret it as an ordinary local selection.

### 6.3 Extraction and overlay

The core linear inclusive cell range is normalized independently of drag direction. If either boundary
lands on a wide continuation, it expands to include the head and continuation. Extraction:

- emits head contents once and skips continuations;
- preserves combining scalars already stored in the head;
- emits one ASCII space for a selected semantic blank;
- joins a selected row boundary when `wrapped`, otherwise inserts `\n` if the range continues;
- appends incrementally under the shared 512 KiB byte cap and fails atomically before a partial glyph
  or clipboard effect can escape.

The compositor clones the selected visible rows and XORs `TerminalStyle.inverse` on every selected
cell, including both cells of a wide glyph. It then composes gutter/status and final cursor as today.
Source rows and cache entries are never mutated. Selection motion participates in the existing paced,
single DEC 2026 presentation transaction, preventing highlight/chrome flicker or an independent draw
loop.

## 7. Nested keyboard gateway and copy action

### 7.1 Child protocol state

`TerminalModes` gains a validated five-bit Kitty keyboard flag value:

- disambiguate escape codes;
- report event types;
- report alternate keys;
- report all keys as escape codes;
- report associated text.

Alacritty's existing keyboard support is enabled. Ingress admits only structurally valid Kitty
set/push/pop/query CSI-u controls; unrelated CSI-u remains unsupported. Set/push/pop is fed to the sole
engine, while query returns the bounded current-mode reply to the PTY. The flags flow through
projection, checkpoint, snapshot, delta, history rows' surrounding surface state, protobuf, and CLI.
Ingress does not independently track or cap Alacritty's keyboard-stack depth; admitted controls use
the pinned engine's native stack semantics without a parallel defensive state owner.

### 7.2 Outer stack and mode selection

The terminal UI guard pushes exactly one disabled Kitty keyboard stack entry on entry and pops exactly
one on every exit path. `DesktopPresenter` is the sole writer of changes to that top entry.

Desired outer flags are:

```text
child flags != 0                         -> child flags
child flags == 0 and finalized selection -> DISAMBIGUATE_ESCAPE_CODES
                                                | REPORT_EVENT_TYPES
                                                | REPORT_ALTERNATE_KEYS
otherwise                                -> 0 (legacy)
```

Therefore nested TUIs normally receive the protocol they declared, ordinary shells remain legacy,
and Zterm temporarily gains structured Super/Command only while it has something to copy. Event
types are required to distinguish one physical press from repeat/release without a timer heuristic;
alternate keys preserve layout-independent shortcut identity and the shifted/base value needed for
lossless legacy downgrade. Pinned Ghostty drops release events unless event reporting is active and
otherwise represents repeat as press, so disambiguation alone cannot satisfy the exact-once/no-orphan
invariant.

Host input modes are physical effects even when semantic rows are not being repainted. The dedicated
projection contains application cursor (`DECSET/DECRST 1`), application keypad (`ESC =`/`ESC >`),
bracketed paste (`2004`), focus reporting (`1004`), and derived outer Kitty keyboard flags. Mouse
mode/encoding and alternate-scroll are deliberately absent because Zterm routes them itself while its
physical mouse capture remains fixed.

If a live delta arrives, the CLI first validates a candidate surface and builds a compact two-phase
viewport plan: projected live metrics/layout, a metadata-only cache-anchor observation, and the
post-delta presented-source identity. It reconciles a Copy-sized selection candidate against that
identity before deriving outer keyboard flags. The plan neither mutates `ViewportController` nor
clones cached `TerminalSurfaceRow` data.

While history remains pinned, the presenter encodes only projected fields that differ from its last
successful commit into one buffer, then performs one `write_all + flush`. It emits no live row, cursor,
gutter, status, mouse reset/capture, or visual transaction; an unchanged projection does no I/O. A live
full-frame delta uses the same candidates when composing its one visual transaction. Only after either
presenter path succeeds are surface, viewport/cache metadata, selection controller, and presenter
selection/input projection committed together through infallible assignments. A write/flush failure
commits none of those candidates, clears the physical/visual baseline, and lets a later full frame
restore the complete pre-delta semantic state. This prevents an incompatible epoch, viewport, or
history-extent change from retaining flags 7 after it invalidates the selection, without adding a
post-commit second sync. Selection readiness can change only the projection's outer keyboard flags.

### 7.3 Decode, consume, or forward

The bounded `HostInputCodec` adds Kitty CSI-u parsing with key code/alternates, modifiers, event kind,
associated text, and original bytes. It continues to own SGR mouse, paste, Page keys, and raw fallback;
there is no second terminal-output parser.

With a finalized selection, a `c/C` press containing Control or Super (Shift may accompany the platform
binding; Alt/Hyper/Meta may not) triggers extraction and the clipboard sink. That key's repeat and
release events are consumed by one short input lease, so no orphan reaches the child and repeated OS
events do not rewrite the clipboard. Legacy ETX is treated as Ctrl+C only while a selection exists.

Without a selection, Zterm consumes no copy key. When outer flags equal child flags, all non-owned key
events use their original bytes. During the sole `0 -> flags 7` local elevation, a non-copy Kitty
event is converted once to its standard legacy equivalent (including Ctrl characters, Escape/Alt,
cursor/application-cursor, keypad, Page, and function keys), clears selection, and then follows the
existing detach-prefix/live/history routing. Unknown or malformed bounded sequences remain raw rather
than being guessed or silently dropped. A mismatch other than that deliberate child-zero/outer-seven
state is also forwarded raw; the gateway never claims that lost physical encoding information can be
reconstructed.

## 8. Desktop clipboard sink

Both local selection and remote child effects call one presenter-owned operation. It builds exactly:

```text
ESC ] 52 ; c ; <RFC 4648 STANDARD padded Base64> BEL
```

from a validated `TerminalClipboardWrite`, writes one complete buffer, and flushes. BEL is selected for
the widest current terminal compatibility, matching Herdr's fallback. The operation runs on the sole UI
output owner, outside the visual DEC 2026 transaction, and does not change the presenter's frame
baseline. An I/O error is content-free; terminal refusal is unacknowledged and cannot be represented as
success or failure. No shell command, platform executable, native clipboard dependency, prompt, toast,
or allow/deny setting is added.

Android can later reuse the domain value, selection/extraction rules, and ownership semantics while
replacing this final method with `ClipboardManager` and native gestures/rendering.

## 9. Validation strategy

### Core and terminal parser

- clipboard value exact cap/over-cap, empty/NUL/Unicode/tab/newline, and redacted Debug;
- whole/chunked/C1/BEL/ST/cancel OSC 52, exact encoded/decoded limits, strict selectors/Base64/UTF-8,
  overflow containment, latest-in-ingest, and no reply/render residue;
- Kitty keyboard set/push/pop/query, stack/screen/reset behavior, flag projection, and continued
  Alacritty OSC 52 disablement.

### Session, wire, and bridges

- kind 322 registration/conversion/frame cap and malformed attachment/text rejection;
- first-sync drop, active delivery, later resync delivery, observer exclusion, no-controller drop,
  takeover linearization, disconnect/reconnect no replay, latest-wins burst, stalled writer, and Session
  end cleanup;
- remote phase/ID rewrite and same-UID projection, with clipboard content absent from Debug/errors/logs;
- operations proves clipboard bypasses the capacity-eight queue and retains only one pending value.

### CLI pure and integration

- forward/backward/single-cell/empty click selection, source invalidation, cancelled drag capture, live
  and cached history coordinates, gutter/status exclusion, and the full mouse-owner matrix;
- ASCII, interior blanks, CJK wide boundaries, combining text, wrapped/non-wrapped rows, exact cap, and
  atomic overflow;
- overlay ordering, wide-cell highlight, cursor/status/gutter stability, dirty-run output, paced drag,
  and failed-write baseline behavior;
- outer keyboard push/set/pop restoration, fragmented Kitty input, Cmd/Ctrl/Shift modifiers,
  press/repeat/release lease, legacy downgrade, detach prefix, paste, Page keys, and raw unknown input;
- canonical OSC 52 bytes from both a local selection and one daemon event, with no frame-baseline change.

Manual acceptance uses macOS Ghostty for shell/history drag, Cmd+C into an external application, nested
Herdr selection/scrollbar/OSC 52 copy, and terminal cleanup. Linux verifies Ctrl+Shift+C in Ghostty or
another Kitty-keyboard-capable terminal; PTY/CI fixtures verify exact output bytes where a real desktop
clipboard is unavailable.

## 10. Rollback and non-goals

The slices are independently testable: core/parser, transient routing/wire, keyboard gateway, then
selection/composition. Valid child effects remain inert until kind 322 and the CLI sink are connected.
If selection cannot preserve one-owner routing or terminal restoration, release is blocked rather than
shipping an application-name workaround.

This task does not add copy mode, search, rectangular/word/line selection, auto-scroll beyond the
cached viewport, Android UI, clipboard reads, rich/image clipboard, policy configuration, application
allowlists, prompts, or old-version compatibility.
