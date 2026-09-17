# Notification forwarding: initial findings

Date: 2026-09-17. Baseline: `main` at `71f41d3`.
This is planning evidence, not an approved design.

## Existing behavior and reusable boundaries

| Boundary | Evidence | Implication |
| --- | --- | --- |
| PTY ingress | `crates/terminal/src/ingress.rs:651` handles titles, icon names and OSC 52; other OSC commands are classified as unsupported. | OSC 9, OSC 99 and OSC 777 currently produce no forwarded notification. |
| Bell | `crates/terminal/src/ingress.rs:356` emits `AudibleBell`; `crates/core/src/terminal.rs:946` defines bounded side events. | Recognition exists, but recognition alone is not outer-terminal delivery. |
| Model update consumption | `crates/daemon/src/terminal_driver.rs:393` handles replies and the transient host effect, not `update.events`. | Adding a side-event variant alone would not complete delivery. |
| Host-effect value | `crates/core/src/terminal.rs:96` currently contains only `ClipboardWrite`. | A semantic, renderer-neutral effect boundary already exists. |
| Controller targeting | `crates/daemon/src/terminal_driver.rs:134` owns a latest-only pending effect targeted at publication time. | Target changes clear pending data; no controller means no retained effect. Notification queue/coalescing policy still needs a deliberate decision. |
| Session wire | `crates/daemon/src/session_wire.rs:1490` consumes effects; `:1539` writes a typed clipboard message. | Notifications need explicit protocol and attachment handling. |
| Shared client | `crates/client/src/session.rs:1110`, `crates/client/src/view.rs:441` validate the clipboard event and isolate its pending slot from lifecycle events. | Notifications should not block PTY drain or consume unbounded client memory. |
| Desktop output | `crates/cli/src/terminal_ui/session.rs:704`, `crates/cli/src/terminal_ui/ansi_presenter.rs:319` execute clipboard effects through the presenter. | Physical notification escape encoding belongs at the final presenter. |

The relevant existing contracts are
`.trellis/spec/backend/terminal-model.md`,
`.trellis/spec/backend/terminal-driver.md`,
`.trellis/spec/backend/local-daemon-ipc.md:335`, and
`.trellis/spec/guides/cross-layer-thinking-guide.md`.

The clipboard path is useful architectural evidence. Its single replaceable
slot must not be copied blindly: two notifications can represent distinct
events, and notification/clipboard traffic must not overwrite each other by
accident. A new effect must remain outside screen snapshot/history replay.

## Protocol evidence

- **OSC 9** carries a simple notification message. OSC 9 also contains progress
  subcommands, so accepting every OSC 9 payload as a notification would conflate
  different operations. Source: [iTerm2 proprietary escape codes](https://iterm2.com/documentation-escape-codes.html).
- **OSC 777** supports the `notify` extension with title and body. WezTerm
  documents both this form and OSC 9, with BEL or ST termination. Source:
  [WezTerm escape sequences](https://wezterm.org/escape-sequences.html#operating-system-command-sequences).
- **Ghostty compatibility** includes OSC 9 and OSC 777 when its desktop
  notification setting permits them; its OSC 9 reference documents collisions
  with ConEmu numeric subcommands. Sources:
  [Ghostty configuration](https://ghostty.org/docs/config/reference#desktop-notifications)
  and [Ghostty OSC 9](https://ghostty.org/docs/vt/osc/9).
- **Kitty OSC 99** includes title/body payloads, multipart requests, identifiers,
  support queries, notification updates, and optional activation/close reports.
  A full implementation therefore needs both output forwarding and correctly
  routed replies; it cannot be described as a single text-only output effect.
  Source: [Kitty desktop notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/).

OSC 99 can also carry a simple, complete notification without opting into reply
reporting. The same protocol's advanced interactions are optional features;
basic output support and full bidirectional compatibility are different scope
choices. Kitty's `kitten notify` is one concrete producer, and scripts can emit
the sequence directly. A zterm use case is a build/task completion message
traveling from the hosted program to the user's supporting outer terminal.
This is a possible integration, not evidence that any particular AI coding tool
already emits OSC 99. Source: [Kitty notify tool](https://sw.kovidgoyal.net/kitty/kittens/notify/).

Documentation support is not a completed runtime interoperability test. Actual
desktop display also depends on the outer terminal and its notification settings.

## Scope convergence

The user subsequently selected Ghostty, ordinary OSC 9 and OSC 777 only, and no
buffering/catch-up while disconnected. OSC 99 and standalone BEL forwarding are
outside this version. The authoritative requirements are in `../prd.md`.

## Design follow-through

The proposed `../design.md` assigns independent bounded notification queues,
current-controller targeting, no replay, protocol-preserving canonical output,
and OSC 9 subcommand rejection to the existing owners. Its validation plan
includes UTF-8/C1 framing, which is important for Chinese notification text.

Additional primary evidence: [Ghostty's ConEmu extension reference](https://ghostty.org/docs/vt/osc/conemu)
documents OSC 9;4 progress commands as a separate action from notifications.

Ghostty's current [OSC 777 parser](https://github.com/ghostty-org/ghostty/blob/main/src/terminal/osc/parsers/rxvt_extension.zig)
requires the `notify;title;body` separators, takes the title up to its next
semicolon, and retains the full remaining body. The forwarding design therefore
keeps the body separator even for empty body text and preserves body semicolons.

No product code or implementation tests were changed during this exploration.
