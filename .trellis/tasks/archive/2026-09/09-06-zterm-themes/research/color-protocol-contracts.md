# Color protocol contracts

Date: 2026-09-06. Design for PRD R01–R10; no implementation/runtime claim.
Supersedes narrower scope proposals in the earlier reporting research and audit.

## Control and response scope

| Family | Contract |
| --- | --- |
| OSC4 | Repeated index/color pairs for0–255; fixed set and `?` query. Replies include requested index. Invalid pair does not shift or discard later well-framed pairs. |
| OSC10/11/12/17/19 | Default fg/bg, cursor, selection bg/fg query/set. Successive dynamic-role parameters keep their positions even across unsupported intermediate roles. |
| OSC104 | No indices resets all palette overrides; valid listed indices reset individually. |
| OSC110/111/112/117/119 | Targeted override reset to latest controller base. |
| OSC21 | Indices0–255, foreground/background, cursor/cursor_text, selection_foreground/selection_background. Fixed set; `?` query; empty value sets Dynamic where legal; bare key resets. Unsupported-key query returns `key=?`; supported non-fixed value returns empty. |
| CSI?996n | Known Dark/Light replies `CSI?997;1n`/`CSI?997;2n`. Unknown produces no fabricated preference. |
| CSI?2031h/l | Session-global subscription, initially off. Enabling alone need not report; explicit996 is available. |
| CSI[?]Ps$p | DECRQM actual mutable mode state1/2; unknown0. Cover added5/2031 and existing exposed supported input modes; no invented unrelated modes. |
| DCS $q m ST | DECRQSS actual pen SGR: `DCS1$r<parameters>m ST`; unsupported request `DCS0$r ST`. |
| DCS +q hex-names ST | XTGETTCAP bounded allowlist below; canonical hex replies. |
| CSI #P / Pm#P, #Q / Pm#Q, #R | Ten-slot xterm color stack push/store, pop/restore and report; explicit contract below. |
| OSC30001/30101 | No-argument ordinary push/pop aliases on the same stack. |
| CSI?5h/l | Idempotent virtual screen reverse mode, mapping below. |
| SGR4:0–5,24,58,59 | None/single/double/curly/dotted/dashed, off, color, default color. Use pinned vte's semicolon/colon parsing and supported-sibling behavior. |

Every action executes at its position in the completed bounded child control.
Generate each query reply immediately; a later set in the same ingest cannot
change it. Lists execute left to right. No raw child OSC/DCS crosses the boundary.
Canonical new replies use7-bit introducers; legacy OSC replies match BEL/ST input
termination, and DCS replies use ST. Preserve existing UTF-8/C1 and cancellation
policy. Unsupported/malformed values never echo unvalidated bytes.

Legacy OSC cannot encode Unknown/Dynamic RGB: omit that entry's reply. OSC21 empty
means no fixed value and does not distinguish Unknown from Dynamic on the wire;
keep them distinct internally. An empty palette/default response from the outer
terminal is not proof of a dynamic rendering rule.

## Color encoding and bounds

One bounded opaque color parser handles the admitted child controls. Numeric
helpers may be shared with host reply decoding, but core gains no ANSI framing.

- `rgb:r/g/b`: each component1–4 hex digits, independently scaled to8bits with
  floor(value*255/((1<<bits)-1)).
- X11 `#RGB`, `#RRGGBB`, `#RRRGGGBBB`, `#RRRRGGGGBBBB`: most-significant bits,
  not CSS expansion. `#3a7` yields `#30a070`, unlike `rgb:3/a/7`.
- `rgbi:r/g/b`: finite signed decimal/exponent syntax, clip[0,1], floor(value*255).
  Reject NaN/Infinity and malformed components.
- Case-insensitive X11 names from a pinned, license-preserved static table. No
  runtime network/X server/user-file lookup. Preserve copied-data provenance.
- Opaque colors only. Alpha1 may be accepted; non-opaque alpha is unsupported,
  never silently treated as opaque.
- Canonical reply `rgb:rrrr/gggg/bbbb` repeats each8bit byte into16bits. Existing
  core RGB precision remains8bits; no floating/HDR grid or wire expansion.

Retain the current ordinary completed-string1KiB and aggregate reply64KiB bounds.
Batch large palette requests rather than lifting caps. Invalid setters leave the
addressed value unchanged. The C13 fix removes whole-SGR rejection: supported
siblings survive unsupported/malformed attributes according to pinned vte.

## Notifications and resets

Only changed external controller base/appearance generates subscribed997, after
commit and only with known appearance. A same-class palette change repeats1/2.
App setters, resets, pops and mode5 do not pretend to be an external preference
change. Base refresh continues under overrides so reset reveals the current base.

Main/alternate share color state. SGR resets rendition only. RIS clears overrides,
stack,5/2031 while retaining base. DECSTR keeps color overrides/stack and these
extension modes, preserving upstream soft-reset behavior for its own rendition
and modes. App process exit or alternate exit is not an implicit color reset.

## Stack contract

Follow documented xterm operations: ten slots, stack depth and highest initialized
slot. Omitted/0 pushes/pops;1–10 stores/restores a specific slot without push/pop.
Parameters execute in order. Invalid/uninitialized slot, underflow and overflow
are unchanged-state no-ops. Report `CSI?depth;highest_initialized#Q`; reset is0,0.

The public reference says indexed operations do not push/pop. Inspected xterm
source has different pointer bookkeeping for explicit slots/literal0
(misc.c:7944–8015). ZTerm follows the documented model, not that quirk; fixture
expectations must state this choice, not claim all historic quirks match.

Save effective RGB as fixed values, Dynamic as Dynamic and Unknown with unresolved
inheritance source. Pop restores known colors observed at push across controller
change; a later reset uses the latest base. Do not save controller identity,
subscription, appearance, screen mode or cell rendition. With mode5 active,
store/restore addresses the current visible role mapping.

## Reverse video and overlays

Mode5 uses xterm-style mapping: exchange default foreground/background and palette
0↔7,8↔15. Other indices and direct cell RGB stay literal. This follows concrete
swaps in xterm util.c:3095, not a blanket XOR of every colored cell's SGR7.

Canonical base/overrides use a reversible active-role mapping. Query/set/reset use
that mapping so set→query agrees in reverse mode. Snapshots expose effective mapped
values plus the fallback source for unmeasured outer defaults/indices: Unknown
alone cannot represent reversed unknown defaults. Do not apply mode5 a second time
in the presenter. Fixed cursor/selection stays fixed; Dynamic uses the displayed
cell. The legacy xterm cursor-equals-foreground heuristic is not adopted for
explicit fixed colors. Mode5 never changes appearance preference or outer chrome.

Order: mapped colors → cell SGR inverse → local selection → software cursor.
Explicit underline color resolves its palette reference without inverse swapping;
default underline follows final displayed text foreground. Dynamic selection swaps
the underlying pair per role; Dynamic cursor block uses displayed foreground,
Dynamic cursor text uses displayed background. Preserve unresolved source tokens
instead of substituting black/white.

## Capability allowlist

XTGETTCAP: Co/colors=256, RGB=8 (bits/channel), TN/name=xterm-256color (existing
profile), Tc/Su booleans, Smulx standard4:%p1%d template and Setulc standard packed
RGB58:2 template. Numeric/string values are hex-encoded; recognized booleans have
no value. Unknown/malformed names get a negative reply and stop the remaining
list; retain already completed replies. No runtime terminfo database or unrelated
keys/graphics capability expansion. Keep TERM/COLORTERM declarations unchanged.

Outer glyph support still determines physical underline shape. ZTerm transports
and emits the supported SGR; it cannot rasterize an unsupported outer glyph style.

## Primary sources

- [Xterm Patch411 controls](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html).
- [Pinned xterm source](https://github.com/ThomasDickey/xterm-snapshots/tree/9489b2056ee51fa9dd6a7087483b9b8f85d6a0c4): misc.c stack, util.c ReverseVideo, charproc.c mode dispatch.
- [Contour appearance](https://contour-terminal.org/vt-extensions/color-palette-update-notifications/).
- [Kitty color controls](https://sw.kovidgoyal.net/kitty/color-stack/) and [underlines](https://sw.kovidgoyal.net/kitty/underlines/).
- [Ghostty XTGETTCAP encoder](https://github.com/ghostty-org/ghostty/blob/main/src/terminfo/Source.zig): comparative RGB8/boolean/template encoding; pin provenance if copying constants in implementation.

This is a deliberate opaque text-terminal contract, not every vendor's graphics,
opacity, special attribute/pointer colors or historical implementation quirks.
