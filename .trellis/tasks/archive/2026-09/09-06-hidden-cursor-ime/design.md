# Preserve the active hidden cursor anchor

## Decision and ownership

The confirmed defect is a local violation of an existing representation:
`TerminalCursor` and `ComposedCursor` already separate position from visibility.
`ComposedFrame::compose_inner` owns projection of the active child cursor into
the desktop content area; `DesktopPresenter` owns the final physical CUP and
visibility commands. See `research/diagnosis.md` for causal evidence.

## Proposed change

For an Active, live viewport with in-bounds cursor coordinates, always copy the
child cursor row, column, style, and visibility. Remove child visibility from
the branch that determines whether coordinates are meaningful. For history,
inactive transport, and invalid coordinates, retain the existing hidden fallback.

All snapshot/delta/live-resume callers already use this compositor. Preserve
this shared path. Cursor-only changes should naturally change frame equality
and reach the presenter without explicit repaint flags or timers.

`DesktopPresenter` should continue positioning the cursor after row/chrome
painting and then applying its visibility. Its existing software cursor path
can hide the native glyph while retaining position; it needs regression coverage,
not a second source of cursor state.

## Boundaries and compatibility

- Product edit: `crates/cli/src/terminal_ui/composition.rs`.
- Integrated regression: existing CLI terminal UI tests in
  `crates/cli/src/terminal_ui.rs` (or the most direct existing test module).
- Contract clarification: `.trellis/spec/backend/local-daemon-ipc.md`, with a
  reference to the existing IME invariant in `terminal-colors.md` if useful.
- No domain/protocol/daemon/terminal engine changes; all needed fields exist.
- No global native-cursor visibility override and no Herdr/Pi recognition.

## Synchronization risk

Resolution for the reported environment: on 2026-09-06 the user confirmed that
the candidate fixes the original native IME issue. No post-sync anchor change
was necessary. The broader compatibility question below is retained as research,
not an unresolved acceptance blocker for this task.

Herdr v0.8.2 also repeats the final anchor after DEC 2026 on non-Windows hosts.
Whether that is necessary for this user's outer terminal is not yet established.
First validate corrected positions with the user's native IME. A remaining
failure with correct emitted positions is grounds to return to planning and
review a presenter-owned post-sync anchor contract. Do not silently add arbitrary
extra output or claim the original GUI is fixed based only on Rust tests.

## Rollout and rollback

Use a local candidate build and explicit smoke evidence. Do not replace the
installed binary, restart existing user sessions, or release as part of planning.
The local compositor correction is independently reversible without data or wire
migration. Roll back only this task's edits if the hypothesis is disproved.
