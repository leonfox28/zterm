# Fix Android Pinyin composition being submitted prematurely

## Goal

Fix the reproduced Android editor-contract defect so Pinyin remains local during
composition and confirmed Unicode is submitted exactly once. The user approved
the described correction after reviewing the diagnosis and requested a new branch
on 2026-09-20. Implementation branch: `fix/android-ime-composition`.

## Confirmed background

- Baseline: clean `main` at `796e13c`, zterm 0.1.33.
- Supplied APK: `/Users/huyuanzhe/Downloads/doubaoime_v1.4.5_100405008_official_arm64_release.apk`.
- A new API 36 arm64 emulator reproduces the issue when Doubao uses
  **布局与显示 → 输入拼音显示 → 显示在输入框**.
- `nihao` then selecting `你好` sends `nnini hni ha你好 ` to the host. Candidate-bar
  display and ordinary Android text fields work.
- Classification: local Android editor defect. `TerminalView.kt:624` omits
  cursor/composing metadata; invalid context queries precede IME resets and
  `TerminalView.kt:629` submits the old Pinyin. Full source anchors and evidence:
  [research/reproduction.md](./research/reproduction.md).

## Requirements

- **R1:** Use the exact supplied APK and actual software-key taps; record versions.
- **R2:** Distinguish preedit from child input and compare an ordinary-field control.
- **R3:** Classify the owning defect with source/callback evidence, separating
  observed facts from inference.
- **R4:** Preserve reproduction steps, artifacts, workaround and follow-up scope.
- **R5:** Maintain coherent Editable selection/composing metadata, context queries
  and batch selection notifications in the existing Android input owner.
- **R6:** Preserve valid finish, Unicode/deletion, rejection, epoch isolation and
  healthy resize; verify both Doubao display modes using the supplied APK.

## Acceptance Criteria

- [x] **AC1 / R1:** APK installed/selected; exact environment and hashes recorded.
- [x] **AC2 / R2:** Actual Pinyin taps reproduce premature input; host bytes,
  screenshots/video and ordinary-field controls saved.
- [x] **AC3 / R3:** Read-only callback capture establishes inconsistent editor
  state and the premature commit path; classification documented.
- [x] **AC4 / R4:** Research report, tested workaround and follow-up design/plan saved.
- [x] **AC5 / R5:** An application-independent editor-contract regression fails
  on the old code and passes after correction, including before/after-cursor
  queries and composing replacement without premature child input.
- [x] **AC6 / R6:** Explicit finish is idempotent; cursor/deletion/Unicode and
  rejected/stale input behavior remain correct; existing IME lifetime checks pass.
- [x] **AC7 / R6:** Real Doubao taps in both display modes produce exactly the
  chosen Chinese text with no Pinyin prefix or spurious space.

## Scope

Scope now includes the approved Android correction, meaningful regressions,
real-keyboard acceptance and spec updates. Release publication and physical-phone
acceptance are excluded. Preserve user Sessions/identity; use the isolated test
environment recorded in research. No unresolved product decision remains.
The approved boundary is in [design.md](./design.md), with checks in
[implement.md](./implement.md).
