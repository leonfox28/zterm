# Implementation and validation plan

## Phase gate

- [x] Task creation consent received.
- [x] Record causal evidence and local-defect classification.
- [x] Run application-neutral probe against unmodified source blocks.
- [x] Present the final planning summary and obtain subsequent implementation approval.
- [x] Run `task.py start` and load current `trellis-before-dev` context.

The inline context exemption required the script's documented
`--allow-empty-context` flag; no sub-agent was dispatched.

## Execution

1. Add a focused regression through actual `ComposedFrame` and
   `DesktopPresenter` using a non-origin hidden cursor and unchanged cells.
   Assert final ANSI CUP and hidden mode; then move the hidden cursor and assert
   a new physical position is emitted. Include both main/alternate live layouts
   where fixtures permit. Confirm the regression fails on the current code.
2. Preserve active/live/in-bounds coordinates independently of visibility in
   `composition.rs`. Use `surface.cursor.visible` for the composed visibility.
3. Reuse existing history/reconnect/software-cursor tests; add only a necessary
   boundary assertion if those fixtures do not cover the preserved policy.
4. Run relevant CLI checks, then build a local candidate if native smoke is
   available. Do not mutate user settings to hide an unfixed behavior.
5. Verify original Zterm → Herdr → Pi Chinese IME scenario; compare Herdr shell,
   Herdr Codex, and direct Herdr. Record actual endpoint and terminal versions.
   Keep GUI verification marked pending if no native session is available.
6. If corrected CUP still fails native IME positioning, capture evidence and
   return to planning for the synchronized-output boundary; the current
   correction alone is not end-to-end acceptance.
7. Clarify the hidden-position invariant in the owning backend spec and update
   the task evidence. Load `trellis-check` before concluding implementation.

## Validation commands

```sh
cargo test -p zterm-cli --lib hidden_cursor
cargo test -p zterm-cli --lib
cargo fmt --all -- --check
cargo clippy -p zterm-cli --all-targets -- -D warnings
cargo build -p zterm-cli --bin zterm
```

The first command assumes the new regression name includes `hidden_cursor`.
The research probe is diagnostic evidence, not the acceptance test. Full
workspace checks should follow any applicable quality-gate requirements after
loading their current instructions; do not repeat already-passing checks without
a new change or unresolved issue.

## Review and rollback points

- Review that only the child-hidden active/live branch changes; history and
  transport suppression remain explicit.
- Verify cursor-only frame differences are not discarded by baseline equality.
- Preserve sole-writer ownership, one buffered write/flush, color resolution,
  status/gutter geometry, and selection behavior.
- Rollback is the task-specific compositor/test/spec diff; no schema migration,
  daemon restart, user configuration change, installation, or release is needed.

## Outcome

Implementation, the red/green regression, CLI tests, Clippy, portable CI policy,
candidate build, and spec clarification are complete. The user confirmed the
native Herdr/Pi Chinese IME scenario is fixed on 2026-09-06. No post-sync anchor
change was needed for the reported environment. See `research/validation.md`.
The user subsequently requested the formal release workflow, authorizing the
proposed fix commit, archival, and the existing protected release operator.
