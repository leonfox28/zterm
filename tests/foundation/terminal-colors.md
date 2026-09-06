# Color protocol smoke

Run `python3 tests/foundation/terminal-colors.py` first in the outer terminal,
then in a Session served by matching new CLI/daemon binaries. For remote tests,
copy this fixture to the test host and run it inside the remote Session. Use
isolated test state; do not restart a production daemon just to run this fixture.

The fixture issues read-only queries for all 256 palette entries, default and
special colors, appearance, and mode 2031, followed by a DSR boundary. It waits
at most 250 ms and retains at most 64 KiB. Keep the keyboard idle during capture.
It prints escaped response bytes and swatches; it never writes physical palette,
cursor, selection, or appearance settings. A missing response is reported as
missing, not inferred from a terminal brand or background brightness.

Compare in both light and dark host configurations:

1. Palette swatches and default FG/BG should match the outer observations.
   Literal RGB orange must remain unchanged across palette changes.
2. Change the host theme while ZTerm remains attached. If the host supports
   appearance notifications, repeat the fixture and check updated observations,
   including a palette change that keeps the same dark/light preference.
3. Scroll into retained history and change the palette from the child. Pinned
   text and selection stay in place while its indexed/default colors repaint.
4. Inside a disposable ZTerm Session, exercise a saved palette and override:

   ```sh
   printf '\033[#P\033]4;1;rgb:ff/80/00\007\033]11;rgb:20/20/20\007'
   printf '\033[31mIndexed override\033[0m\n'
   python3 tests/foundation/terminal-colors.py
   printf '\033[#Q\033]104\007\033]110\007\033]111\007'
   ```

   Run setters only inside the disposable ZTerm Session; directly in the outer
   terminal they would change its palette. Automated stream-order assertions
   live in `crates/terminal/tests/color_protocol.rs`.
5. Exercise explicit cursor and cursor-text colors using OSC 21 in that Session;
   verify a steady software block over narrow/wide/combining glyphs. Reset the
   keys and verify the native cursor returns. Check selection colors separately.
6. Detach/reconnect and take over from a second terminal with a different theme.
   Application overrides survive; reset resolves to the new controller base.
   On exit, outer colors remain intact and only a mode-2031 subscription actually
   enabled by ZTerm is restored.

Herdr/Pi is a secondary visual reproduction. Compare it with direct connection
after the neutral protocol fixture; an application's explicitly chosen RGB theme
is not proof of a default-color reporting defect.

The automated workspace suite owns numeric parsing, stream ordering, invalid
controls, stack/reset lifetimes, history repaint, cursor semantics, initial PTY
queries, controller authorization, remote transport and reconnect. Physical host
theme switching and Linux runtime evidence must be recorded separately; this
recipe alone is not evidence that those runs passed.
