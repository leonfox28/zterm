# Terminal protocol support audit

Date: 2026-09-22. Baseline: `main` at `535e2fc`, workspace `0.1.35`,
`alacritty_terminal = 0.26.0`, local resolved `vte = 0.15.0`.

This is a source-backed inventory of common protocols, not an exhaustive
VT/xterm conformance certification. No runtime implementation was changed.

Follow-up, 2026-09-24: the user approved the five-item first batch and work on
`feat/terminal-protocol-compatibility`. The current requirements and concrete
design are in [the parent PRD](../prd.md) and [design](../design.md). The matrices
below remain the original baseline; their recommendation wording records the
earlier audit, rather than the current task approval status.
The five-item implementation and verification are recorded in
[implementation-evidence.md](implementation-evidence.md).

## Main findings

1. Most ordinary terminal text/grid operations, color controls, Kitty keyboard
   negotiation, and synchronized output already have implementations.
2. OSC 8 hyperlinks, OSC 7 working-directory reporting, OSC 133/633 shell
   integration, progress notifications, and image protocols are absent.
3. Several recognized controls stop before reaching a frontend: titles, bells,
   child-requested resize, cursor shape/blinking, hidden text and strikethrough.
4. OSC 52 writing works through the desktop presenter, but the Android event
   handler explicitly discards it as an intentional scope decision. Android
   already supports local selection and explicit Copy to the system clipboard;
   these are separate entry points. Clipboard reads are deliberately rejected.
5. Basic compatibility gaps include REP and ANSI.SYS-style CSI-u cursor restore.
   The latter is distinct from the supported Kitty CSI-u keyboard controls.
6. OSC 21 is a subset, and its unknown-field response differs from the current
   Kitty specification. Existing tests intentionally assert the current behavior.

## What determines actual support

The path is PTY output -> Zterm ingress policy -> Alacritty grid or Zterm-owned
color/effect handling -> semantic projection -> daemon/wire -> desktop/Android.
The desktop reconstructs presentation sequences; it does not relay the raw PTY
stream. A feature in Alacritty or the outer terminal is therefore insufficient.

Important source anchors, all relative to the repository root:

- [Ingress CSI dispatch](../../../../crates/terminal/src/ingress.rs#L465),
  [control-string dispatch](../../../../crates/terminal/src/ingress.rs#L626),
  [keyboard admission](../../../../crates/terminal/src/ingress.rs#L724), and
  [OSC 52 admission](../../../../crates/terminal/src/ingress.rs#L754).
- [Color OSC owner](../../../../crates/terminal/src/colors.rs#L138),
  [dynamic slots](../../../../crates/terminal/src/colors.rs#L233), and
  [XTGETTCAP allowlist](../../../../crates/terminal/src/colors.rs#L318).
- [Cell projection](../../../../crates/terminal/src/projection.rs#L218),
  [cursor projection](../../../../crates/terminal/src/projection.rs#L120), and
  [input-mode projection](../../../../crates/terminal/src/projection.rs#L289).
- [Style and cursor domain](../../../../crates/core/src/terminal.rs#L159),
  [terminal wire schema](../../../../proto/zterm/v2/terminal.proto), and
  [daemon update consumption](../../../../crates/daemon/src/terminal_driver.rs#L380).
- [Desktop notification/clipboard output](../../../../crates/cli/src/terminal_ui/ansi_presenter.rs#L320),
  [desktop effect handling](../../../../crates/cli/src/terminal_ui/session.rs#L704), and
  [Android effect handling](../../../../crates/android/src/terminal.rs#L981).

The daemon processes `update.replies` and `update.host_effects`, but does not
deliver `update.events`. Title, icon, bell and resize requests are only side
events. Snapshots have no title/icon state to recover after attachment.

## OSC matrix

"Implemented" below means the indicated subset has a semantic or effect path;
desktop behavior can still depend on the physical terminal. "Missing" includes
explicit filtering, rather than implying that an unknown sequence leaks as text.

| OSC | Purpose | Current behavior and limitation |
| --- | --- | --- |
| 0 / 2 | Window title | Recognition only: emits `TitleChanged`; no client delivery. OSC 0 follows the title path only, rather than updating both title and icon. |
| 1 | Icon name | Recognition only: emits `IconNameChanged`; no client delivery. |
| 4 / 104 | Indexed palette set/query/reset | Implemented for indices 0-255. Query replies require known RGB values; unknown values do not invent a color. |
| 10 / 11 / 12 | Default foreground/background/cursor color | Implemented set/query with chained parameters. |
| 17 / 19 | Selection background/foreground | Implemented set/query. |
| 110 / 111 / 112 / 117 / 119 | Reset the preceding dynamic roles | Implemented; reset returns to the current controller base. |
| 5 / 6 / 105 / 106; 13-16 / 18 and corresponding resets | Additional xterm color roles/modes | Missing. The numeric 10-19 parser range does not mean every role is supported: `dynamic_slot` maps only 10, 11, 12, 17, 19. |
| 21 | Extended color control | Partial: palette indices and six foreground/background/cursor/selection roles; no `visual_bell` or transparent-background roles, no non-opaque alpha. Unknown-field reply differs from current Kitty docs; see below. |
| 30001 / 30101 | Color stack push/pop | Implemented aliases sharing the ten-slot xterm color stack. |
| 7 | Working-directory/host URL | Missing; consumed as unsupported OSC. No shell-reported directory metadata reaches a client. |
| 8 | Explicit hyperlinks | Missing; opener/closer consumed before Alacritty. Display text survives, but URI/id metadata does not. An outer terminal's automatic URL recognition is a separate feature. |
| 9 | Ordinary notification | Implemented for nonempty ordinary text, bounded to 1,024 bytes including the OSC command/body separators. Desktop emits OSC 9; Android posts through its notification consumer and settings/OS permission gates. |
| 9;4 | Progress state/value | Missing; explicitly excluded from ordinary notification admission. Numeric OSC 9 subcommands are not implemented as their own commands. |
| 777;notify | Title/body notification | Implemented for the exact `notify;title;body` form. Other OSC 777 subcommands are unsupported. |
| 99 | Kitty rich notifications | Missing, including identifiers, updates, actions and responses. |
| 52 write | Application-to-clipboard text | Partial: only selector `c`, nonempty canonical Base64, valid UTF-8 without NUL, maximum 512 KiB decoded. Desktop emits a canonical OSC 52 request to its outer terminal. Android intentionally discards the received `ClipboardWrite` event; its local selection and system Copy action already work. |
| 52 read / other selectors | Clipboard query, primary selection, etc. | Explicitly rejected. Empty writes/clears are rejected too. This is a documented product policy, not an unconnected read callback. |
| 133 | Prompt/command boundaries and exit status | Missing; no command-block semantics from these sequences. |
| 633 | VS Code shell integration | Missing. This is a vendor extension and should not be a baseline compatibility requirement by itself. |
| 1337 | iTerm2 extensions | Missing as a family: `CurrentDir`, `RemoteHost`, marks, `File`, upload requests, cursor/profile extensions, etc. Zterm's own file-upload feature uses a separate protocol. |
| 22 | Mouse pointer shape | Missing. |
| 50 | Font control | Missing. |
| 66 | Kitty text sizing/explicit cell width | Missing; its contained text is consumed with the unsupported OSC. |
| 5522 | Kitty MIME clipboard protocol | Missing; unrelated to Zterm's plain-text OSC 52 write support. |

Protocol meanings checked against primary documentation:
[xterm OSC/control reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html),
[iTerm2 title, directory, hyperlinks and extensions](https://iterm2.com/documentation-escape-codes.html),
[Kitty colors](https://sw.kovidgoyal.net/kitty/color-stack/),
[shell integration sequences](https://code.visualstudio.com/docs/terminal/shell-integration),
[ConEmu progress](https://conemu.github.io/en/AnsiEscapeCodes.html),
[Kitty notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/),
[pointer shapes](https://sw.kovidgoyal.net/kitty/pointer-shapes/),
[text sizing](https://sw.kovidgoyal.net/kitty/text-sizing-protocol/), and
[MIME clipboard](https://sw.kovidgoyal.net/kitty/clipboard/).

### OSC 21 interoperability discrepancy

For `OSC 21;nonsense=? ST`, Zterm returns `OSC 21;nonsense=? ST`.
The current [Kitty color query specification](https://sw.kovidgoyal.net/kitty/color-stack/#querying-current-color-values)
requires an `unknown=` field containing the unpadded Base64 name, here
`unknown=bm9uc2Vuc2U`. This is directly observed in the runtime probe and follows
`colors.rs`; `extended_colors_distinguish_unknown_dynamic_and_unsupported_and_ignore_bad_pairs`
also asserts the current behavior. The local
[color contract](../../../spec/backend/terminal-colors.md) documents `=?`, so a
future conformance change needs to reconcile implementation, tests and contract.
This finding compares today's documentation; it does not establish when the
protocol text changed.

## Non-OSC support and gaps

| Family | Status | Evidence / practical effect |
| --- | --- | --- |
| Ordinary VT text/grid controls | Broad existing support, not exhaustively certified | Corpus covers cursor motion, erase, scrolling region, main/alternate screens, wide and combining characters. Alacritty is the single grid owner. |
| SGR indexed/RGB, bold/dim/italic/inverse | Implemented subset | Semantic styles preserve these fields; 256-color and truecolor corpus coverage. |
| Underline shapes and color | Implemented | SGR 4:0-5, 24, 58/59; none plus single/double/curly/dotted/dashed; tested in `color_protocol`. |
| SGR 5/6 blink, 8 conceal, 9 strike | Missing at presentation boundary | Runtime probes return ordinary `X` with default style for each. Hidden/strike flags exist upstream but are absent from `TerminalStyle`; conceal does not hide the text in Zterm. |
| Cursor visibility/position | Implemented | Included in semantic cursor. |
| DECSCUSR cursor shape and DEC 12 blink | Parser/state only | Cursor DTO has no shape/blink fields; cursor-blinking events are ignored. Android paints a block; desktop custom-color cursor is also a steady software block. |
| REP, `CSI Ps b` | Explicitly rejected | `A` + `CSI 3 b` yields only `A`, not `AAAA`. The upstream parser has REP, but Zterm ingress filters it. |
| Save/restore cursor | Partial | `ESC 7` / `ESC 8` work. `CSI s` saves, but plain `CSI u` is rejected by the Kitty-only admission branch. |
| Primary DA, status DSR and CPR | Implemented subset | Primary DA `CSI ?1;2c`; status `CSI 0n`; ordinary/private CPR. Exact replies covered by the corpus. |
| DA2/DA3, XTVERSION | Missing | Secondary/tertiary `c` queries are filtered; version query is not implemented by the pinned parser/handler. No capability reply should be inferred from the host's terminal identity. |
| Window operations / reports, `CSI ... t` | Missing end-to-end | Only positive `8;rows;columns` generates an undelivered resize event. Cell/pixel-size reports and title stack operations are filtered. This does not mean normal user/window-driven PTY resize is missing. |
| DECRQM | Supported allowlist | Known modes are reported; unrecognized modes return state 0. Some upstream-only modes are not part of the reported subset. |
| DECRQSS, `DCS $q` | Partial | Only current SGR (`m`) has a positive response; e.g. scroll margins (`r`) returns negative `DCS 0$r ST`. |
| XTGETTCAP, `DCS +q` | Partial | Only Co/colors, RGB, TN/name, Tc, Su, Smulx, Setulc. Unknown names produce a negative response and stop that list. |
| Kitty keyboard controls | Implemented five-flag negotiation | Set/push/pop/query are admitted and projected. Desktop adapts outer input modes; Android encodes native input through shared code. This audit does not certify every physical keyboard/layout combination. |
| xterm modifyOtherKeys | Missing | VTE recognizes the sequences, but Alacritty 0.26.0 does not implement its handler hooks; no Zterm mode/input state exists. Set followed by query produces no reply. |
| Bracketed paste / application cursor / application keypad | Implemented | Projected modes, desktop presenter mode synchronization and shared/native input encoding. |
| Focus reports, DEC 1004 | Desktop implemented; Android gap | Desktop enables outer focus reports. Android has no focus-report action or producer; its `Visible` action updates navigation only. |
| Mouse reports | Implemented common subset | X10/1000/1002/1003 plus default/1005 UTF-8/1006 SGR encodings. No 1015 urxvt or 1016 pixel encoding in the semantic model. Android bridge currently generates tap press/release and wheel reports, not desktop-equivalent drag/hover events. |
| Synchronized output, DEC 2026 | Implemented | Begin/end/query plus 150 ms bounded release and boundary-aware publication. Existing tests cover chunking, queries, effects and recovery. |
| Appearance reporting | Implemented | `CSI ?996n`, `CSI ?997;...n`, DEC 2031 subscription/query. Unknown appearance does not generate an invented reply. |
| xterm color stack | Implemented | `CSI #P/#Q/#R` and numbered slots. Separate from unsupported title/SGR stacks. |
| BEL / visual bell | Recognition only | Emits `AudibleBell` / `VisualBell`; daemon does not deliver side events. |
| SIXEL / ReGIS | Missing | Their DCS controls are consumed as unsupported; no graphic resource in the semantic model. |
| Kitty graphics | Missing | APC graphics payload consumed; no image state or renderer. Kitty keyboard support does not imply graphics support. |
| iTerm2 images/file transfer | Missing | OSC 1337 consumed; no protocol-compatible inline image/file path. |
| tmux passthrough DCS | Missing | Wrapper is consumed without executing its inner OSC. Applications using passthrough cannot rely on supported inner OSC reaching the outer terminal. Ordinary tmux text operation is a separate question. |

Control meanings were cross-checked with the
[xterm reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html).
Image framing was checked with [Kitty graphics](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
and [iTerm2 images](https://iterm2.com/documentation-images.html);
keyboard negotiation with [Kitty keyboard](https://sw.kovidgoyal.net/kitty/keyboard-protocol/).

## Android clipboard scope clarification

The user pointed out that Android selection/copy was already implemented.
The [Android design](../../09-06-android-app/design.md#L491) explicitly specifies
native-style selection handles, a floating system Copy action, and
`ClipData`/`ClipboardManager` writes only after explicit Copy. It separately
excludes host-driven kind-322 clipboard effects for that increment.
The [View implementation](../../../../apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt#L540)
implements that local Copy path. The discarded remote `ClipboardWrite` event
therefore follows the design; it is not evidence that Android cannot copy.

OSC 52 would add application-initiated writes, such as a configured remote
editor copying text without selecting it in the Android view. That is an
optional new workflow, not a prerequisite or repair for the existing system
selection/copy workflow. The previous first-batch recommendation conflated
protocol parity with missing user functionality and is withdrawn below.

## Suggested order for follow-up work

Refined after the user's follow-up asking which capabilities Zterm should add.
These are recommendations, not approved implementation scope. The product fit
is a tool-independent remote terminal with retained sessions and desktop/Android
controllers. Correct ordinary TUI behavior and consistent interaction across
devices have higher value than completing every vendor extension.

1. **First batch: daily terminal compatibility and interaction.**
   - Cursor shape and its declared blinking state, SGR strike/conceal, CSI-u
     cursor restore, and bounded REP. These address observable behavior, not
     extra UI features. Revisit existing ingress restrictions explicitly.
   - OSC 8 hyperlinks, including retained link metadata across history/reconnect.
     Start with web links; remote `file:` targets need a defined destination
     action rather than being interpreted as controller-local files.
   - OSC 0/2 application titles delivered as session metadata; preserve the
     distinction between the application title and a user-assigned session name.
   - Correct OSC 21 unknown-field replies and a useful geometry/query subset,
     starting with character dimensions known to the authoritative model. Report
     actual capabilities; do not impersonate all of xterm or invent pixel sizes.
2. **Second batch: shell-aware retained sessions.** OSC 7 plus OSC 133 can support
   directory display, command-boundary navigation, selecting a command's output,
   and showing completion/exit status. Pair protocol support with at least one
   concrete consumer and optional shell hooks; receiving markers alone is not
   the feature. Keep ordinary terminal operation independent of those hooks.
   These are generic shell semantics, not parsing a particular AI tool.
3. **Demand-driven follow-ups.** OSC 9;4 progress, bell policy, Android focus/
   drag/hover reports, modifyOtherKeys, extra status queries and OSC 99. Existing
   OSC 9/777 already provide basic notifications, so another notification family
   is less urgent than hyperlink delivery. Android application-initiated OSC 52
   writes are optional if a concrete remote-editor/script workflow requires
   them; retain existing local explicit Copy and disabled clipboard reads.
   Evaluate tmux passthrough
   against concrete nested-session needs, with admission of supported inner
   controls instead of arbitrary forwarding.
4. **Larger optional features.** Kitty graphics / SIXEL / iTerm2 images, OSC 66
   and MIME clipboard need demonstrated usage. Images require retained resource
   ownership, wire representation, reconnect/history behavior and two frontend
   renderers; allowing raw escape strings through the filter is insufficient.

Suggested acceptance is observable across desktop/Android and detach/reconnect:
an editor's cursor changes shape, existing explicit selection/copy keeps working,
explicit links remain actionable in retained content, and the same TUI keeps
its cursor position/styles. Use representative applications as validation
targets without parsing their proprietary output. Neovim documents
[cursor shape changes](https://neovim.io/doc/user/tui/#tui-cursor-shape) and an
[OSC 52 clipboard provider](https://neovim.io/doc/user/provider/#clipboard-osc52);
[Kitty shell integration](https://sw.kovidgoyal.net/kitty/shell-integration/)
illustrates consumers of prompt/command markers.

Clipboard read rejection, primary-selection exclusion and vendor-specific
window/font/profile commands are policy choices; they need not all become a
compatibility roadmap. OSC 633, OSC 1337 as a whole, and text blinking are not
recommended first-batch requirements. Current PTY environment advertises `TERM=xterm-256color`
and `COLORTERM=truecolor` in `crates/platform/src/pty.rs`.

## Verification and limits

Executed `cargo test -p zterm-terminal --all-features`: **54 tests passed**,
0 failed; doc-tests contained 0 tests. Breakdown: unit 13, colors 10,
notifications 3, security 14, synchronized output 3, corpus 5, snapshot/delta 6.

A temporary Rust executable linked the freshly built local `zterm-terminal`
and `zterm-core` libraries. It created isolated 5x20/2x20 models and printed
escaped replies, side-event classifications and public surface contents. The
temporary executable/source were removed automatically; no live terminal or
system clipboard was touched. Observed cases:

| Probe | Observed result |
| --- | --- |
| `A\e[3b` | Visible `A`; `UnsupportedSequence(Csi)`. |
| `\e[2;3H\e[s\e[4;8H\e[u\e[6n` | Reply `\e[4;8R`: restore did not happen. |
| Same positions using `\e7` / `\e8` | Reply `\e[2;3R`: restore works. |
| OSC 7, 8, 133, 633, 9;4, 99, 22, 50, 1337 | Unsupported OSC, no effect/reply; OSC 8 display text `link` remains. |
| `OSC 21;nonsense=? ST` | Reply preserves `nonsense=?`, confirming the documented discrepancy. |
| OSC 2, BEL, resize request | Title/bell/resize side events only; no host effect. |
| DA2 / `CSI 18 t` | Unsupported CSI, no reply. |
| `DCS $q r ST` | Negative `DCS 0$r ST`. |
| SIXEL / Kitty APC / tmux DCS | Unsupported control, no host effect. |
| modifyOtherKeys set + query | No reply or projected input mode. |
| SGR 5 / 6 / 8 / 9 followed by `X` | Surface retains `X` with default style. |
| `CSI 6 SP q` | Public cursor unchanged. |

The desktop/Android conclusions additionally trace production consumers. This
turn did not run a physical-terminal or Android device end-to-end session, so
it makes no new platform runtime certification claim. Existing passing tests
verify the current contract, including intentional restrictions; they do not
establish complete xterm or current Kitty conformance.
