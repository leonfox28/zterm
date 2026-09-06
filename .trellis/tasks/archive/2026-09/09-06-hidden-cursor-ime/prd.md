# Preserve hidden cursor positions for Chinese IME

## Goal

Diagnose the Chinese IME candidate window appearing at the terminal's top-left
when Pi runs inside Herdr through Zterm, and prepare a bounded Zterm correction
so the candidate window follows the input location.

## Background

- User-reported matrix: Zterm → Herdr → shell works; Zterm → Herdr → Pi
  misplaces the IME candidate window; Zterm → Herdr → Codex works. Herdr
  without Zterm works with both agents.
- Locally installed versions observed on 2026-09-06: Zterm 0.1.22, Herdr
  0.8.2, Pi 0.85.1. The versions/configuration on the connected `dev` host
  have not been independently checked.
- Before the repair, Zterm's active live composition gated cursor coordinates on visibility at
  `crates/cli/src/terminal_ui/composition.rs:273`. A hidden cursor is replaced
  by `(0, 0)`; the presenter emits those coordinates at
  `crates/cli/src/terminal_ui/ansi_presenter.rs:227`.
- Pi positions its hidden terminal cursor for IME. Herdr 0.8.2 retains a
  hidden cursor anchor. Detailed causal evidence belongs in
  `research/diagnosis.md`.

## Requirements

- R1: Preserve the authoritative cursor row and column for an in-bounds cursor
  in the active live viewport, independently of whether its glyph is visible.
- R2: Respect the child's cursor visibility; restoring IME position must not
  force a native cursor or add an extra drawn cursor.
- R3: Preserve existing history, synchronization/reconnection, bounds, chrome,
  and software-cursor behavior.
- R4: Explain the reported application matrix with evidence and distinguish
  deterministic code findings from manual IME validation still needed.

## Acceptance Criteria

- [x] A child at a nonzero input location yields the same outer-terminal
  position with cursor visibility enabled or disabled (R1, R2).
- [x] Cursor-only movement while hidden updates the outer position even when
  terminal cells are unchanged (R1).
- [x] Actual composed/presented output keeps the cursor hidden, repairs the
  position after row/chrome painting, and does not move it to the origin (R1, R2).
- [x] Existing visible/native and custom software cursors, main/alternate
  screens, history, reconnect/synchronization and invalid-coordinate policies
  retain their intended behavior (R3).
- [x] Confirm the original Chinese IME scenario with the candidate build and
  retain the comparison evidence and version/verification limits (R4).
  The user confirmed the candidate works; comparison cases were not individually
  re-reported after the repair. See `research/validation.md`.

## Out of scope

- Herdr/Pi changes, agent-name detection, user configuration changes, protocol
  redesign, dependency upgrades, or automatic installation.

## Authorized release follow-up

After accepting the fix, the user requested the formal release workflow.
Commit the reviewed changes, archive the completed repair task, and use the
repository's existing operator to prepare the next patch release and proceed
through protected PR/main CI, merge, tagging, signing, and immutable publication.
The release request does not authorize replacing the user's installed binary.

## Execution status

The user approved the reviewed repair plan on 2026-09-06. The local compositor
correction and integrated regression are implemented. The regression failed on
the original code and passes after the correction. The user then tested the
candidate and confirmed on 2026-09-06: “测试了没有问题了”. Native acceptance is
complete through user testing. Automated checks have passed; the task is ready
for commit and archive. The subsequent release request authorizes that commit
and the release follow-up above. The installed binary is unchanged.
See `research/validation.md` for checks and the candidate-build smoke procedure.
