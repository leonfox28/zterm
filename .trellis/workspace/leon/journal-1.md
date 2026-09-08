# Journal - leon (Part 1)

> AI development session journal
> Started: 2026-08-20

---



## Session 1: Phase Zero relay bootstrap and v0.1.0 release

**Date**: 2026-08-21
**Task**: Phase Zero relay bootstrap and v0.1.0 release
**Branch**: `main`

### Summary

Completed the Rust 1.98.0 workspace and quality gates, published unified product version 0.1.0 with isolated GHCR production/development channels, deployed the official Iroh 1.0.3 relay by immutable production digest under the 1Panel Compose root, verified public authenticated relay fallback with QAD disabled, and exercised rollback plus production restore.

### Git Commits

| Hash | Message |
|------|---------|
| `43b06ff` | (see git log) |
| `a346a45` | (see git log) |
| `b9cac37` | (see git log) |
| `3b10aa8` | (see git log) |

### Status

[OK] **Completed**


## Session 2: Simplify relay release and deployment

**Date**: 2026-08-21
**Task**: Simplify relay release and deployment
**Branch**: `main`

### Summary

Simplified Relay version mapping and deployment, released v0.1.1 to GHCR, migrated the default server to the minimal zterm-relay Compose, verified one authenticated handshake, and removed duplicate tag-triggered CI.

### Git Commits

| Hash | Message |
|------|---------|
| `c2b574d` | (see git log) |
| `92dda0e` | (see git log) |
| `55563c3` | (see git log) |

### Status

[OK] **Completed**


## Session 3: Complete Phase One Foundation Gate

**Date**: 2026-08-22
**Task**: Complete Phase One Foundation Gate
**Branch**: `main`

### Summary

Completed and verified the Iroh profile, terminal model, PTY lifecycle, retained terminal driver, black-box compatibility, resource budgets, five-platform CI matrix, and fixed the PTY wait-lock starvation exposed by hosted CI.

### Git Commits

| Hash | Message |
|------|---------|
| `0026c4b` | (see git log) |
| `1b1fb8c` | (see git log) |

### Status

[OK] **Completed**


## Session 4: Complete M2-M3 core and local daemon

**Date**: 2026-08-22
**Task**: Complete M2-M3 core and local daemon
**Branch**: `main`

### Summary

Implemented typed core and protobuf contracts, secure per-user state and identity, same-UID Unix IPC, detached single-instance daemon lifecycle, and thin CLI diagnostics. Added real Linux cross-UID rejection coverage, native Windows shared-boundary CI, independent review fixes, hosted evidence, and archived the completed child task.

### Git Commits

| Hash | Message |
|------|---------|
| `2eb16dc` | (see git log) |
| `0850435` | (see git log) |
| `38f9e78` | (see git log) |
| `e5050a4` | (see git log) |

### Status

[OK] **Completed**


## Session 5: Complete M4 persistent local sessions

**Date**: 2026-08-22
**Task**: Complete M4 persistent local sessions
**Branch**: `main`

### Summary

Implemented and independently verified daemon-lifetime named terminal sessions, authoritative PTY/VT ownership, same-UID local attachments, bounded snapshot/resync, controller takeover, exact mutation replay, resource governance, unwind-safe cleanup, and fatal-listener recovery. All local gates and GitHub Actions run 32570831589 passed on macOS arm64/Intel, Linux x86_64/arm64, and Windows shared boundaries; the M4 child task was archived without advancing M5-M8.

### Git Commits

| Hash | Message |
|------|---------|
| `70ae314` | (see git log) |
| `c4746c7` | (see git log) |
| `6a562ad` | (see git log) |
| `ffe169f` | (see git log) |
| `6ed5753` | (see git log) |

### Status

[OK] **Completed**


## Session 6: Complete M5-M6 transport and device authorization

**Date**: 2026-08-23
**Task**: Complete M5-M6 transport and device authorization
**Branch**: `main`

### Summary

Implemented and independently verified daemon-owned Iroh transport, connection brokering, one-time directional pairing, device authorization/revocation, same-UID device IPC, lifecycle/resource/concurrency hardening, and Linux real-Iroh/cross-UID gates. Restored the accepted official n0 evidence boundary after removing an unnecessary self-hosted Relay workflow; all seven jobs passed on final head 4ec0cba in run 32615123176. Marked parent M5-M6 complete without advancing M7-M8 and archived the child task.

### Git Commits

| Hash | Message |
|------|---------|
| `62d7393` | (see git log) |
| `7ebcb09` | (see git log) |
| `47cece1` | (see git log) |
| `193e008` | (see git log) |
| `80f8852` | (see git log) |
| `4b85260` | (see git log) |
| `b1a08a6` | (see git log) |
| `5e021cd` | (see git log) |
| `bf3d313` | (see git log) |
| `1d90b55` | (see git log) |
| `4ec0cba` | (see git log) |
| `3516b30` | (see git log) |

### Status

[OK] **Completed**


## Session 7: Close remote CLI and plan Phase 1 release acceptance

**Date**: 2026-08-24
**Task**: Close remote CLI and plan Phase 1 release acceptance
**Branch**: `main`

### Summary

Recorded hosted CI evidence without overclaiming remote Session behavior, archived the M7-M8 implementation child, and created approved M9 signed distribution plus M10 installed-binary acceptance plans.

### Git Commits

| Hash | Message |
|------|---------|
| `3ae4c3a` | (see git log) |

### Status

[OK] **Completed**


## Session 8: Remote terminal UX v0.1.9
<!-- trellis-session: v=2 fp=17753c03a16dc25d -->

**Date**: 2026-09-01
**Task**: Remote terminal UX v0.1.9
**Branch**: `main`

### Summary

Delivered bounded remote scrollback and a theme-aware device/path/RTT status row; fixed oversized and legal-size rapid-resize lifecycle races; published signed v0.1.9 after full cross-platform CI and install verification; user accepted the final build in macOS Ghostty.

### Git Commits

| Hash | Message |
|------|---------|
| `fff36b9` | feat: improve remote terminal UX |
| `3080d1f` | fix: pass cross-platform release gates |
| `f2b9b11` | chore: prepare v0.1.8 release |
| `8976c31` | fix: preserve terminal outcomes during resize races |
| `a104a1c` | fix: restore shared reset preflight type |
| `158364e` | chore: prepare v0.1.9 release |
| `fceb075` | fix: serialize terminal resize synchronization |

### Status

[OK] **Completed**


## Session 9: Update Trellis to 0.6.16
<!-- trellis-session: v=2 fp=4d96c395b9773db6 -->

**Date**: 2026-09-01
**Task**: Update Trellis to 0.6.16
**Branch**: `main`

### Summary

Updated the project Trellis runtime and platform integrations to 0.6.16, validated the generated templates and runtime syntax, and pushed the work commit.

### Main Changes

- Updated Trellis runtime, workflow, hooks, and platform adapters to 0.6.16.

### Git Commits

| Hash | Message |
|------|---------|
| `b91b942` | chore: update trellis to 0.6.16 |

### Testing

- [OK] trellis update --dry-run reported Already up to date; Python, JavaScript, TypeScript parsing, task JSON, and template hashes passed.

### Status

[OK] **Completed**


## Session 10: Harden release preparation
<!-- trellis-session: v=2 fp=5dd87451af3d1605 -->

**Date**: 2026-09-04
**Task**: Harden release preparation
**Branch**: `fix/release-prepare-reliability`

### Summary

Made release preparation deterministic with Cargo-owned workspace lock refresh, focused validation, exact clean-commit branch/PR resume, real Cargo fixture coverage, clearer diagnostics, and synchronized release docs/specs. Focused gates, independent Trellis review, and full just check passed.

### Git Commits

| Hash | Message |
|------|---------|
| `367a7f9` | fix: make release preparation deterministic |

### Status

[OK] **Completed**


## Session 11: Unify local and remote terminal attachments
<!-- trellis-session: v=2 fp=da2d32926ddf81b9 -->

**Date**: 2026-09-05
**Task**: Unify local and remote terminal attachments
**Branch**: `fix-zterm-herdr-not-synchronized-attachment-snapshot`

### Summary

Unified local and remote daemon attachment semantics behind a frontend-owned Session client, added an opaque viewer-daemon tunnel over shared Iroh connections, made terminal chrome universal with exact local status, fixed event-entry snapshot acknowledgement for alternate-screen resize, and verified Herdr 0.8.2 plus full workspace gates.

### Git Commits

| Hash | Message |
|------|---------|
| `4328e2e` | fix: unify local and remote terminal attachments |
| `c343146` | docs: record unified terminal attachment contract |

### Status

[OK] **Completed**


## Session 12: Simplify terminal architecture and bound client control
<!-- trellis-session: v=2 fp=33aef6839ee31169 -->

**Date**: 2026-09-05
**Task**: Simplify terminal architecture and bound client control
**Branch**: `fix-zterm-herdr-not-synchronized-attachment-snapshot`

### Summary

Completed the single-agent architecture review and implementation: separated SessionClient transport/session/view/ipc ownership, bounded control deadlines and deferred buffers, removed repeated projections and surface clones, and centralized UI transitions. Updated four executable specs. just check passed (507 tests passed, 6 ignored); Herdr black-box passed. Windows cross-check was limited by missing host SDK. User approved the work commit, task archival and subsequent standard PR/release flow.

### Git Commits

| Hash | Message |
|------|---------|
| `0c21738` | refactor: simplify terminal architecture and bound client control |

### Status

[OK] **Completed**


## Session 13: Fix shared terminal synchronization and return-to-live presentation
<!-- trellis-session: v=2 fp=a044cd65746d58d3 -->

**Date**: 2026-09-05
**Task**: Fix shared terminal synchronization and return-to-live presentation
**Branch**: `fix/terminal-sync-scroll`

### Summary

Preserved explicit resume delta ACK authority across the shared client/view/UI boundary and made return-to-live snapshot presentation atomic. Native quality gate and real local/remote Herdr validation passed. User approved commit and v0.1.17 publication; proceeding through normal protected PR/CI/release flow.

### Main Changes

- Ordinary deltas never acquire snapshot ACK semantics from UI synchronization state.
- Resume snapshots commit live cells, layout and readiness after successful flush; Active compares actual desired frames.

### Git Commits

| Hash | Message |
|------|---------|
| `1573745` | fix: preserve terminal sync barriers and live presentation |

### Testing

- [OK] Final just check and Herdr blackbox PASS; local persistent Herdr 3/3 and actual paired dev primed-server smoke passed.
- [OK] Restoring either old defect makes its maintained regression fail.

### Status

[OK] **Completed**

### Next Steps

- Merge the fix PR, prepare v0.1.17, require exact green main CI, approve normal protected signing and verify immutable native plus relay publication.


## Session 14: Simplify CLI workflows and daemon diagnostics
<!-- trellis-session: v=2 fp=eb6d83a60da07470 -->

**Date**: 2026-09-05
**Task**: Simplify CLI workflows and daemon diagnostics
**Branch**: `zterm-cli-commands-execution`

### Summary

Implemented human-readable CLI commands, English y/yes and -y confirmation, atomic Session shutdown admission, update startup and existing daemon event logging. User approved commit/archive and v0.1.19 release flow.

### Main Changes

- Removed public JSON/force flags; simplified setup, pairing aliases, Session lists and one-shot logs -n without follow.
- Added existing-owner operational events and corrected controller-detach and requested-close reason semantics.

### Git Commits

| Hash | Message |
|------|---------|
| `b4941e0` | feat: simplify CLI workflows and daemon diagnostics |

### Testing

- [OK] Independent Trellis check and full just check passed; real PTY confirmation, CLI/daemon/IPC and isolated log privacy tests passed.

### Status

[OK] **Completed**

### Next Steps

- Prepare and publish v0.1.19 through the repository PR/CI/signing workflow.


## Session 15: Unify Ctrl+] command mode and prepare release
<!-- trellis-session: v=2 fp=bbb1a4bafae0fdb7 -->

**Date**: 2026-09-06
**Task**: Unify Ctrl+] command mode and prepare release
**Branch**: `investigate-zterm-herdr-ctrl-close-exit`

### Summary

Implemented a shared fixed Ctrl+] command dispatcher for legacy and enhanced input; local cancellation, release ownership, quoting and --escape removal. Preserved clipboard and keyboard-reporting policy; retained Ghostty/Doubao diagnosis without IME adaptation. Direct implementation and review, no sub-agents. Final just check and generic outer-PTY detach/reattach tests passed. User authorized v0.1.20 release through the repository operator.

### Git Commits

| Hash | Message |
|------|---------|
| `a686f6f` | fix(cli): unify Ctrl+] command handling |

### Status

[OK] **Completed**


## Session 16: Fix retained terminal reattach input and geometry
<!-- trellis-session: v=2 fp=04e0b25de70151b9 -->

**Date**: 2026-09-06
**Task**: Fix retained terminal reattach input and geometry
**Branch**: `investigate-zterm-reattach-input`

### Summary

Initialized CLI resize deduplication from the authoritative snapshot and reconciled physical sizes after initial acknowledgement. Added real Main/Alternate reattach and inactive SIGWINCH/input regressions. Final just check and eight private local probes passed; disposable paired dev Herdr reattach accepted input and cleanup was verified. Direct implementation/review without sub-agents. User authorized formal v0.1.21 release.

### Git Commits

| Hash | Message |
|------|---------|
| `dd7f385` | fix(cli): reconcile reattach geometry from host snapshot |

### Status

[OK] **Completed**


## Session 17: Terminal color and appearance compatibility
<!-- trellis-session: v=2 fp=8e165d9423ebf81f -->

**Date**: 2026-09-06
**Task**: Terminal color and appearance compatibility
**Branch**: `mundane-moth`

### Summary

Implemented the complete color protocol scope inline without subagents; native just check passed (560 tests, 0 failures). User authorized v0.1.22 release workflow.

### Main Changes

- Added host observations, required color metadata, OSC color protocols, appearance notifications, underline compatibility, history repaint and software cursor.
- Fixed controller takeover ACK and stale-notification races; matching new CLI and daemon builds are required.

### Git Commits

| Hash | Message |
|------|---------|
| `139d097` | feat(terminal): support color and appearance protocols |

### Testing

- [OK] Native just check passed; isolated PTY color swatch fixture passed. Physical GUI/Herdr smoke and Linux hosted evidence remain separate.

### Status

[OK] **Completed**

### Next Steps

- Prepare v0.1.22 in this feature PR, wait for exact PR/main CI, then publish through the protected signed-release operator.


## Session 18: Preserve hidden cursor position for Chinese IME
<!-- trellis-session: v=2 fp=6d1f0896f0a9872d -->

**Date**: 2026-09-06
**Task**: Preserve hidden cursor position for Chinese IME
**Branch**: `wicked-spider`

### Summary

Fixed the CLI compositor resetting hidden cursor coordinates to the origin. Native Herdr/Pi IME smoke passed by user confirmation. Preparing v0.1.23 through the formal release workflow.

### Main Changes

- Preserve active live cursor coordinates independently of visibility; document the contract.

### Git Commits

| Hash | Message |
|------|---------|
| `8416f4d` | fix(cli): preserve hidden cursor position for IME |

### Testing

- [OK] Regression failed before correction and passed after; CLI tests, Clippy, ci-policy, candidate build passed.
- [OK] User confirmed Chinese IME placement is correct in Herdr/Pi with the candidate.

### Status

[OK] **Completed**

### Next Steps

- Publish v0.1.23 with the existing protected release operator.


## Session 19: Remote self-update fix and v0.1.24 release
<!-- trellis-session: v=2 fp=a73b33dbfe0df924 -->

**Date**: 2026-09-06
**Task**: Remote self-update fix and v0.1.24 release
**Branch**: `short-toad`

### Summary

Detached updater survives originating PTY termination. PR #30 merged and v0.1.24 published immutable with eight assets; real remote signed-upgrade acceptance remains pending.

### Main Changes

- Added detached update ownership, bounded approval/control protocol, durable outcome logging and migration guidance.

### Git Commits

| Hash | Message |
|------|---------|
| `ca7eb7e` | fix(update): complete upgrades after the originating session ends |
| `4048885` | chore: prepare v0.1.24 release |

### Testing

- [OK] Local just check passed; all 14 update tests passed on macOS arm64, Linux arm64 and Linux x64 in PR CI 34035113162.
- [OK] Main CI 34035436342 and signed release workflow 34035804008 passed, including all three installer proofs and immutable publication.

### Status

[OK] **Completed**

### Next Steps

- Bootstrap v0.1.24 via SSH, then validate a subsequent signed upgrade from a real remote zterm Session; keep remote-self-update task active.


## Session 20: Release v0.1.26 with Android
<!-- trellis-session: v=2 fp=57c4721a9d71d70b -->

**Date**: 2026-09-07
**Task**: Release v0.1.26 with Android
**Branch**: `holy-zebra`

### Summary

Published immutable v0.1.26 through PR #33 and the canonical release operator. Added Android ARM64 APK to exact-main candidates and protected signing while retaining macOS ARM64 and Linux ARM64/x64. All hosted checks, installers, complete public inventory verification, APK certificate/16 KB inspection, and public-APK emulator upgrade passed. Broader phone-specific acceptance remains pending; no phone or installed host-daemon operation.

### Git Commits

| Hash | Message |
|------|---------|
| `f793542` | feat(android): add terminal app and signed APK releases |
| `9ec604d` | chore: prepare v0.1.26 release |
| `85dff0f` | docs(android): clarify hosted and emulator validation evidence |
| `77fadca` | Merge pull request #33 from leonfox28/feat/android-release |
| `891a219` | chore(android): record v0.1.26 publication evidence |

### Status

[OK] **Completed**


## Session 21: Publish v0.1.27 terminal presentation continuity
<!-- trellis-session: v=2 fp=64ece5bbe3976780 -->

**Date**: 2026-09-08
**Task**: Publish v0.1.27 terminal presentation continuity
**Branch**: `feat/terminal-presentation-continuity`

### Summary

Published immutable v0.1.27 through PR #35 and the canonical exact-main candidate/signing workflow; visual continuity follow-up remains open.

### Main Changes

- Shipped coherent DEC 2026 publication, healthy resize input/IME lifetime, Android integral geometry and desktop final-cell coverage.
- Published all 11 assets; release notes disclose the remaining keyboard-close blank/history refill and incomplete desktop GUI/Herdr visual acceptance.

### Git Commits

| Hash | Message |
|------|---------|
| `ed7c170` | feat(terminal): publish synchronized output at coherent boundaries |
| `a0065d9` | feat(client): preserve presentation and input across resize |
| `807fd41` | chore: prepare v0.1.27 release |
| `cb19c75` | Merge pull request #35 from leonfox28/feat/terminal-presentation-continuity |

### Testing

- [OK] Local full gate and focused Android/emulator checks passed before release; PR CI 34173146157 and exact-main CI 34173688282 passed.
- [OK] Signed release 34174373737 passed APK signing, three native HTTPS installer proofs, roundtrip verification, attestation and immutable publication; metadata source/version/certificate/hash checks passed.

### Status

[OK] **Completed**

### Next Steps

- Keep terminal-presentation-continuity active to fix Android keyboard-close blank/history refill and complete controlled whole-transition/desktop visual acceptance.
- No real Mac main Session was touched and no local daemon was restarted or updated.
