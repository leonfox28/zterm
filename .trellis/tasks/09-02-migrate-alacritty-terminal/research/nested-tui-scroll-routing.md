# Nested TUI Scroll Routing

## Scope and revisions

- Zterm source reviewed at `db9daa95c9698dfbecc0033edd847fb30b9e1c27`; the implementation branch was
  fast-forwarded to released `main` `bce1d57d8bd91b5c0e58bcdf422d899cedcd7fac` before product edits, with no
  terminal-routing code drift between those revisions.
- Herdr source reviewed at the task-pinned
  `cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6`, with current remote HEAD
  `da49cb8a9dac6facc8f77671376914ee0ef47291` checked for relevant drift.
- Pi source reviewed at `badlogic/pi-mono`
  `e266507b606b9552fa277252644054afd4384b11`.

## Ownership rule

Nested viewport state is safe only when one wheel report has exactly one owner. Process names must
not participate in routing. The authoritative child terminal modes already provide the contract:

1. While Zterm is displaying its own retained-history viewport, Zterm owns wheel/Page navigation
   until it reaches the live bottom or an ordinary input resumes live state.
2. At live bottom, a child mouse-reporting mode owns wheel input. Zterm must not change its own
   offset and must forward exactly one encoded wheel report.
3. At live bottom, alternate screen plus alternate-scroll owns the wheel. Zterm must not change its
   own offset and must forward exactly one cursor-key sequence.
4. Otherwise the host owns the wheel and changes only the attachment-local Zterm viewport. It must
   not write wheel bytes to the child PTY or mutate the canonical terminal model.

The approved visible scrollbar adds one geometry-scoped exception, not an application exception:
on main screen its reserved gutter column is outside the child PTY rectangle, so wheel/click/drag in
that column belongs to Zterm chrome even when the child reports mouse. Zterm must never clamp that
coordinate into the child's last column. The alternate screen reclaims the column and removes the
hit target, after which ordinary child-mode routing applies across the full width.

Host capture and child ownership are different layers. Zterm may ask the physical Ghostty/kitty
terminal for all-motion SGR reports so it can observe input, while retaining the child-requested
mode separately and filtering/re-encoding events to that mode. Reasserting host capture after a
snapshot/delta does not enable a new mode inside the nested application.

## Herdr inside Zterm

Herdr's default mouse UI enables host mouse capture. Its terminal routing distinguishes
`MouseReport`, `AlternateScroll`, and `HostScroll`; when its own child pane requests mouse input,
Herdr forwards the report again to that pane. Therefore the expected nested path is:

```text
physical terminal wheel
  -> Zterm host capture
  -> one wheel report to Herdr
  -> Herdr chooses its own UI / pane history / inner child
```

Zterm must remain at live bottom on the child-owned branch. It must not also move its retained
history, multiply the report, or inspect the executable name. If Herdr is configured with
`ui.mouse_capture = false`, it has explicitly declined child mouse ownership; Zterm host history is
then the expected wheel owner.

Primary references:

- [Herdr pane wheel routing](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/pane/terminal.rs)
- [Herdr terminal attach input](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/server/pane_input.rs)
- [Herdr host mouse setup](https://github.com/herdrdev/herdr/blob/cc88b3b8e5bb9f7d9f23ed6ae85a52fd7b5b9ed6/src/client/terminal_setup.rs)

## Pi inside Zterm

Pi now has two materially different renderers:

- `TuiMainScreen` writes into the terminal main buffer and intentionally delegates scrollback to
  the terminal. In this mode Pi has no application-owned terminal viewport for Zterm to target;
  Zterm host-history scrolling is correct.
- `TuiAltScreen` enters DEC alternate screen (`1049`), enables SGR mouse reporting (`1006`) with
  button/all-motion tracking (`1000/1002` or `1000/1002/1003`), and owns one or more `ScrollView`
  instances plus interactive scrollbars. In this mode Zterm must forward exactly one wheel report
  and leave Pi to apply its own `wheelScrollLines` value. At the reviewed revision Pi defaults that
  internal value to one line.

The expected fullscreen path is therefore:

```text
physical terminal wheel
  -> Zterm host capture
  -> one SGR wheel report to Pi fullscreen
  -> Pi moves the ScrollView under the pointer by its own configured amount
```

Pi's default main-screen and optional fullscreen modes prove why Zterm must route from terminal
modes rather than from a list of known application names.

Primary references:

- [Pi interchangeable main/alternate renderers](https://github.com/badlogic/pi-mono/blob/e266507b606b9552fa277252644054afd4384b11/packages/tui/README.md)
- [Pi alternate-screen mouse declarations and wheel routing](https://github.com/badlogic/pi-mono/blob/e266507b606b9552fa277252644054afd4384b11/packages/tui/src/tui-alt-screen.ts)
- [Pi coding-agent renderer selection](https://github.com/badlogic/pi-mono/blob/e266507b606b9552fa277252644054afd4384b11/packages/coding-agent/src/modes/interactive/interactive-mode.ts)

## Compatibility limit

If a nested application expects wheel input but declares neither mouse reporting nor the supported
alternate-scroll contract, no outer terminal or multiplexer can infer that private intent
reliably. Zterm should treat that as host history, exactly as a normal terminal would, and must not
add Herdr/Pi/tmux/process-name special cases. A future explicit force-host/force-child modifier may
be considered separately if real compatibility evidence requires it.

## Required tests

- Main screen + no child mouse: one wheel report changes only attachment-local Zterm offset.
- Herdr/Pi-style alternate screen + SGR button/all-motion mouse: one host wheel report produces
  exactly one child report and zero Zterm offset movement.
- Alternate screen + alternate-scroll + no mouse reporting: one host wheel report produces exactly
  one cursor key and zero Zterm offset movement.
- Child mode exit returns wheel ownership to Zterm without losing host capture.
- Scrolled Zterm history remains pinned when background output changes child modes; ordinary input
  resumes live before it reaches the child.
- No routing branch reads process name, `TERM` identity, tmux markers, or application-specific text.
