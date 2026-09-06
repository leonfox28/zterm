# ZTerm color and theme capability audit

> Historical research: the user subsequently requested all C05–C18 gaps and C13.
> The converged PRD/design and color-protocol-contracts.md now own scope and decisions.

Date: 2026-09-06. Repository baseline: current `mundane-moth` checkout; no product
code changes in this task. This is a source/specification audit, not a claim of
physical terminal or remote-session conformance testing.

The user requested a comprehensive gap inventory against terminal and multiplexer
color/theme functionality before deciding implementation scope. Their confirmed
priority remains the color compatibility foundation ahead of theme selection UI.
This audit does not silently approve every missing extension for implementation.

## Comparison categories

- Basic character rendition: SGR defaults, indexed and RGB colors, and relevant
  text attributes.
- Established xterm interoperability: dynamic-color and palette query/set/reset.
- Modern extensions: appearance reports, colored/styled underlines, color stacks,
  and newer color-control protocols. These are not uniformly implemented by all
  terminals; absence is not necessarily a breach of a universal standard.
- Multiplexer responsibilities: one coherent effective color state, isolation,
  lifecycle, rendering, and consistent replies across local/remote attachments.
- Product preferences: theme presets, picker, files, preview and UI roles. These
  are useful product features without a standardized terminal protocol UX.

## Current capability matrix

| ID | Capability | Current status | Evidence / practical implication |
| --- | --- | --- | --- |
| C01 | Default foreground/background and SGR reset | Supported in the existing subset | Semantic defaults and 39/49 are preserved. SGR 0 resets rendition; it does not implement OSC palette/default-color reset. |
| C02 | Basic/bright ANSI colors and indices 0–255 | Supported | Named basic/bright colors project to indices 0–15; any u8 index survives the wire and is emitted as indexed SGR. Exact colors depend on the outer palette. |
| C03 | Explicit RGB foreground/background | Supported | RGB channels are preserved; no dark/light recoloring is performed. Existing tests cover indexed foreground and RGB background. |
| C04 | Bold, dim, italic, simple underline and per-cell inverse | Supported in the existing subset | Flags survive projection/wire/presentation. Actual glyph/intensity appearance is still supplied by the outer terminal. This is not a claim that all extended SGR combinations are supported. |
| C05 | Color query: defaults/cursor/palette | Missing | OSC 4/10/11/12 queries are consumed without a color reply. Child applications cannot obtain these values through zterm. |
| C06 | Set palette and dynamic default/cursor colors | Missing | OSC 4 with RGB and OSC 10/11/12 setters are outside the allowlist. An app's SGR 48 background works, but changing the terminal's default background using OSC 11 does not. |
| C07 | Reset modified colors/palette | Missing | OSC 104 and 110/111/112 are unsupported. Resetting to base colors differs from restoring colors that an application previously saved. |
| C08 | Acquire and maintain effective outer-terminal color state | Missing | The CLI emits semantic defaults/indices, but there is no negotiated RGB palette/default/cursor/appearance state propagated to the Session. The user's outer theme is visible indirectly but cannot be authoritatively described to the child. |
| C09 | Direct dark/light inquiry | Missing | CSI ?996n is rejected by the DSR allowlist; no ?997 report is generated. `COLORTERM=truecolor` describes capability, not appearance. |
| C10 | Appearance/palette update subscription and capability probing | Missing | Mode 2031 state and reports are absent. DECRQM CSI ?2031$p is rejected with other `$` CSI bodies. Applications cannot discover or subscribe to this feature. Notifications should also allow requery after a palette change within the same light/dark class. |
| C11 | Per-Session effective palette/defaults in snapshot/delta/reattach | Missing as a color-state feature | Existing live/history transport preserves each cell's Default/Indexed/RGB values, but no palette/default-color/appearance state exists in the semantic snapshots or deltas. Adding writes/reporting requires this ownership and lifecycle design. |
| C12 | Colored/styled underlines | Missing / flattened | SGR 58/59 causes the whole SGR sequence to be filtered. Underline styles that reach the engine are projected into one Boolean underline flag, losing distinctions such as curly/double/dashed. |
| C13 | Preserve supported styles in mixed SGR containing unsupported underline color | Current compatibility defect at the ingress policy boundary | A complete SGR is rejected if any top-level parameter is 58/59. Valid foreground/background or reset parameters in the same sequence are also discarded; see the example below. |
| C14 | Cursor-specific and selection-specific color semantics | Partial display, missing controls | A cursor position, visibility and text template style are projected, but this is not an independent cursor-color state. Selection display toggles inverse locally; OSC selection foreground/background 17/19 and associated controls are absent. |
| C15 | Save/restore palette and dynamic colors with a stack | Missing optional extension | Neither xterm XTPUSHCOLORS/XTPOPCOLORS/XTREPORTCOLORS nor kitty OSC 30001/30101 is implemented end to end. A stack restores a previously active theme, unlike a reset to configured defaults. |
| C16 | Newer unified color-control protocol | Missing optional extension | Kitty OSC 21 is outside the OSC allowlist. It can represent unset/dynamic cursor and selection colors; not a universal prerequisite for the initial xterm-compatible deliverable. |
| C17 | Query current SGR/capability information | Partial capability declaration; missing relevant inquiry paths | TERM/COLORTERM are set, but DECRQSS SGR (DCS $q m), XTGETTCAP (DCS +q), and general DECRQM are not supported by the current ingress path. Scope should focus on capabilities actually exposed, not implement every unrelated query. |
| C18 | Whole-screen reverse-video mode | Missing legacy compatibility | Pinned vte 0.15.0 does not recognize private mode 5, and the projected mode set contains no whole-screen inversion state. Per-cell SGR 7 still works. This is separate from dark/light appearance selection. |
| C19 | Theme presets, chooser, preview, customization and persistence | Missing product feature | Existing config is daemon infrastructure-only; status uses inverse defaults, scrollbar defaults, selection inverse. P1–P7 remain deferred product proposals. |

## Code anchors

- `crates/core/src/terminal.rs:135`: Default/Indexed/RGB color representation;
  `:147`: supported style fields; `:194`: cursor data; `:297`: mode fields.
- `crates/terminal/src/projection.rs:117`: projected cursor/modes;
  `:187`: style flags; `:199`: color variants; `:207`: named/basic/bright index
  mapping; `:242`: projected mode subset.
- `crates/cli/src/terminal_ui/ansi_presenter.rs:129`: local selection overlay;
  `:204`: cursor template style output; `:320`: style writer;
  `:342`: Default/Indexed/RGB encoding.
- `crates/terminal/src/ingress.rs:419`: DSR reply allowlist;
  `:451`: private-mode dispatch; `:487`: unsupported whole-SGR filtering;
  `:499`: string dispatch; `:524`: OSC allowlist; `:657`: underline-color scan.
- `crates/terminal/src/engine.rs:109`: unsupported upstream color/reply callbacks.
- `proto/zterm/v2/terminal.proto:146`: color/style/cursor/surface DTOs;
  `:199`: snapshot; `:211`: delta. No palette or appearance state is carried.
- `crates/platform/src/pty.rs:730`: fixed TERM/COLORTERM capability declarations.
- `crates/terminal/tests/terminal_corpus.rs:131`: existing color corpus;
  `crates/terminal/tests/security_policy.rs:343`: existing underline filter test.
- `.trellis/spec/backend/terminal-model.md`: the current subset and explicit
  exclusion of color queries, palette state, and underline color/style detail.
- `.trellis/spec/backend/session-service.md:108`: one controller per Session;
  base-color ownership should use that existing contract.

## Mixed SGR finding C13

An application-neutral input illustrating the source-level issue is:

```text
ESC [ 31 ; 58 ; 5 ; 2 m X
```

This requests red text and an indexed underline color in the same SGR. Current
`sgr_contains_underline_color` identifies the top-level 58, after which
`dispatch_csi` rejects the entire control. The red foreground is therefore not
applied and `X` retains the preceding style. A SGR containing 59 can similarly
discard a supported foreground/background or reset in the same sequence.

The existing test proves that numeric RGB components 58/59 are not mistaken for
underline attributes. It does not assert preservation of ordinary attributes
when a genuine underline-color attribute appears beside them.

Classification: an overly broad policy boundary causes a local compatibility
defect. Supporting underline-color rendering is a separate feature; graceful
handling of unsupported attributes should preserve supported sibling attributes.
The behavior follows directly from the inspected code; no new runtime probe was
run and no claim is made that this input caused the user's Herdr screenshot.

## Multiplexer-specific design obligations

Once color state is supported, it must be scoped to a Session's virtual terminal:

- The current controller supplies the base outer color/appearance information;
  a child may have explicit overrides if setting colors is supported.
- Queries and rendering must read the same effective values. Cells holding an
  index or Default must react to relevant palette/default changes even if no
  cell text changes; history and live views need the same interpretation.
- Reattachment/takeover refreshes base information while applying the agreed
  override/reset policy. Disconnected Sessions have defined last-known/unknown
  behavior. No stale controller may overwrite the current controller's state.
- Each Session has isolated overrides, subscriptions and any supported stacks.
  An application's color change must not leak to unrelated Sessions or leave
  the outer terminal contaminated on detach.
- Main/alternate screens and terminal reset follow an explicit, tested color
  lifetime contract. Do not infer that color state is automatically restored
  merely by switching screens or observing a process exit.
- Missing/delayed outer replies must not be consumed as ordinary keystrokes or
  stall PTY drain; reply ordering and framing remain deterministic.

These are required properties of the proposed feature, not evidence that
currently rejected color setters are already leaking across Sessions.

## Recommended ordering (not yet approved implementation scope)

1. Define shared color-state ownership plus acquisition/reporting and appearance
   inquiry/notification. This covers C05/C08–C11 and the necessary parts of C17.
2. Build setting/resetting and the corresponding rendering/isolation/lifecycle
   behavior on that same state (C06/C07 and cursor aspects of C14). This is the
   natural foundation for a broadly compatible terminal theme implementation.
3. Correct mixed-SGR color loss (C13). Independently choose whether full colored
   and styled underline support (C12) belongs in the same deliverable.
4. Consider optional stacks, selection controls, legacy screen inversion, and
   newer unified protocols by actual application needs (C14–C18).
5. Build product theme selection/presets on the working foundation (C19/P1–P7).

The first two steps can be one delivery with internal implementation stages;
their separation here describes technical dependency, not an approved scope cut.

## Primary protocol sources

- [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html):
  SGR, OSC palette/default/cursor set/query/reset, DECRQM, DECRQSS, screen inversion,
  and xterm color stack. These have different historical/extension origins.
- [tmux input implementation](https://github.com/tmux/tmux/blob/master/input.c):
  concrete multiplexer handlers for color inquiries and current-theme reporting.
- [Contour appearance and palette notifications](https://contour-terminal.org/vt-extensions/color-palette-update-notifications/):
  CSI 996/997 and mode 2031, including palette changes without a dark/light flip.
- [Kitty colored/styled underline extension](https://sw.kovidgoyal.net/kitty/underlines/):
  SGR 58/59 and styles 4:0–5.
- [Kitty color control](https://sw.kovidgoyal.net/kitty/color-stack/):
  optional stack/unified protocol, dynamic/unset colors, and reset semantics.

This inventory intentionally does not treat every historic xterm/Tektronix or
vendor-specific graphics/color-space extension as a required zterm feature.
Fonts, window transparency, background images and OS-native theme settings
remain outer-terminal responsibilities for the current raw-terminal CLI product.
