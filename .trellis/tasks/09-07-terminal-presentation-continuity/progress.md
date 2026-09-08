# Implementation progress — 2026-09-08

Code is implemented on `feat/terminal-presentation-continuity`; visual continuity
acceptance remains incomplete. Inline only.
No user daemon restart, no access to the real macOS `main` Session. All device tests
use the independent `/tmp/zterm-presentation-fixture-0908` host and new `emulator-5556`.

Host: coherent DEC 2026 publication/deadline, exact-boundary bounded history tickets,
input/ACK progress. Clients: healthy input lifetime plus coordinate generation;
Android exact grid/remainder below toolbar, drawn geometry, direct IME events;
desktop resolved-cell diff and full unknown-baseline coverage without preclear.

Full `just check` passed. Android build/lint passed. Focused desktop suite 72 passed
(3 isolated helpers), Android native Rust 13, host model/driver/Session/wire tests
passed. Real emulator geometry/Chinese/high-low-TUI pixel tests passed with gesture
and three-button navigation, 420/360 dpi, font 14/9; gesture/history/copy/rotation/
font/mouse/reversed-animation and attach-race tests passed. Final test adjustment passed the targeted test, CLI Clippy and formatting checks.

See `research/runtime-acceptance.md` for exact timings/artifacts and limits. Desktop
GUI visual capture was denied by the tool; no visual pass or quantitative jank claim.

The disposable daemon was gracefully stopped after its last owned Session was closed;
only emulator-5556 was shut down. Next: review the concrete Phase 3.4 commit plan. Nothing is committed or published. Archive/journal must wait
until work commits are approved and desktop acceptance gap is handled accurately.

User follow-up evidence review: endpoint pixel equality does not prove absence of
transient flicker. The new keyboard-close recording still shows an empty top area
before history refill. Do not report flicker as resolved or archive the task; retain
this issue and whole-transition acceptance as open. See acceptance follow-up section.

Release steering: the user authorized “先走发布流程吧” after the above disclosure.
Proceeding with v0.1.27; local doctor passes, origin/main still equals baseline,
previous full gate plus final owner checks are green. Keep the visual follow-up
open, and do not activate the release on the running Mac daemon.
