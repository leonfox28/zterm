# Implementation plan (approved 2026-09-06)

Main agent only. No sub-agents for implementation, research or review.

## Completed investigation

- [x] Create the explicitly requested Trellis task on main 997a731 / v0.1.20.
- [x] Record the clarified `zterm connect dev` workflow and working local detach.
- [x] Reproduce first attach/detach/reattach in private local state with real CLI.
- [x] Compare generic Main flags 7/15, Alternate flags 7/0 and real Herdr 0.8.2.
- [x] Observe actual input at the raw child / shell marker, not only screen output.
- [x] Add metadata-only observation to isolated source copies and trace the
  redundant same-size resize, ordinary delta, Synchronizing state and dropped input.
- [x] Confirm the Main/new-width sibling misses a required resize.
- [x] Record local implementation classification, source anchors, history,
  evidence limitations and private cleanup in research/findings.md.

## Implementation gate

- [x] Present the findings and bounded repair scope for review.
- [x] After explicit implementation approval, start the task and load applicable
  trellis-before-dev guidelines directly. Do not infer release authorization.

## Implemented work

- [x] Add a failing regression through the real initializer with retained
  Alternate mode, successful local detach and failed post-reattach input.
- [x] Reconcile the resize baseline with initial snapshot geometry before first
  Active, then compare correctly projected desired geometry.
- [x] Preserve the latest physical resize across inactive waits, using known
  screen identity once available; avoid Main-only re-projection after snapshot.
- [x] Cover equal and different geometry, Main/Alternate, and queued startup
  resize without creating artificial delta ACKs or bypassing input fences.
- [x] Extend the generic outer-PTY acceptance to prove post-reattach keyboard
  input and retained Session identity; keep Herdr as a supplementary smoke case.
- [x] Direct full-scope review and update the executable layout/input contract.

## Checks

- Relevant CLI initializer/coalescer and queued-delta tests.
- `cargo +1.98.0 test -p zterm-cli --lib --test daemon_autospawn --all-features --locked`.
- Explicit formatting checks for include! source files as well as workspace fmt.
- Workspace Clippy and the required local `just check` gate once code is final.
- Isolated local Herdr reattach with a real marker effect and a paired-route
  disposable-session smoke if available; preserve evidence limits.

Investigation baseline remains preserved. Product implementation and direct
verification are complete; see verification.md for commands, red/green evidence,
paired-route cleanup reconciliation and platform limits. The user authorized commit, wrap-up and release with “走发布流程吧” on
2026-09-06; proceed through the release operator for version 0.1.21.
