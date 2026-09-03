# Nested TUI Right-Margin Rendering

## Scope

This follow-up investigates the real-terminal symptom observed after commit `9418772`: Herdr's
theme-colored pane scrollbar is visible on entry, then disappears after the first wheel redraw.
The required result is renderer correctness for any nested TUI, not application detection or a
Herdr-specific compatibility branch.

The user clarified that “generic” must not merely mean “the patch has no Herdr process-name check.”
The investigation must first identify the terminal/multiplexer invariant that prevents a class of
composition defects, even if that leads to more code or a broader architectural change. Herdr and
PiAgent are evidence-producing examples, not prescribed special cases or APIs to imitate.

Sources and probes were reviewed on 2026-09-03:

- Zterm branch `fix/live-bottom-scrollbar-flicker` at `941877281135697b114692d4a643b593cbf632e5`;
- Herdr release `v0.8.2` at `9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c`;
- the locally installed Ghostty `1.3.1`, whose matching source tag is
  `22efb0be2bbea73e5339f5426fa3b20edabcaa11`;
- a task-private 24x79 -> 24x80 Herdr PTY/model probe outside the product worktree.

## Discriminating evidence

### The initial glyph is child-owned

The user confirmed that the initially visible bar has Herdr's theme color rather than Zterm's own
gutter appearance. This rejects the leading alternative that the first bar is merely stale Zterm
chrome left over from Main-to-Alternate ownership transfer.

Herdr's source independently supports that observation:

- it reserves a stable one-column gutter for an ordinary terminal pane and draws `▕`/`▐` there
  whenever the pane has scrollback;
- it draws pane content before pane chrome, so its scrollbar is the final Herdr writer for that
  column;
- it disables terminal line wrapping while its client UI is active and brackets redraws with
  synchronized-output markers.

Primary source:

- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/ui/scrollbar.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/ui/panes.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/client/mod.rs>

### Herdr and the authoritative Zterm model retain the bar

The task-private Herdr `v0.8.2` probe generated 400 lines in one pane, then sent an actual SGR wheel
report at a pane coordinate. Before and after the wheel:

- Herdr was on the outer Alternate screen with `AnyMotion` + SGR mouse reporting;
- the authoritative Zterm snapshot contained `▕` down the physical final column and `▐` for the
  thumb;
- replaying the returned snapshot/resync into a second `TerminalModel` retained every final-column
  glyph.

This rules out Herdr hiding the bar, wheel misrouting in the probed mode, Alacritty grid projection,
and full-snapshot replay as the primary cause. It also explains why the prior model-only black box
passed while the physical Ghostty smoke failed.

### The changed-row encoder has a cross-emulator right-margin hazard

Zterm's full encoder paints each row without a trailing erase. Its delta encoder instead does this
for every changed row:

```text
CUP(row, 1) -> row content -> SGR reset -> EL0
```

`EL0` erases from the current cursor cell through the end of the row. When the replacement content
occupies the physical final cell, there is no stale suffix to erase. Nevertheless, Zterm emits the
erase after the final glyph and relies on the outer terminal's post-print cursor/pending-wrap
semantics to make that erase harmless.

Ghostty 1.3.1 does not make that assumption safe. Its `.right` `eraseLine` starts at the current
cursor `x`, clears through `cols`, and resets pending wrap. Its own regression prints a full-width
row, observes pending wrap, applies erase-right, and expects the rightmost cell to be erased before
the next print. Wraparound mode 7 is enabled by default.

Primary source:

- <https://github.com/ghostty-org/ghostty/blob/22efb0be2bbea73e5339f5426fa3b20edabcaa11/src/terminal/Terminal.zig#L2392-L2450>
- <https://github.com/ghostty-org/ghostty/blob/22efb0be2bbea73e5339f5426fa3b20edabcaa11/src/terminal/Terminal.zig#L10873-L10910>
- <https://github.com/ghostty-org/ghostty/blob/22efb0be2bbea73e5339f5426fa3b20edabcaa11/src/terminal/modes.zig#L202>

An equivalent byte probe retained the glyph in Alacritty and tmux. That is not exculpatory: it is
the cross-emulator difference which allowed Zterm's existing replay tests to pass. The current
sequence is semantically wrong because it issues an inclusive erase when the suffix length is zero.

## Root-cause conclusion

The highest-confidence cause is Zterm's unconditional post-content `EL0` in changed-row deltas. It
exactly predicts the asymmetric symptom:

1. the initial full frame shows a theme-colored final-column glyph;
2. a later incremental frame rewrites that row;
3. Zterm paints the final-column glyph and then asks Ghostty to erase starting at that same cell;
4. later TUI diffs may regard the still-present logical bar as unchanged, so no later child write
   repairs the physical cell.

This is a renderer contract defect, not a scrollbar or mouse-routing defect. It can affect borders,
status cells, full-width text, wide glyphs, and any future TUI that legitimately occupies the right
margin.

## Protocol and architecture boundary

### VT control sequences do not provide nested-surface ownership

ECMA-48 defines imperative operations on a terminal presentation component. In particular, `EL0`
puts the active character position and every position through the line end into the erased state.
It does not express “replace the suffix only if one exists,” a declarative row, a nested widget, or
which layer owns an overlapping cell. `terminfo` can describe outer-terminal capabilities such as
automatic right margins and newline glitches, but it cannot add a missing zero-length erase or
merge independently authored UI layers.

Alternate-screen and mouse-reporting modes remain useful ownership signals at the terminal boundary:
they tell Zterm which terminal surface is active and whether input belongs to the child. They do not
describe a scrollbar or allocate screen regions between the child and Zterm chrome. Synchronized
output mode 2026 can keep the outer emulator showing its preceding buffer while it processes one
batch, preventing presentation of an intermediate state; it does not decide what the final buffer
should contain and cannot repair an incorrect erase.

Primary sources:

- <https://ecma-international.org/wp-content/uploads/ECMA-48_5th_edition_june_1991.pdf>
- <https://invisible-island.net/ncurses/man/terminfo.5.html>
- <https://contour-terminal.org/vt-extensions/synchronized-output/>

### Multiplexers solve nesting with a virtual screen and one final presenter

There is no separate standard “nested TUI protocol” to adopt. A multiplexer terminates the child's
terminal protocol: every pane is a PTY/virtual terminal, child bytes mutate a screen/grid model, and
the multiplexer renders its own complete result to the outer terminal. `tmux` exposes each pane as a
separate PTY and stores screen/grid state before its `tty-draw` layer emits outer-terminal updates.
Herdr follows the same boundary with libghostty-vt plus a Ratatui frame.

The general UI precedent is older than either project. Curses keeps a desired virtual screen and a
known physical screen; applications first merge window updates into the virtual screen, then one
`doupdate` compares it with the physical screen and writes the terminal. Ratatui likewise asks the
application to populate one whole current frame, diffs it against its preceding buffer, and warns
that direct backend writes outside that pipeline can desynchronize the retained baseline.

Primary sources:

- <https://man.openbsd.org/tmux.1>
- <https://github.com/tmux/tmux/blob/master/input.c>
- <https://github.com/tmux/tmux/blob/master/screen.c>
- <https://github.com/tmux/tmux/blob/master/tty-draw.c>
- <https://invisible-island.net/ncurses/man/curs_refresh.3x.html>
- <https://docs.rs/ratatui/latest/ratatui/struct.Terminal.html>

## Zterm architectural finding

Zterm already has an authoritative semantic `ProjectedScreen` in the daemon, but converts it to
canonical ANSI before the attachment receives it. The CLI then appends attachment-local status and
scrollbar ANSI to those pre-encoded bytes inside the same synchronized-output envelope. That gives
the user one write transaction, but not one semantic frame or one retained physical-screen baseline.
It leaves correctness distributed between a daemon row encoder and several later CLI chrome writers.

The higher-level invariant should instead be:

1. Child PTY bytes terminate at exactly one terminal parser/model; nested application identity is
   irrelevant.
2. Terminal cells, attachment-local history, status, gutter, cursor, and mode policy are composed
   into one desired frame with explicit, non-overlapping region ownership.
3. Exactly one host presenter compares that desired frame with the last successfully committed
   physical frame and encodes the transition for the outer backend.
4. The committed baseline advances only after the complete output transaction is written and
   flushed; a resize, active-screen change, capability change, or uncertain write triggers an
   explicit resynchronization rather than allowing an outside writer to repair cells ad hoc.
5. ANSI is one desktop backend encoding, not the cross-platform presentation model. Android can
   consume the same semantic surface and compositor output through a native cell renderer and its
   own touch/vsync policy.

Under this invariant, a Herdr scrollbar is not detected as a scrollbar. It is simply the desired
cell at column `W - 1`; a Zterm gutter is a different region assigned during layout before either is
rendered. No later layer can erase a cell that the retained final frame still owns.

## Architectural options

### A. Repair only the current row encoder

The extent-aware `EL0` correction below is still required protocol correctness for the existing ANSI
path and for compatibility fallback. It fixes the reproduced defect generically, but by itself does
not remove the early-encoding/late-overlay boundary. Treating it as the whole solution would not
satisfy the clarified architectural goal.

### B. Add exact cell diffing in the daemon

Diffing `ProjectedScreen` cells in the daemon would avoid row-tail erasure and improve the terminal
content encoder. It still cannot include attachment-local connection status, gutter state, or future
native Android presentation. CLI chrome would remain an out-of-band physical writer, so this only
moves the split rather than eliminating it.

### C. Add a semantic attachment surface and client-side compositor/presenter

This is the recommended target architecture. The daemon transports a versioned semantic screen or
cell patch plus revision, cursor, active-screen, modes, and history identity. The attachment keeps a
committed semantic terminal surface, composes local chrome into a complete `ComposedFrame`, and gives
that frame to exactly one backend presenter. The desktop presenter performs capability-aware cell
diff/ANSI output; Android uses a native renderer. Existing ANSI snapshot/delta remains a
capability-negotiated compatibility fallback until the migration window closes.

This changes wire and retained presentation state, so it needs bounded cell encoding, resync and
write-failure contracts, Unicode/wide-cell rules, backward compatibility, and independent rollback.
Those costs are real, but each is demanded by either multi-client transport, exact composition, or
the already-planned Android renderer rather than by the Herdr example.

### Rejected shortcuts

- Parsing daemon-authored ANSI a second time in the CLI reconstructs state that already existed in
  the daemon, adds a second parser/state machine, and gives Android no useful protocol.
- Moving all chrome into the daemon conflates session state with attachment-local connection,
  viewport, platform, and renderer policy.
- Inventing a growing set of structured row-clear/chrome commands creates a second pseudo-terminal
  protocol without obtaining a complete desired frame or a single authoritative baseline.

## Generic correction

Changed-row encoding must carry the replacement's **visual cell extent**:

1. paint the replacement content;
2. if the extent is strictly less than the row width, explicitly `CUP` to the first stale cell,
   reset SGR, and issue `EL0`;
3. if the extent equals the row width, emit no erase because the stale suffix is empty;
4. restore final cursor and input modes as today.

Explicitly positioning the suffix clear removes reliance on post-print cursor behavior for ordinary,
wide, styled, combining, and wrapped rows. Content remains before the tail clear, preserving the
existing no-blank ordering on terminals that ignore DEC 2026.

This correction does not require exposing DECAWM in `TerminalModes`, forwarding child-private
rendering modes to the physical terminal, changing the wire, or interpreting scrollbar glyphs. The
outer Zterm renderer continues to virtualize the child terminal and emits only its canonical
allowlisted ANSI.

## How Herdr handles a nested Pi or other TUI

Herdr does not transparently forward its pane's PTY bytes to the physical terminal. Each pane has a
PTY and a `libghostty-vt` terminal model. A nested Pi/TUI writes ANSI to that PTY; Herdr parses it,
copies the complete visible pane cells into a Ratatui frame, then draws Herdr-owned borders and
scrollbar cells into that same frame. On a child alternate screen Herdr returns its stable scrollbar
gutter to the child, so the nested TUI receives the full pane width.

The final Herdr presenter retains the preceding `FrameData`, scans semantic cells, invalidates both
halves when narrow/wide glyph widths change, and writes only changed cells. A removed glyph is
replaced by a literal blank cell at its exact coordinate; the diff path does not use `EL0` to infer a
row tail. Herdr also disables outer-terminal autowrap during its UI lifetime and wraps each completed
frame in synchronized output.

Mouse ownership is independently derived from the nested terminal model. Mouse reporting wins and
the wheel is encoded and forwarded to the child; otherwise alternate-screen alternate-scroll is
translated to cursor keys; otherwise Herdr changes its own pane scrollback. Neither rendering nor
input routing recognizes Pi, Herdr, an executable name, or a scrollbar glyph.

Primary source:

- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/pane.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/ui/panes.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/protocol/render_ansi.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/app/input/mouse.rs>
- <https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/client/mod.rs>

## Simplicity comparison

The proposed Zterm extent is not new retained state or an extra grid pass. The current
`encode_row_content` already computes the same trailing-content `end` before emitting a row. The
minimal patch returns that existing value and uses one conditional suffix clear.

The alternatives are less attractive for this follow-up:

- clearing before painting removes the branch but reintroduces a visible blank stage on terminals
  without synchronized-output support;
- always padding every changed row to full width removes `EL0` but can add almost one terminal width
  of spaces for a one-cell update and force more full resyncs over remote links;
- adopting Herdr's exact cell diff gives the cleanest long-term framebuffer invariant, but requires
  width-overlap invalidation, blank-cell writes, style/run batching, host-autowrap lifecycle and a
  retained final presentation surface. Zterm's CLI still composes daemon-authored ANSI with local
  chrome, so faithfully copying Herdr would be a renderer/protocol phase rather than a smaller bug
  fix.

Therefore the extent-aware row replacement remains a correct compatibility-path repair inside
Zterm's current row-delta architecture, but it is no longer considered the complete answer. The
clarified product decision is whether to land that guardrail first and then migrate to option C
before Android, or expand the current follow-up into the complete semantic-presentation migration.

## Deferred, behavior-neutral observation

Herdr also brackets frames with DEC synchronized output, while Zterm currently rejects child mode
2026 as an unsupported side event and publishes latest state by PTY read/revision. Preserving child
transaction boundaries could reduce intermediate model publications, but it is not required to fix
the right-margin erasure and would expand driver/model semantics. Keep it out of this patch unless a
separate red test proves frame tearing remains after the extent-aware encoder fix.
