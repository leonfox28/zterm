# Design: virtual terminal color compatibility

## Scope and architecture

Implements PRD R01–R10 (C05–C18), preserving C01–C04. The authoritative byte-level
subset is [protocol contracts](research/color-protocol-contracts.md). Evidence and
exact integration anchors are in [state research](research/color-state-architecture.md)
and [frontend research](research/color-frontend-architecture.md).

```text
physical terminal
    → sole stdin/control decoder → bounded outer observations
    → typed create/attach/update command, current controller authorization
    → Session → TerminalDriver ordered commit → TerminalModel
                                                  ├ Alacritty grid/SGR
child PTY output → existing bounded ingress ────────┤ ZtermColorState
child PTY input  ← ordered bounded canonical replies┘
    → semantic snapshot/delta/history + effective colors
    → CLI semantic composition → color/overlay resolver → sole ANSI presenter
```

The viewer daemon's remote tunnel stays opaque. No raw color OSC, reply closure,
terminal-engine type or presentation variant enters core/protobuf. Child color
changes never invoke a frontend query callback and never mutate physical colors.

## One state owner and data model

`AlacrittyEngine` owns `ZtermColorState`; upstream Alacritty palette mutation and
ColorRequest callbacks remain blocked. Alacritty alone owns cell/grid/scrollback,
current pen, underline attributes, and ordinary screen/input modes. The new object
owns only bounded palette/special-role metadata, added appearance/reverse modes,
base observations, overrides and stack. This avoids a second palette authority.

Domain values use u8 RGB,256 fixed palette slots, six roles (fg/bg/cursor/cursor
text/selection fg/bg), and Unknown/Dark/Light appearance. Base palette/fg/bg have
Unknown or RGB; special roles additionally allow Dynamic. Each app override is
Inherit, RGB, legal Dynamic or saved Unknown with its original source. Keep Unknown distinct from Inherit and fixed black.

Effective snapshots contain:

- complete mapped palette/roles and appearance;
- `changed_at: Revision` from the existing revision domain;
- reverse-mode state plus source-aware inheritance fallback for unobserved outer
  values (mode5 swaps unknown defaults/indices without inventing RGB);
- cursor presentation policy/provenance sufficient to distinguish inherited native
  cursor from explicit fixed or Dynamic customization requiring software painting.

An effective RGB value alone cannot supply that cursor provenance. Likewise,
Unknown without its original default/index source cannot encode mode5 correctly.
Do not introduce raw ANSI fallback tokens; use exactly 262 bounded numeric source indices (0–261).

Keep cells Default/Indexed/RGB. Replace underline Boolean with canonical enum
None/Single/Double/Curly/Dotted/Dashed and optional semantic underline color. Copy
from pinned upstream cell extras and current pen; do not parse SGR twice. Fixed
underline metadata is not combining-text storage and must not consume the existing
4096 combining-cell allowance.

## Mutation, replies and revision ordering

Ingress dispatches admitted typed color operations in byte order. Parse only
completed bounded controls and answer queries immediately into UpdateCollector.
`set A → query → set B → query` returns A then B even in one ingest. Color controls
remain intercepted instead of also mutating upstream palette state.

Add `TerminalModel::update_base_colors` and corresponding driver/Session/client
methods using semantic values. Preflight Revision before mutation. Equal updates
are no-ops. Base changes hidden by overrides still matter to future reset and may
advance model revision; effective `changed_at` advances only when effective query,
rendering or required presentation provenance changes. Subscription changes remain
model state even when they do not change cells.

Use one driver commit gate for ingest, resize and base updates from preflight
through reply write/publication. Release the model mutex before PTY reply I/O;
retain the gate so later base notifications cannot overtake earlier queries or
publish a lower revision afterward. Lock order: commit gate → short model lock;
release model lock → existing PTY writer → publication. Never await frontend or
network under the gate, and keep owner-only interruption/reaping independent.
Reply overflow/I/O failure follows current fatal-driver finalization, not partial
silent reply loss. Do not create a second uncoordinated reply sender.
The driver returns Option<Revision> for base updates, comparing before/after
under the commit/model locks. None means this base did not change. Never compare
a separately sampled revision watch with the result: unrelated PTY output can
race and falsely force a takeover replacement snapshot.

## Initialization, controller and lifetime

| Transition | Contract |
| --- | --- |
| CLI entry | Collect available outer observations before stateful prepare/create; bounded local deadline, signals/detach remain usable. |
| New default-main | Thread base through default_main/create_reserved/model construction before spawning/starting PTY drain. Startup queries see this base. |
| Interactive named create | Extend SessionCreateRequest and create fingerprint/API with initial base; do not create first and send colors after attach. Noninteractive no-viewer creation uses explicit Unknown. |
| Existing free Session attach | Install authorized base at controller grant before initial capture. |
| Occupied/pending takeover | Staged candidate base only; active controller's view and child queries are unchanged. |
| Takeover commit | Preflight and install candidate base with lease handoff; stale proposals lose authority. If prepared snapshot predates the committed state, replace and acknowledge it before activation. |
| Active refresh/replacement sync | Exact current attachment/lease, monotonically ordered per-attachment proposals. Coalesce latest state; stale/duplicate updates cannot repeatedly notify. |
| Disconnect/detach | Keep last observed base, overrides, stack and child subscription. No physical callback waits or guessed app reset. |
| Reconnect | New attachment identity, latest completed client observations, normal checkpoint/full-sync rules; no replay of commands bearing old attachment IDs. |
| Main/alternate | Share colors/stack/2031/mode5. Ordinary upstream screen state remains upstream-owned. |
| Reset/end | Protocol resets follow research contract. Session/root end destroys color state; daemon state is not persisted across restart. |

No SQLite/config migration, global theme store, appearance timer or process-name
heuristic. Existing daemon config remains infrastructure-only.

## Semantic transport and cache

Extend core surfaces, deltas, history frames, projection checkpoints, protobuf and
conversions with a complete bounded color snapshot. A delta may have zero row
patches. Use fresh tags, preserve reserved tag9 and retired wire kinds, allocate a
canonical attach323, create209 and structured base-update324; retire300/202. Exactly 262 total slots (256 indexed plus six roles), boxed fixed arrays in
core, bounded inherited-source indices, legal enum/role combinations and `changed_at <= enclosing revision` are validated at conversion.

Keep transactional candidate construction and last-successful-output commit.
Missing required colors are invalid, not equivalent to Unknown. Increment internal
projection checkpoint format from 2 to 3. Old checkpoints require full sync, not migration.

History cells remain semantic. Colors use the newest validated `changed_at` seen
on live/history streams; older history cannot roll them back, and equal stamps
cannot disagree. A newer history frame may carry colors ahead of an in-flight live
delta. Palette changes neither change history epoch nor force refetch. Retained
visible history redraws with the current palette while hidden live text keeps its
existing no-repaint behavior. Color-only changes preserve still-valid selections.

### Version compatibility

Keep wire major2/ALPN zterm/2 and one canonical representation. Move attach to
kind323, Session create to209 and allocate base update324; retire300/202 with no
aliases/fallback. Old endpoints reject the new kind before dispatch; new endpoints
reject retired kinds before create/attach effects. This follows existing unknown-
kind policy without feature negotiation or an extra semantic-version marker.
Required complete color fields still validate on every semantic response.

A marker added to old kind300/202 alone would be ignored by old servers, allowing
shell startup before the client rejects the later snapshot. A separate preflight
would add a round trip and incarnation binding. Fresh canonical kinds reject before
Session creation/PTY spawn/takeover/input intrinsically. Normal discovery,
connection and empty operation-lease issuance may still precede rejection; this
is not a claim of zero control-plane activity. Extend registry/retirement tests,
local IPC and route-neutral client dispatch; the viewer tunnel remains opaque.

Ship matching CLI/daemon/remote-host binaries. Do not automatically restart live
daemons, migrate old-process terminal state or kill Sessions for upgrade/testing.

## Outer probe and input ownership

OuterProbe produces typed selector batches; the sole presenter serializes and
flushes physical queries between complete presentation transactions. It cannot
interleave query bytes into an in-progress frame or add another stdout owner.

The physical stream has one decoder and one stdin reader. Typed terminal replies
are recognized before prefix, selection/history or child-key routing. Bracketed
paste owns its payload first; apparent color replies inside paste remain literal.
Use exact bounded grammars for4/10/11/12/17/19/21,997,2031 DECRQM and DSR status.
BEL/split ST, UTF-8, malformed/oversized recognized replies and cancellation retain
framing. Unsupported input keeps existing handling; do not use regex over chunks.

A single collection queries all256 slots in bounded frames, defaults/special roles,
OSC21 dynamic semantics,996 and2031 status, then DSR5n as an ordered response fence.
Use a named overall250ms local collection deadline and at most64KiB observed reply
bytes. This is one deadline, not256 serial waits. Completion/fence may end early;
timeout commits partial observations without delaying daemon PTY drain.

No response IDs exist. A local generation cannot prove delayed reply freshness.
After timeout, consume late replies without applying them; do not launch another
ambiguous round until its trailing DSR boundary arrives. Keep one coalesced refresh
bit. If a terminal never supplies the ordered boundary, retain last observations
and report limited live refresh; timeout is not proof of freshness. Even a DSR
boundary assumes responses follow processed-request order, not arbitrary reordering.
Entry, valid external notifications and lifecycle refresh trigger rounds; no
periodic polling, terminal-brand detection or guessed RGB cube.

Within the same physical stream, a refresh merges newly observed entries with the
last completed observations; missing entries remain last-observed, not silently
zero. A fresh controller starts with its own known/unknown set. Valid OSC21 Dynamic
special-role evidence outranks ambiguous legacy RGB evidence. Do not render a
client proposal before the Session returns its authoritative effective update.

Current Active fence calls tcflush and loses partial frames. Refactor narrowly:
physical reply framing survives keyboard admission epochs; retain the sole-reader
join boundary, drain through that decoder while discarding old keyboard units,
then admit a new keyboard epoch. Do not blindly flush unparsed bytes or resurrect
stale keyboard input. Mark partially started keyboard/paste units with their epoch;
hand off decoder state across replacement. Preserve existing complete-paste resume
and stale-reader race guarantees. Drain remains bounded/cancelable; on exceeded
bound fail the transition cleanly instead of dropping framing silently.

Probe physical2031 before changing it. Preserve pre-enabled/permanently enabled;
enable only a reported reset mode, recording ownership. Unsupported/permanently
reset remains unchanged. Guard restores only ZTerm's own enable on all existing
cleanup paths before raw-mode release. Raw outer997 never forwards directly to a
child; Session authorization/state/subscription decides canonical child reports.

## Presentation

Keep separate semantic composition and physical resolved baseline. Existing
presenter baseline supplies fallback history rows; never bake colors, selection or
cursor overlays into that semantic fallback. Resolve only in the sole presenter.
A full content repaint on changed effective color identity is acceptable initially;
no premature sparse-palette optimization. Retain one buffered DEC2026 transaction,
one flush, and commit all candidate state only after successful output.

Known defaults/indices become RGB; unknown values preserve source-aware inherited
SGR. Mode5 mapping is already effective state; don't double-apply it. Child RGB
stays literal outside requested overlays. Fill erased blanks with child background.
Status/gutter/notices use the outer/base context, not app overrides.

Selection resolves custom roles after cell inverse and does not alter text/source
identity. Cursor paints the actual cell, not the pen template. Explicit cursor or
cursor-text override (including Dynamic) uses a steady software block and hides
native cursor while maintaining CUP for IME. Normalize wide continuations to their
head and paint the whole span; preserve combining/blank glyphs. Movement, visibility
and native/software transitions dirty old/new spans. No software overlay in history
or for hidden cursor. Reset removes overlay and returns native behavior. No physical
OSC4/10/11/12/17/19/21 setters, physical color stacks or write-to-detect probes.

## Risks and operational boundaries

- Outer unsupported queries leave values unknown; physical underline fonts and
  current native cursor shape remain outside ZTerm's control.
- No-ID replies and input-fence lifetime require focused regression tests; do not
  claim arbitrarily delayed/reordered host responses can be authenticated.
- One color/query owner is necessary; sharing palette authority with Alacritty
  would create set/query/reset divergence, especially for Dynamic roles.
- Software custom-color cursor is a steady block, not a new shape/blink feature.
- Protocol changes require matching endpoints. No persisted user-color migration
  or destructive rollback is needed; revert the cohesive code change if necessary.
- The screenshot cause remains unproven; application-neutral checks are the primary
  acceptance evidence, with Herdr/Pi smoke when the actual environment is available.
