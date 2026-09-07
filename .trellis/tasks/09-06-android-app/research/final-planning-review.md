# Final planning review — prototype v8 (2026-09-07)

Status: approved by the user on 2026-09-07: "好，开始开发吧，不要用子代理".
This approves the final v8 planning summary and requires inline implementation
and checking, without sub-agents. No repeated product-review approval is needed
unless the accepted scope changes materially. The user subsequently confirmed
the macOS/Linux host baseline and requested the latest remote mainline. The
branch is refreshed to `de25a38`; see `mainline-refresh.md`. The task remains
`planning` under the current turn's workflow instruction; no product code or
`task.py start` has been executed at this checkpoint.

## Agreed outcome

An Android remote terminal controller delivered as an APK, using Kotlin/Compose
for native UI and a Rust/UniFFI boundary to reuse existing Zterm semantics.
Development uses the local emulator; final acceptance uses Xiaomi 17 Pro Max.
The phone's installed OS version is recorded when available, not inferred.

## First-release behavior

- Pair from camera QR, a system-picked image, or the minimal credential dialog.
- Home restores the exact previous Session; first entry without a previous ID
  resolves zero/one/multiple Sessions through explicit New/direct target/the
  auto-expanded title list. Occupancy needs takeover confirmation; stale IDs
  never silently retarget. Long-press device removal affects only local records.
- Session switching/creation lives in the terminal title panel. Each row's
  menu has Rename/Delete, and Delete confirms ending that Session's processes.
- One continuous locally cached scrolling surface, cross-screen native-style
  selection/Copy, native IME, and the same eight persistent shortcuts. Healthy
  return from scrolling retains first input until synchronization; actual
  disconnected input is never queued for replay.
- Independent language/theme settings default to Follow system. Add persisted
  12/14/16 font sizes with preview (initial default 12) and landscape support.
- Keep a healthy attachment when backgrounded on a best-effort basis. Recover
  actual interruption against the same Session. Provide concise error/empty/
  pending states in existing routes and dialogs.

## Acceptance and boundaries

PRD requirement IDs map to explicit acceptance checks. Runtime evidence must
cover pairing and duplicate suppression, exact Session ownership/mutations,
IME and first-input behavior, Unicode/cross-page selection, cached gestures,
rotation/font/insets, recovery/background behavior, persistent settings/device
records, and signed APK updates preserving identity. Emulator evidence and
phone evidence are separate. Prototype screenshots/sample test output do not
satisfy any Android runtime checkbox.

Deferred: app-store publication, local shell, simultaneous terminal panes,
unlimited/offline history, full-history search/export, customizable shortcuts,
gesture zoom, foreground keepalive service, and continuous-background guarantees.

## Review evidence and execution order

The PRD convergence pass retained requirement IDs and evidence anchors, folded
the accepted additions into their owning sections, corrected the stale
first-input-discard acceptance clause, and reconciled preference changes with
actual resize/selection invalidation. The final PRD was read top to bottom.
There are no unresolved user-owned product questions from this review.
`prd.md`, `design.md`, and `implement.md` are synchronized; `ui-design.md` and
the native v8 manifest record prototype checks and limitations.

The user confirmed on 2026-09-07 that macOS/Linux basic functionality has been
verified, satisfying the existing host prerequisite for Android development.
The migration owner and `mainline-refresh.md` distinguish this report from
unobserved per-route fixture results. Read-only local and `dev` access succeeds.
After task activation, follow `implement.md` through bridge/build, shared client,
pairing/Sessions, terminal/gestures, settings/lifecycle, and APK/device acceptance.
The mainline refresh requires no new product decision or repeated plan approval.
