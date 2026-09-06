# Investigate unresponsive input after zterm detach and reattach

## Goal

Explain why reattaching to a retained Session running Herdr displays the screen
but does not respond to input, and repair the shared attachment initializer with application-independent
regression coverage.

## Background and confirmed findings

The user reports `zterm connect dev`: start Herdr, detach with Ctrl+] then `.`,
reconnect, see Herdr's screen, but only zterm's local detach shortcut responds.
The local environment is zterm 0.1.20 and Herdr 0.8.2; source baseline is main
997a731. Previous terminal context was Ghostty 1.3.1 and Doubao 0.9.7.

The failure is reproduced with an isolated local Herdr Session and a generic
Alternate-screen raw child, including legacy keyboard mode. Generic Main-screen
children remain interactive after reattach. Observation proves the CLI sends a
redundant same-size resize from a provisional Main-width baseline, enters
Synchronizing and drops input before IPC. A same-size resize yields an ordinary
delta and does not produce the barrier needed to leave that state. The same bad
baseline misses a required resize when reattaching a Main-screen Session from
a wider outer terminal.

Classification: **local implementation defect in attachment geometry
initialization**, not a Herdr/IME-specific shortcut issue. The existing host
snapshot and CLI resize owners suffice; restore their contract. Evidence,
precise code anchors and rejected alternatives are in
[research/findings.md](./research/findings.md).

## Requirements

- R1: Use private state, daemon and Herdr socket/config; preserve all existing
  user Sessions, daemons, pairing, configuration and remote state.
- R2: Compare initial and repeated attachment, with application-neutral input
  capture plus a real Herdr command effect; do not equate screen display with
  useful interaction.
- R3: Trace the input-admission, resize and synchronization chain, classify the
  root cause, distinguish observation from inference and explain sibling cases.
- R4: Propose an application-independent repair and regression scope that keeps
  existing prefix, clipboard, keyboard-mode and ordinary-delta ACK contracts.
- R5: All work runs directly in the main agent, without sub-agents. The user approved the bounded implementation with “好 那你修复吧”.
  The subsequent “走发布流程吧” authorizes commit, wrap-up and release.

## Investigation acceptance

- [x] A1 (R1/R2): Private baseline records versions, exact lifecycle and actual
  child input. All private daemon and Herdr cleanup commands returned 0.
- [x] A2 (R3): Causal trace contains provisional/actual/desired sizes, the
  redundant resize, ordinary delta, blocked input and still-working Detach.
- [x] A3 (R2/R4): Repair plan includes retained Alternate reattachment, useful
  input and the Main/new-width sibling, with generic evidence.
- [x] A4 (R1/R5): No product code or existing user/remote Session was changed;
  limits and direct single-agent execution are recorded.

## Approved implementation scope

[design.md](./design.md) and [implement.md](./implement.md) propose reconciling
initial resize state from the authoritative snapshot and latest physical layout.
Acceptance must prove no redundant resize for equal geometry, one real resize
for different geometry, then useful input after the proper Active barrier.
Preserve physical resizes observed during initialization and the existing
queued-delta safeguards. Implementation was explicitly approved on 2026-09-06.

## Limits and exclusions

The exact dev daemon version/network route and physical keyboard stream were
not captured. Post-fix acceptance covers private local Sessions plus a uniquely
named disposable paired dev Session; see verification.md. No Herdr special case, IME adaptation, keyboard-mode change,
transport redesign, installed-binary update or sub-agent work is included.
Release 0.1.21 was subsequently authorized after implementation acceptance.

## Repair acceptance

- [x] A5: Real CLI reattachment to retained Main and Alternate screens accepts
  subsequent child input, preserving Session identity and terminal cleanup.
- [x] A6: Equal actual/desired geometry sends no resize; changed geometry reaches
  the latest desired size through the existing snapshot/Active barrier.
- [x] A7: Physical resizes during prepare/acknowledgement use the authoritative
  screen and latest dimensions; startup input remains fenced.
- [x] A8: Direct review, relevant regression tests and local quality gate pass;
  Herdr smoke and platform/route limits are recorded.
