# Copy shortcut and nested keyboard protocol research

## Problem

Once Zterm owns a semantic selection, the outer terminal no longer knows that a copy action is
performable. The design therefore needs a reliable way to receive the desktop copy shortcut without
changing the keyboard bytes observed by an ordinary shell or a nested TUI.

## Evidence

- Ghostty's current default copy binding is `Super+C` on macOS and `Ctrl+Shift+C` elsewhere. It is
  marked `performable`, so with no Ghostty-native selection the binding falls through to terminal
  key encoding. In legacy mode Ghostty deliberately emits no bytes for a macOS Super-modified text
  key. Merely drawing a Zterm-local highlight therefore cannot make `Cmd+C` reach Zterm.
  <https://github.com/ghostty-org/ghostty/blob/c81f0b26871c7fbbe2fc35549fdad1f64ed29094/src/config/Config.zig#L6628-L6641>
  <https://github.com/ghostty-org/ghostty/blob/c81f0b26871c7fbbe2fc35549fdad1f64ed29094/src/input/key_encode.zig#L536-L547>
- Kitty's keyboard protocol provides a stack-scoped enhancement mode, structured modifiers
  (including Super/Command), event kinds, and alternate keys. Applications can push their mode on
  entry and pop it on exit rather than overwriting the caller's state.
  <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>
- `alacritty_terminal 0.26.0`, already Zterm's sole terminal engine, implements the Kitty keyboard
  set/push/pop/query state and exposes the five flags in `TermMode`. Zterm currently disables that
  engine feature and rejects every CSI-u control before it reaches the engine, so nested programs'
  declarations are neither tracked nor projected.
  <https://github.com/alacritty/alacritty/blob/v0.26.0/alacritty_terminal/src/term/mod.rs>
- Herdr 0.8.2 pushes `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES |
  REPORT_ALTERNATE_KEYS` at its outer boundary, parses input into a structured key, consumes
  Ctrl/Cmd+C only when a finalized selection exists, suppresses that key's repeats/releases, and
  otherwise encodes the key for the child-declared protocol. This is an application-independent
  terminal gateway, not a Herdr-name exception.
  <https://github.com/herdrdev/herdr/blob/v0.8.2/src/terminal_modes.rs>
  <https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/input/clipboard.rs>

## Decision

1. Enable Alacritty's existing Kitty keyboard state machine and admit only the protocol's bounded
   set/push/pop/query controls through Zterm ingress. Project the validated five-bit child keyboard
   mode through semantic snapshot/delta/history state.
2. Scope one outer keyboard-stack entry to the Zterm UI guard. Normally mirror a non-zero child
   mode exactly. When the child mode is zero, elevate the outer mode to
   `DISAMBIGUATE_ESCAPE_CODES | REPORT_EVENT_TYPES | REPORT_ALTERNATE_KEYS` only while a non-empty
   finalized Zterm selection exists. Disambiguation makes Ghostty encode Cmd+C; event types make
   repeat/release observable; alternate keys preserve layout-independent shortcut identity and the
   shifted/base value required by legacy downgrade.
3. Extend the sole host-input codec to parse bounded Kitty CSI-u events. Consume a matching
   Ctrl/Super+C press only when Zterm owns a finalized selection, and consume the corresponding
   repeat/release lease so no orphan event reaches the child.
4. If outer and child keyboard modes are equal, preserve the original bytes. The only intentional
   mismatch is the temporary zero-to-flags-7 elevation for a local selection; non-copy events
   in that state are converted once to their legacy equivalent before forwarding and clear the
   selection. Unknown or malformed bounded input remains byte-preserving rather than being guessed.
5. Pop exactly Zterm's stack entry on every normal, error, signal, cancellation, and panic exit.
   Do not globally leave an enhanced mode enabled and do not recognize terminal/process names.

Pinned Ghostty's encoder proves that release is dropped without `REPORT_EVENT_TYPES` and repeat is
otherwise indistinguishable from press. The Kitty protocol defines alternate keys specifically for
shortcut matching; Ghostty emits the shifted/base values only when that bit is enabled. Thus flags 7,
not disambiguation alone, are the smallest set that satisfies one-copy-per-physical-actuation and
lossless zero-mode downgrade without a timer heuristic.

This is the minimum architecture that makes the requested Cmd+C behavior real on Ghostty while
preserving nested TUI keyboard negotiation. Parsing only the literal Cmd+C byte sequence would be a
terminal-specific patch and would regress other modified keys as soon as Zterm enabled enhancement.
