# Terminal-theme color options for the zterm status bar

## Finding

There is no portable terminal-theme color with the semantic meaning "status
bar". Ghostty themes normally define the default background and foreground,
cursor and selection colors, plus palette indices 0–255. Applications may use
those colors, but the palette indices are not semantic status roles.

- [Ghostty color themes](https://ghostty.org/docs/features/theme)
- [Ghostty color concepts](https://ghostty.org/docs/vt/concepts/colors)
- [Ghostty VT color reference](https://ghostty.org/docs/vt/reference)

Ghostty also supports style-specific special colors, including reversed cells,
but this is attached to the generic reverse-video style rather than a dedicated
status-bar theme slot. When no special reverse color is supplied, ordinary
palette/default colors apply.

## Options considered

### Reverse the default foreground and background — recommended

Render the complete physical status row with SGR reverse video, including
trailing spaces, then reset the style before returning to child rendering.

- Uses the terminal theme's own default foreground/background pair.
- Works in dark and light themes without querying Ghostty or hard-coding RGB.
- Preserves the project's generic Unix ANSI/TTY boundary.
- Has the same contrast as normal terminal text, but can appear visually strong
  because a dark theme's light foreground becomes the row background.

### Fixed ANSI palette colors

For example, an ANSI blue or bright-black background would still be remapped by
the terminal theme. It is not a semantic contract: themes are free to choose
palette values with weak contrast for the selected foreground/background pair.

### Query theme colors and derive a custom RGB background

Ghostty supports querying dynamic foreground/background and palette colors, but
the response is asynchronous terminal input. Adding a host-color negotiation
state machine for one status row would expand protocol, timeout, terminal-input,
and restoration complexity without improving the core user outcome.

### Selection/highlight colors

Selection colors are terminal-UI state, not a portable application status role.
Ghostty's documented theme exposes selection colors, but it does not provide a
portable status application with a synchronous way to request and reuse them.

## Confirmed MVP contract

- The row contains exactly three left-to-right fields:
  `<device> | <direct|relay|--> | <integer ms|-->`.
- The complete row has a visible background using reverse video; unused cells to
  the right are filled with spaces under the same style.
- No hard-coded RGB, Ghostty-only protocol, user color configuration, icons, or
  per-field colors in this task.
- ANSI style must be reset/restored at the row boundary so the status style can
  neither inherit child SGR state nor leak back into child rendering.
