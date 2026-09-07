# UI completeness review — 2026-09-06

Status: accepted by the user after prototype v7 ("按你说的来"). The current
PRD owns the resulting requirements; prototype v8 illustrates these additions.

## Existing behavior that needs complete state illustrations

CORE-3, PAIR-5/6, SESSION-1/3/5, and LIFE-3 already require connection and
validation feedback. Cover connecting/reconnecting, unreachable/revoked hosts,
empty Sessions, ended Sessions, camera permission denial, invalid/expired
credentials, no QR in an image, and canceled system selection. These are
states of the existing routes/dialogs, not extra feature pages. Keep the last
readable terminal during interruption and a short actionable retry/error
message. Preserve disabled input until synchronization finishes.

INPUT-1/2 and design.md already cover system-keyboard paste, one-shot visible
Ctrl/Alt modifiers, Chinese composition, and Back handling. They do not require
more shortcut buttons or a restored terminal overflow menu.

## Accepted decisions

1. **Input continuity after scrolling.** The previous SCROLL-3/design draft consumed the first key/paste to return from browsing/selection, without
   sending it. This is explicit existing draft behavior, not an implementation
   bug. It can feel like losing the first character in the user's continuous
   terminal model. The accepted revision preserves the triggering complete input unit
   within a fixed bound during healthy same-attachment return, forwarding once
   only after synchronization. True reconnect, navigation, takeover, or an
   invalidated attachment must discard it; never queue arbitrary offline input
   for replay. PRD SCROLL-3 now owns this change.
   Evidence for reuse: `.trellis/spec/backend/local-daemon-ipc.md:543-554`
   documents the desktop's bounded resume-input owner and acknowledgement/
   Active fence; `crates/cli/src/terminal_ui.rs:3726-3759` exercises whole-paste
   retention. This does not establish a working Android adapter.
2. **Terminal readability.** Add a terminal font-size option with an immediate
   preview, and explicitly support landscape to show more columns. Keep font
   family selection and gesture zoom out of this proposal. Grid changes use the
   existing resize owner and preserve Session identity; font-size defaults need emulator/phone readability checks. UI-9 starts with
   12/14/16, default 12, independently persisted from language and theme.
3. **Saved-device removal.** Long-press a home device card to remove its saved
   record, keeping ordinary tap as direct connection. Describe it as local
   removal, distinct from Session Delete and host-side authorization revocation;
   it must not close remote Sessions. Removing the matching recent record must
   also remove its resume card. CORE-4 owns the accepted capability; it is not an existing host-removal
   API claim.

## Accepted first-connection navigation

CORE-1 and design.md now resolve first connection without a previous ID and
with multiple Sessions by entering the terminal container and automatically
expanding its existing title Session list.
A sole available Session can be the target; no Sessions shows explicit New
Session. Occupancy still requires explicit takeover, and connecting never
creates a Session. This avoids a new route or choosing an arbitrary Session,
at the cost of a selection tap on first multi-Session connection. The user accepted this rule. A stale previously recorded ID still shows
ended and does not authorize automatic retargeting.

The task remains planning. These accepted requirements and prototype changes
do not claim product implementation or Android runtime validation.
