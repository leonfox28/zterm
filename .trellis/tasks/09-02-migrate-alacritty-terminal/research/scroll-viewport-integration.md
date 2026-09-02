# Scroll Viewport Integration Research

## Reviewed state

- Scroll research revision: `db9daa95c9698dfbecc0033edd847fb30b9e1c27`; implementation baseline was
  fast-forwarded to released `main` `bce1d57d8bd91b5c0e58bcdf422d899cedcd7fac` before product edits. The
  intervening changes only update release metadata and the daemon-autospawn regression fixture.
- Product engine: official `alacritty_terminal 0.26.0`, behind `zterm-terminal`.
- Current terminal wire kinds end at 314; history request/page are 312/313.
- `Capabilities::HISTORY_PAGING` is bit 17 and is advertised by the local connection broker and
  terminal service path. Current main already assigns bit 18 to `AGENT_EVENTS`; the viewport must
  therefore use the next free bit 19 rather than assuming it immediately follows history paging.
- Terminal content/control frame limits remain 8 MiB / 1 MiB.

## Confirmed capture failure

`crates/cli/src/terminal_ui.rs` asks the physical terminal for all-motion SGR mouse with
`HOST_INPUT_CAPTURE = DECSET 1003 + DECSET 1006` when entering raw UI. However,
`crates/terminal/src/ansi.rs::controlled_mode_reset` emits `DECRST 1003 + DECRST 1006` in every
daemon-authored full screen and delta baseline. `TerminalRenderer::apply_snapshot` never restores
capture, and `apply_delta` restores it only when `child_transition_disables_host_capture` detects a
narrow semantic transition. An ordinary initial snapshot or ordinary output delta therefore stops
the outer terminal from sending wheel reports.

The correct boundary is unconditional transaction ordering:

```text
daemon-authored terminal ANSI -> Zterm status/chrome repair -> HOST_INPUT_CAPTURE -> flush
```

Child modes remain the daemon-authored `TerminalModes`; the outer capture write must not mutate or
fabricate those modes. `TerminalGuard` remains the sole cleanup owner.

## Current page browser limitation

`ViewportController` currently holds `Live | History | ResumePending`. A history request returns
only formatted negative-grid rows. On the first live wheel-up, `navigate(older, amount)` discards
`amount`, requests `Newest`, then places the view at the bottom of that history page. It cannot show
the native viewport for offset three:

```text
history Line(-3)
history Line(-2)
history Line(-1)
live    Line(0)
...
```

The current cursor is stable for bounded page navigation and remains useful for old-peer fallback,
but a continuous viewport needs a full-height projection spanning history and live rows.

## Existing cross-layer contracts to preserve

- `zterm-core` owns transport-neutral DTOs; proto owns wire representation; Alacritty types never
  cross the `zterm-terminal` boundary.
- Session actor attachments already own per-client checkpoints and sync state. Presentation scroll
  belongs there, not in the shared `TerminalModel` and not in persisted remote-resume identity.
- `TerminalViewCommandWriter` submits typed commands through a bounded channel, receives an ack, and
  observes correlated results through the bounded event stream. Closure races are correlated with a
  queued terminal outcome.
- Local IPC and remote bridge validate kind, request ID, attachment ID, message shape and payload
  size. New remote messages must be capability-gated; unknown kind probing is forbidden.
- Current history permits one outstanding request. Viewport actions should keep the same invariant
  and coalesce burst input instead of creating an event-sized queue.
- Snapshot/delta are live authoritative state. While history is visible, the CLI observes revision
  and child modes without overwriting the frozen history presentation; ordinary input requests a
  live replacement before forwarding exactly once.

## Alternatives evaluated

### Let the physical terminal scroll its own scrollback

Rejected as the product model. Zterm deliberately captures mouse to route nested TUI input, and its
outer terminal scrollback is a replay artifact rather than the daemon-authoritative history. It
cannot provide portable metrics, absolute drag, reconnect behavior or the Android contract.

### Mutate Alacritty `display_offset`

Rejected. One Session model is shared by attachments, while offset is presentation state. Calling
`scroll_display` for one attachment would contaminate another attachment or require risky temporary
mutation of the authoritative model. Direct `Line` projection is sufficient and remains read-only.

### Extend only the legacy history page cursor

Rejected for the continuous path. Its rows are history-only, so offset three cannot include the
remaining live screen. Page direction also cannot express an arbitrary scrollbar target cleanly.
The API remains unchanged only as mixed-version fallback.

### Client-only reconstruction

Rejected for the CLI. It receives canonical ANSI snapshots/deltas, not a retained semantic row grid;
parsing them into another terminal state would duplicate the engine and expand the security surface.

### Dedicated semantic viewport action/frame

Selected. The terminal-owning daemon keeps a small attachment-local action baseline and asks the
existing model for one immutable full-height row projection. This supports relative wheel, absolute
track/drag, explicit metrics, capability negotiation and later Android presentation without sharing
CLI glyphs or adding another VT parser.

## Projection and rebase rule

For height `R`, retained history `H` and clamped offset `O`, frame row `i` reads Alacritty
`Line(i - O)`. Offset zero remains the existing live snapshot/delta path. Within one history epoch,
growth from `previous_max` to `current_max` is added to the saved offset before a relative action so
the old content stays pinned. If resize, clear or capacity eviction invalidates identity, the server
clamps to current bounds and returns a complete `Rebased` frame; it never mixes rows from epochs.

## Layout consequence

The approved CLI main-screen layout is `N-1` child columns plus one Zterm gutter when usable width is
greater than four. History appearing does not resize. Alternate screen clears/reclaims the gutter
and resizes once to `N`; exit resizes once back to `N-1`. The child may redraw once before and once
after the SIGWINCH. Correctness criteria are eventual matching geometry, no resize loop, no input
loss and no stale gutter—not the absence of a second redraw.

The gutter is outside the child rectangle. Direct wheel/click/drag in that explicit chrome column is
host-owned and never clamped into the child's last cell; all content-rectangle input continues to use
standard child mouse/alternate-scroll modes.

## Bug Analysis: capture and viewport state crossed layer boundaries

### 1. Root Cause Category

- **Category**: B/D/E — cross-layer contract, test coverage gap, and implicit assumption.
- **Specific cause**: the UI enabled physical mouse capture once but daemon-authored repaint bytes
  later disabled it; child modes and outer capture were treated as one state. The first sync fixes
  also treated an in-epoch replacement snapshot as equivalent to a transport reconnect, while a
  strict server `Active` check assumed commands and snapshots are observed in one total order even
  though they travel in opposite directions on a duplex stream.
- **Evidence update**: the initial prior split was capture loss 45%, missing history state 35%, and
  outer-terminal behavior 20%. Exact output bytes showing `DECRST 1003/1006` after capture raised
  capture loss above 90%. After restoring capture, deterministic Session/remote tests and the real
  outer-PTY fixture discriminated remaining model/presentation/sync races. Confidence in the final
  layered cause is above 95% because the focused regressions and full workspace gate pass.

### 2. Why Earlier Fixes Were Incomplete

1. Conditional capture restoration after a child-mode transition fixed only one path; ordinary
   snapshot/delta/resync/history transactions could still end with capture disabled.
2. Resetting history on every `Synchronizing` event removed stale pixels but lost the valid pinned
   attachment baseline and sent a redundant sync request during host-initiated replacement.
3. Requiring server `Active` for every input/read-only request ignored a command sent while Active
   that arrived after the host began a replacement snapshot. Broadly allowing `Awaiting` would have
   admitted fresh/takeover controllers, so the final fence also needs `ever_active`, exact
   controller, generation, and stream epoch.
4. Pure parser/UI tests did not prove that a real outer PTY emitted one SGR wheel report and received
   the expected three-history-plus-live repaint; the deterministic multiprocess fixture closed that
   evidence gap without sleeps or product test hooks.

### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Architecture | Keep scroll metrics on `ActorAttachment`; model projection stays immutable | Done |
| P0 | Output contract | Compose child/history, chrome, host capture, then exactly one flush | Done |
| P0 | Sync identity | Distinguish replacement sync from reconnect and scope `ever_active` to exact controller/epoch | Done |
| P0 | Test coverage | Byte-exact renderer tests plus real outer-PTY three-line scroll regression | Done |
| P1 | Compatibility | Capability-gate 315/316 and complete lost read-only controls as correlated Gap | Done |
| P1 | Runtime acceptance | Record macOS/Linux local/direct/relay nested-TUI smoke before Android | Pending |

### 4. Systematic Expansion

- **Similar issues**: future selection/clipboard/gesture controls, status overlays, and Android
  native chrome can repeat the same shared-model versus per-view ownership error.
- **Design improvement**: every new terminal presentation feature must name its model owner,
  attachment baseline, wire correlation, capability gate, and survival/reset transitions.
- **Process improvement**: review the complete model -> Session -> proto -> bridge -> CLI transaction
  and require at least one real boundary fixture, not only same-layer unit tests.

### 5. Knowledge Capture

- [x] Updated terminal-model, terminal-driver, session-service, core-wire-domain,
  local-daemon-IPC, and transport-auth executable contracts.
- [x] Updated cross-layer and cross-platform thinking guides.
- [x] Added direct model isolation/resize/eviction/clear regressions and the real outer-PTY scroll
  fixture.
- [ ] Record the external macOS/Linux direct/relay acceptance matrix before Android work.

This product repository has no `src/templates/markdown/spec/` tree, so there is no generated spec
template counterpart to synchronize; the project-owned `.trellis/spec/` files are the source of
truth.
