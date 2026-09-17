# Herdr notification compatibility

Checked on 2026-09-17. The installed executable reports `herdr 0.9.1`.
The user's local configuration selects `ui.toast.delivery = "terminal"`.
Only notification settings were inspected; no settings were changed and no
notification was triggered.

## Version-matched source findings

Primary source:
[Herdr v0.9.1 terminal_notify.rs](https://github.com/herdrdev/herdr/blob/v0.9.1/src/terminal_notify.rs).

- Lines 11-31: backend selection checks `TERM_PROGRAM`, `KITTY_WINDOW_ID`,
  and `TERM`. Ghostty is selected by `TERM_PROGRAM=ghostty` or
  `TERM=xterm-ghostty`.
- Lines 34-55: Ghostty, iTerm2 and WezTerm use OSC 9; Kitty uses OSC 99.
  The resulting bytes are written to stdout and flushed.
- Lines 65-70: the OSC 9 message joins non-empty body text with the title as
  `title: body`, then uses `ESC ] 9 ; message ESC \\` framing.
- Lines 73-80: Kitty's OSC 99 branch emits a simple title request, or two
  requests for title/body using `i=1`, `d=0` and `p=body` metadata.
- Lines 46-49 and 95-105: when `TMUX` is present, Herdr additionally wraps the
  sequence in a tmux DCS passthrough wrapper. This is a separate nesting case;
  direct Herdr-in-zterm support must not be claimed as generic DCS passthrough.

The [client notification consumer](https://github.com/herdrdev/herdr/blob/v0.9.1/src/client/notifications.rs)
calls the terminal notifier for terminal delivery and the platform desktop
notifier for system delivery. These are distinct output paths.

## Delivery modes

The [official configuration documentation](https://herdr.dev/docs/configuration/#notifications)
distinguishes:

| Setting | Delivery owner |
| --- | --- |
| `herdr` | Herdr's own in-app toast UI |
| `terminal` | The supporting outer terminal, via terminal control sequences |
| `system` | The operating-system notification service where the Herdr client runs |
| `off` | No popup |

For the current Ghostty target, terminal delivery uses OSC 9. Ghostty also
documents OSC 777 support under its
[`desktop-notifications` setting](https://ghostty.org/docs/config/reference#desktop-notifications).
Actual popup display depends on terminal/OS notification policy.

## Implications for this task

The intended path for the user's combination is:

```text
Herdr client -> OSC 9 in PTY output -> zterm -> OSC 9 on outer stdout
             -> Ghostty -> desktop notification
```

OSC 99 is not required for Herdr's Ghostty backend. OSC 9 forwarding therefore
addresses the central first-version compatibility case.

Application detection is a separate technical acceptance detail. Zterm sets
`TERM=xterm-256color` at `crates/platform/src/pty.rs:732`; the same builder does
not explicitly set `TERM_PROGRAM`. Herdr emits nothing if its backend detector
finds no supported terminal. Do not assume that a local or remote Herdr process
will detect Ghostty merely because the viewer runs in Ghostty. During design,
verify the actual hosted environment and define honest compatibility behavior
without changing Zterm's TERM profile to impersonate Ghostty or adding a
Herdr-specific program branch.

No runtime notification delivery through zterm has been tested yet.
