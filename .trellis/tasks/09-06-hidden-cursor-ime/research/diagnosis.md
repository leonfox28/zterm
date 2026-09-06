# Hidden cursor IME diagnosis

Implementation follow-up: the user confirmed the candidate fixes the original
scenario on 2026-09-06 (“测试了没有问题了”). The investigation below records what
was known before implementation; final validation is in `validation.md`.

## Observable failure and evidence boundary

User report: the Chinese IME preedit/candidate UI appears at the outer terminal's
top-left for Zterm → Herdr → Pi, although the editor is near the bottom. The
same nested shell and Codex work; both agents work in Herdr without Zterm.
The screenshot shows `zterm connect dev` and a `direct` route. Direct transport
still passes through Zterm's semantic model and desktop presentation.

Local observations (2026-09-06): Zterm 0.1.22, Herdr 0.8.2, Pi 0.85.1;
checkout commit `68eab39`. Remote `dev` versions/config and the outer terminal's
identity/version remain unverified. No user session was restarted or modified.

## Root-cause classification

**Local implementation defect**, deterministically confirmed in Zterm's frame
composition. The complete end-to-end native IME outcome still needs a smoke test.

Existing contracts already have independent cursor coordinates and visibility,
one semantic compositor, and one desktop presenter. The terminal color contract
already requires preserving CUP for IME when a software cursor hides the native
cursor (`.trellis/spec/backend/terminal-colors.md:63`). The analogous child-hidden
path incorrectly discards that position. No missing protocol field, second
cursor owner, or agent-specific mechanism is required to represent the state.

Related task evidence agrees with this boundary:
`.trellis/tasks/archive/2026-09/09-06-zterm-themes/design.md:209` requires a
software cursor to preserve CUP for IME, while its
`research/color-frontend-architecture.md:170` excludes hidden cursors from
software overlays. Position preservation must not accidentally enable an overlay.

## Causal path

1. Pi 0.85.1 uses a software editor cursor by default. Installed
   `@earendil-works/pi-tui/dist/tui-main-screen.js:563` implements
   `positionHardwareCursor`: move to the editor's cursor, then show/hide based
   on `getShowHardwareCursor()`. Hidden still has a meaningful position.
   Installed Pi `dist/core/settings-manager.js:943` defaults to
   `settings.showHardwareCursor ?? process.env.PI_HARDWARE_CURSOR === "1"`.
2. Herdr v0.8.2 `src/ui/tab_surface.rs:133` preserves `cursor.x/y` while deciding
   visibility independently. `src/protocol/render_ansi.rs:548` resolves hidden
   coordinates; `write_host_cursor_state` emits CUP even for a hidden cursor.
   Herdr repeats the anchor after synchronized output on non-Windows targets
   (`render_ansi.rs:497` and `:626`).
3. Zterm's host terminal projection preserves row/column independently of
   `SHOW_CURSOR`: `crates/terminal/src/projection.rs:128` and `:142`.
   Snapshot/delta construction carries the latest cursor
   (`crates/terminal/src/model.rs:302`), so the domain can represent a hidden,
   moving cursor without adding wire state.
4. `crates/cli/src/terminal_ui/composition.rs:273` requires
   `surface.cursor.visible` before retaining coordinates. Otherwise it creates
   `(row: 0, column: 0, visible: false)`. This conflates hidden-with-valid-anchor
   with history/inactive/out-of-bounds presentation.
5. `crates/cli/src/terminal_ui/ansi_presenter.rs:227` unconditionally writes CUP
   using the composed coordinates, then `?25h`/`?25l`. The hidden input position
   therefore becomes `ESC [ 1 ; 1 H` at the real terminal. Hidden cursor-only
   movement can also collapse to the same composed baseline and be suppressed.

Primary upstream sources:

- [Pi v0.85.1 main-screen renderer](https://github.com/badlogic/pi-mono/blob/v0.85.1/packages/tui/src/tui-main-screen.ts)
- [Pi terminal setup](https://github.com/badlogic/pi-mono/blob/v0.85.1/packages/coding-agent/docs/terminal-setup.md)
- [Herdr v0.8.2 cursor composition](https://github.com/ogulcancelik/herdr/blob/v0.8.2/src/ui/tab_surface.rs)
- [Herdr v0.8.2 ANSI presentation](https://github.com/ogulcancelik/herdr/blob/v0.8.2/src/protocol/render_ansi.rs)

## Application-neutral probe

Run:

```sh
python3 .trellis/tasks/09-06-hidden-cursor-ime/research/probe_cursor.py
```

The script extracts the **unmodified** production cursor composition and cursor
ANSI writer blocks and compiles them with minimal Rust surrounding types. It
changes no product source and does not claim to exercise transport, the complete
compositor, or a native IME. Observed output before correction (zero-based
coordinates; CUP uses one-based coordinates):

```text
visible: child=(34,27,true) outer=(34,27,true) ANSI="\u{1b}[35;28H\u{1b}[?25h"
hidden: child=(34,27,false) outer=(0,0,false) ANSI="\u{1b}[1;1H\u{1b}[?25l"
hidden-moved: child=(35,31,false) outer=(0,0,false) ANSI="\u{1b}[1;1H\u{1b}[?25l"
history: child=(34,27,true) outer=(0,0,false) ANSI="\u{1b}[1;1H\u{1b}[?25l"
reconnecting: child=(34,27,true) outer=(0,0,false) ANSI="\u{1b}[1;1H\u{1b}[?25l"
```

The implementation regression must use the real composed/presented path;
this investigation probe is not a replacement for that test.

## Matrix explanation and rejected alternatives

- Pi's hidden-but-positioned cursor directly triggers the faulty branch. A
  visible shell cursor avoids it. Codex succeeding is consistent with different
  effective cursor visibility, but its exact state was not captured and is
  **inference**, not a verified claim about every Codex version.
- Removing Zterm removes this coordinate reset. Herdr's explicit anchor handling
  supports that explanation; the user's report supplies the native UI comparison.
- Encoding/font/input-method conversion does not explain the explicit origin CUP.
- Network latency and Herdr sidebar offsets cannot explain an unconditional
  reset of both coordinates to zero; no transport change is justified by this
  evidence.
- Agent recognition or forcing a visible cursor would bypass this branch while
  leaving the general hidden-position contract broken. Fix the owning compositor.

## Remaining compatibility observation

Herdr's post-sync anchor repeat is relevant: some native IMEs may not observe
cursor updates inside DEC 2026. Zterm's presenter currently ends with
`HOST_SYNC_END` and emits no post-sync anchor. Position preservation is a confirmed
necessary fix; it is not yet proven sufficient for the user's GUI. Verify with
the actual outer terminal after the local correction. If correct coordinates
still do not move the candidate window, capture that distinction and reconverge
the design before changing the presenter's synchronization contract.

## Optional diagnostic workaround

`PI_HARDWARE_CURSOR=1 pi` enables Pi's native cursor when there is no explicit
`showHardwareCursor` setting overriding it. Pi `/settings` also exposes the
hardware cursor setting. This is an A/B diagnostic, not the product correction,
and has not been applied to the user's settings or tested on the remote host.
