# ZTerm 颜色与主题协议兼容性

## Goal

Make ZTerm a coherent virtual color terminal: nested applications can discover,
query, change, reset and save colors, and the local/remote view agrees with those
replies. Complete C05–C18, including the C13 mixed-SGR defect, as one delivery
before building theme selection UI.

On 2026-09-06 the user requested theme brainstorming/task creation, prioritized
color reporting, requested a standards-gap inventory, then requested filling all
listed missing features and compatibility defects. The latest request supersedes
the earlier reporting-only scope. P8 now covers this foundation; P1–P7/C19 remain
deferred. Screenshot attribution is not a prerequisite to delivery.

## Background and evidence

- ZTerm is a raw-terminal CLI inside an outer terminal (README.md). Prior remote
  UX selected defaults/inverse status and excluded configurable colors only from
  that task (.trellis/tasks/archive/2026-09/08-31-remote-terminal-ux/prd.md:51,79).
- Default/Indexed/RGB cells survive the model (crates/core/src/terminal.rs:135)
  and are emitted directly (crates/cli/src/terminal_ui/ansi_presenter.rs:320,342).
  C01–C04 remain regression requirements within the current rendition subset.
- Color OSC is outside ingress policy; engine color callbacks are dropped
  (crates/terminal/src/ingress.rs:524; crates/terminal/src/engine.rs:109).
  Visual inheritance therefore does not currently provide queryable RGB values.
- The Herdr/Pi screenshot shows low contrast but its exact cause is undetermined.
  Missing protocols are independently confirmed; app configuration/retained RGB
  may also contribute. See [diagnosis](research/nested-application-theme-diagnosis.md).
  No application-specific branch or promise to recolor authored RGB is allowed.
- The [C01–C19 audit](research/color-theme-capability-audit.md) preserves baseline
  evidence and distinguishes established interoperability from modern optional
  extensions. This task includes the listed extensions without claiming every
  terminal implements them.
- One controller per Session, with explicit takeover, already exists
  (.trellis/spec/backend/session-service.md:108). Its frontend supplies base
  observations; the Session model owns effective virtual colors.

## Requirements

| ID | Mapping | Observable requirement |
| --- | --- | --- |
| R01 | P8, C05–C08 | Acquire outer defaults, 256 palette entries and supported special colors. Implement OSC 4/10/11/12/17/19 query/set and 104/110/111/112/117/119 reset. Multi-value controls and interleaved queries observe stream order. |
| R02 | P8, C09/C10/C17 | Appearance inquiry 996/997, mode 2031 set/reset/query and subscribed notifications after changed controller base/appearance, including same-appearance palette changes. Own app writes do not create external-theme notification loops. |
| R03 | P8, C11 | One Session color owner handles base, overrides, stack and lifetime. Snapshot/delta/history/resume carry coherent effective state; color-only updates redraw live and retained history without changing text/scroll position. |
| R04 | P8, C12/C13 | Preserve single/double/curly/dotted/dashed underlines and default/indexed/RGB underline colors. Mixed SGR retains supported siblings beside 58/59. |
| R05 | P8, C14 | Cursor/local selection colors, including independent cursor text and selection fg/bg roles. Explicit cursor customization uses a software steady block; reset restores inherited native behavior. |
| R06 | P8, C15 | Bounded xterm push/pop/store/restore/report and kitty OSC 30001/30101 aliases share a stack. Restore saved effective values; reset uses latest controller base. |
| R07 | P8, C16 | OSC21 for indices 0–255, foreground/background, cursor/cursor_text and selection_foreground/selection_background. Distinguish fixed, dynamic/unset, unavailable and unsupported. |
| R08 | P8, C17 | DECRQSS current SGR, relevant XTGETTCAP color/underline capabilities, and DECRQM for added color modes and currently exposed supported modes. Negative results for unsupported requests. |
| R09 | P8, C18 | Private mode 5 virtual screen reverse video with documented xterm-style color mapping. Independent SGR7, literal child RGB, unaffected physical chrome and appearance preference. |
| R10 | all | Input/reply separation, bounded parsing/replies, controller authorization, atomic presentation and Session isolation. Child color setters never modify the outer terminal's global colors. |

R04 owns a confirmed rendition-correctness defect at
crates/terminal/src/ingress.rs:487,657: `ESC[31;58;5;2mX` loses the entire SGR,
including red foreground. Cover 59/reset siblings too. Existing coverage at
crates/terminal/tests/security_policy.rs:343 only protects RGB channel values
58/59. This is not a proven cause of the screenshot.

[Protocol contracts](research/color-protocol-contracts.md) define required grammar,
negative responses, limits and screen mapping for R01–R10.

## Acceptance criteria

| ID | Requirements | Required evidence / result |
| --- | --- | --- |
| A01 | R01/R07/R08 | Application-neutral corpus covers supported query/set/reset, canonical/negative replies. Whole, one-byte and varied chunking agree. Set/query/set replies capture intermediate values. |
| A02 | R01/R02/R10 | Bounded startup probe supplies available observations before newly created PTY output. Partial/nonresponsive outer terminals stay usable; no fabricated RGB/appearance. Missing response boundaries cannot authenticate later rounds. |
| A03 | R02/R03 | Subscribed child is notified after changed base commits, then requeries updated colors, including same-appearance changes. Unsubscribed children and own app writes produce no unsolicited external-change report. |
| A04 | R03/R10 | Local/remote create/attach, occupied attach, pending/committed takeover, detach and reconnect obey state lifetime. Old controller proposals are rejected; affected prepared snapshots are replaced/acknowledged before activation. |
| A05 | R03 | Zero-row color delta recolors defaults/indices, erased blanks and retained history. Literal RGB, history position/text and valid selection stay stable. Older history responses cannot roll colors back. |
| A06 | R04/R08 | Mixed 31/38/48/0 with 58/59 preserves supported siblings. Underlines survive projection/wire/history/presentation; DECRQSS reports actual pen. Existing indexed/RGB corpus remains green (crates/terminal/tests/terminal_corpus.rs:131). |
| A07 | R05/R09 | Selection/software cursor precedence handles actual wide/combining/blank glyphs; movement repairs old/new spans. Hidden/history cursor stays hidden; reset returns native behavior. Mode5 mapping and SGR7 remain independent. |
| A08 | R06/R07 | Stack aliases, slots, nesting, under/overflow and reports match contract. Push → temporary theme → new controller → pop restores saved known values; reset then uses new base. Dynamic differs from fixed black. |
| A09 | R10 | Fragmented outer replies across input epochs never become keys, cancel prefix or alter selection. Apparent replies inside bracketed paste stay literal. Stale-key and exact-once resume/paste tests remain green. |
| A10 | R03/R10 | Replies, base changes and revision publication are ordered. Malformed frames/output failures never partially commit. Required color metadata and canonical request-kind mismatch are checked without fallback. |
| A11 | R10 | Detach/end/error/signal cleanup preserves physical global colors and restores only a 2031 subscription ZTerm enabled. Colors do not leak between Sessions. |
| A12 | all | Focused behavior checks plus native `just check` pass. Record physical-terminal smoke and hosted-only limits honestly; source inspection is not Herdr diagnosis or Linux runtime evidence. |

## Decisions and limits

- One engine-owned metadata object; Alacritty remains the sole grid/SGR engine.
  No duplicate upstream palette, second CLI emulator or dependency upgrade.
- Unreported colors remain Unknown with inherited rendering. Last observed base
  survives detach; a new controller supplies its own known/unknown base. Child
  queries never wait for a viewer/network callback.
- Appearance is terminal-reported preference, independent of app RGB background
  or screen inversion. Unsupported outer appearance cannot supply reliable 996
  replies/live 997 notifications; no brightness or brand heuristic.
- Overrides survive detach/reconnect/takeover/main-alternate switching until
  explicit reset or Session end. RIS clears overrides/stacks/added modes while
  retaining base. App exit inside a shell is not a reset signal.
- Paint known colors with SGR; never forward global setters. Software cursor is
  a steady block because current DTOs have no shape/blink; inherited native shape
  remains outer-terminal behavior.
- Keep wire v2 and one representation; fresh canonical create/attach request kinds
  reject old/new mismatches before Session/PTY effects. Matching CLI/daemon/host
  binaries are required; no downgrade, forced restart or old-process migration.
- One cohesive task, ordered stages: independent shipping of shared model,
  protocol and rendering changes would leave query/display inconsistencies.

## Out of scope and preserved theme backlog

| ID | Deferred proposal and acceptance intent |
| --- | --- |
| P1 | User-selected defaults/base16 plus UI theme; decide indices16–255 policy. Authored RGB remains literal outside intentional overlays. |
| P2 | Follow outer, dark/light and optional licensed presets; agreed surfaces stay legible. |
| P3 | Semantic status/scrollbar/selection/notice theme roles. No content/layout redesign. Current defaults/inverse: crates/cli/src/terminal_ui/composition.rs:233,257,278. Status stays `<device> | local` or `<device> | <path> | <latency>` (crates/cli/src/terminal_ui.rs:1412). |
| P4 | Discover, preview representative colors/content/UI, select/reset via CLI or picker; interaction deferred. |
| P5 | Local saved default and temporary override across local/remote reopen, without temporary choices rewriting preferences. Existing daemon config v1 rejects unknown fields (crates/daemon/src/config.rs:13,18), in effective account `.zterm/config.toml` (crates/platform/src/user_state.rs:84; .trellis/spec/backend/effective-user-state.md); client preference storage deferred. |
| P6 | Live theme apply without replacing Session/losing input, cancelable preview, coherent history/output/resize/exit; cross-client preference propagation deferred. |
| P7 | Validated custom local files preserving last valid choice; format/import/default policy deferred. |

No marketplace, device sync, per-device/project/Session rules, scheduled themes,
graphical editor, third-party theme importer or status layout changes. Ctrl+]
owner remains unchanged (crates/cli/src/terminal_ui/prefix.rs:7,10,15).
Fonts, ligatures, transparency, images, native chrome, direct OS integration,
graphics/Tektronix/pointer/visual-bell color families and unrelated capabilities
are outside this opaque text-color contract. Unsupported OSC21 vendor keys get
negative answers rather than silently accepted state.

## Artifact status

User approved implementation on 2026-09-06 and requested no subagents.
R01–R10/C05–C18 are implemented inline; P1–P7/C19 remain the theme UI backlog.
Code, protocol fixtures and executable specs are in the working tree. Native
`just check` passed (560 tests, 0 failures); see execution.md for acceptance mapping,
observed failures/fixes and honest physical/hosted-only evidence boundaries.
No production daemon, remote deployment, commit or release has been performed.
