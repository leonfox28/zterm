# Herdr 0.8.2 and tmux 3.7c terminal-history comparison

## Scope

This comparison answers the product question raised for zterm's remote attachment:
whether scrollback should be delegated to the outer terminal (Ghostty), or remain
owned and rendered by zterm. It also checks how status chrome affects PTY geometry.

Sources are the official Herdr v0.8.2 documentation/source and official tmux 3.7c
documentation/source.

## Summary

| Concern | Herdr v0.8.2 | tmux 3.7c | Implication for zterm |
| --- | --- | --- | --- |
| History owner | Server-owned per-pane terminal history | Server-owned per-pane history, bounded by `history-limit` | Keep history daemon-authoritative; do not delegate it to Ghostty |
| Ordinary wheel on shell output | Scrolls host history | Enters copy mode and scrolls tmux history when mouse support is enabled | A wheel/trackpad gesture should work without first typing a shortcut |
| Full-screen/mouse-aware child | Wheel or Page keys are forwarded according to terminal modes | Wheel is forwarded when alternate screen, pane mode, or child mouse mode owns it | Route by negotiated terminal modes, never by application name |
| Explicit history UI | `prefix+[` copy mode | `C-b [` copy mode | Both modes also provide copy/search; they do not prove that scroll-only zterm needs a separate mode |
| Output while browsing | Process stays live; follows at bottom, then pins after navigation | Process stays live, but the copy-mode display is a cloned/frozen view | Prefer Herdr's live-then-pinned model with a revision-bound cursor |
| Outer terminal scrollback | Not the authoritative pane-history model | Explicitly documented as incomplete and not kept consistent | Retain zterm's outer alternate-screen isolation |
| Status chrome | Tab/status row can be placed at the bottom and occupies its own row | Status is one bottom row by default and is subtracted from client/window height | Reserve one physical row and resize the remote PTY to `rows - 1` |

## Herdr findings

1. Herdr keeps the terminal and its bounded scrollback on the server. Detach and
   reattach restore the recent screen from the live terminal; disk history is a
   separate experimental feature and is off by default.
   - [Session state and restore](https://herdr.dev/docs/session-state/)

2. Direct terminal attachment converts wheel and unmodified PageUp/PageDown input
   into typed scroll requests. The server then selects one of three routes from
   terminal state: host scrollback, child mouse report, or alternate-scroll input.
   It does not identify `vim`, `less`, tmux, or another program by name.
   - [Client input conversion](https://github.com/herdrdev/herdr/blob/v0.8.2/src/client/mod.rs#L173-L228)
   - [Server-side attach routing](https://github.com/herdrdev/herdr/blob/v0.8.2/src/server/headless.rs#L345-L406)
   - [Mode-derived wheel routing](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane/terminal.rs#L1829-L1850)

3. Herdr also provides `prefix+[` copy mode. The pane process is not paused:
   output follows while the view is at the bottom, and the view stays pinned once
   the user navigates into history. This is a useful network-attachment model
   because live execution and a stable historical viewport remain separate.
   - [Keyboard and copy mode](https://herdr.dev/docs/keyboard/)

4. Herdr can put its desktop tab row at the bottom and place right-aligned status
   segments in that row. The layout reserves the row first and lays out/resizes
   pane PTYs inside the remaining rectangle; it does not paint status text over
   the last row owned by the child.
   - [UI and status configuration](https://herdr.dev/docs/configuration/#ui-and-sidebar)
   - [Reserved tab-row layout](https://github.com/herdrdev/herdr/blob/v0.8.2/src/ui.rs#L185-L213)

## tmux findings

1. tmux keeps a bounded history for each pane (`history-limit`, 2,000 lines by
   default in 3.7c). `C-b [` enters copy mode, which provides keyboard navigation,
   paging, search, selection, and copying.
   - [Getting Started: copy mode and history](https://github.com/tmux/tmux/wiki/Getting-Started#copy-and-paste)
   - [3.7c `history-limit`](https://github.com/tmux/tmux/blob/3.7c/options-table.c#L677-L687)

2. With tmux mouse support enabled, the default `WheelUpPane` binding forwards
   the wheel when the child is on the alternate screen, already in a pane mode,
   or has requested mouse input. Otherwise it enters copy mode automatically.
   Once in copy mode, wheel and PageUp/PageDown move the tmux-owned history.
   - [3.7c default mouse bindings](https://github.com/tmux/tmux/blob/3.7c/key-bindings.c#L445-L454)
   - [3.7c copy-mode wheel and page bindings](https://github.com/tmux/tmux/blob/3.7c/key-bindings.c#L528-L549)

3. tmux copy mode deliberately clones the pane screen into a backing screen at
   entry, so its visible historical view is frozen while the child process keeps
   running. This differs from Herdr's live-at-bottom/pinned-in-history behavior.
   - [3.7c copy-mode backing screen](https://github.com/tmux/tmux/blob/3.7c/window-copy.c#L237-L250)
   - [3.7c copy-mode initialization](https://github.com/tmux/tmux/blob/3.7c/window-copy.c#L466-L512)

4. tmux explicitly says it does not try to keep outer-terminal scrollback
   consistent because it would be incomplete with windows and panes. Its status
   line is one bottom row by default; client sizing subtracts the status-line
   height before sizing windows and panes.
   - [FAQ: terminal scrollback is not authoritative](https://github.com/tmux/tmux/wiki/FAQ#i-want-to-use-the-mouse-to-select-panes-but-the-terminal-to-copy-how)
   - [3.7c status defaults](https://github.com/tmux/tmux/blob/3.7c/options-table.c#L831-L837)
   - [3.7c status-aware resize](https://github.com/tmux/tmux/blob/3.7c/resize.c#L181-L188)

## Recommended zterm behavior

Adopt the shared ownership model, with Herdr's live-view semantics:

1. Keep zterm's outer alternate screen and daemon-authoritative bounded history.
2. In normal live mode, make wheel/trackpad and unmodified PageUp/PageDown browse
   zterm history automatically for ordinary main-screen transcript output.
3. If the authoritative terminal modes say the child owns mouse input,
   alternate-scroll, or full-screen Page keys, forward the input unchanged. This
   preserves nested tmux/Herdr, editors, pagers, and TUIs without program-name
   special cases.
4. Do not add an explicit zterm history-mode shortcut in this task. Wheel,
   trackpad, and PageUp/PageDown already cover scroll-only history browsing.
   A separate mode becomes justified only if zterm later adds selection/search,
   or needs an explicit force-browse escape hatch while a full-screen child owns
   the normal scrolling inputs.
5. While at the bottom, follow live output. After moving into history, keep the
   viewport pinned to a revision/epoch-bound anchor while the Session continues.
   Scrolling back to the bottom resumes live display. Normal key or paste input
   returns to the live bottom before being forwarded, matching ordinary terminal
   scrollback rather than creating a hidden modal state.
6. Reserve one physical bottom row for zterm status and size the remote PTY to
   the remaining rows. Never overlay status bytes on the child-owned terminal.
   Keep the persistent row limited to the three confirmed connection fields;
   do not add a history-position field in this task.

The rejected shortcut is to leave the outer alternate screen and rely on Ghostty
scrollback. It would make behavior terminal-dependent, mix local attachment
rendering with remote history, make reconnect/history gaps ambiguous, and conflict
with a stable zterm-owned status row.
