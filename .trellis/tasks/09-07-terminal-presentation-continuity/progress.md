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
only emulator-5556 was shut down. The user subsequently authorized commits and
publication while retaining the visual acceptance gap below.

User follow-up evidence review: endpoint pixel equality does not prove absence of
transient flicker. The new keyboard-close recording still shows an empty top area
before history refill. Do not report flicker as resolved or archive the task; retain
this issue and whole-transition acceptance as open. See acceptance follow-up section.

Release steering: the user authorized “先走发布流程吧” after the above disclosure.
v0.1.27 is published through PR #35 at merge source `cb19c754641d6c0a715ace9d02354b88c57abb2c`.
PR CI, exact-main CI/candidate and formal signed release all passed. The stable,
immutable release has all 11 expected assets and notes the known visual issue.
See `research/release-0.1.27.md` for source/run/tag/metadata evidence. Keep the
visual follow-up open; no release was activated on the running Mac daemon.

Post-release upward-scroll report: reproduced a local View handoff defect (37 px
cells, unchanged row y=75 -> 111 -> 73 across a two-pixel upward crossing).
Corrected fractional shift against actual frame offset with the existing one-row
overscan bound. Three Canvas/touch regressions fail before and pass after; full
`just check`, Android build/lint and two isolated real-host UI tests pass. See
`research/android-scroll-handoff.md`. Disposable runtime cleanup is complete.
This follow-up is ready for the concrete Phase 3.4 commit confirmation; no new
formal release or phone acceptance is claimed. Preserve the older visual gaps.

User confirmed the phone's ghosting is gone, then requested smoother scrolling
and authorized emulator testing. Added an opt-in real-input performance fixture.
Dev/non-debuggable comparisons both pass: unchanged terminal redraw costs
17.03/8.15 ms median Window draw time on this 60 Hz emulator; all measured gesture
targets hit native cached history, but system deadline misses remain. Same-row
requests still produce complete native deliveries. Exact delayed-page edge waits
remain covered by the three Canvas regressions. Preliminary asynchronous position
samples were rejected and removed; do not claim natural stall percentages from
them. See the final accepted measurements in `research/android-scroll-handoff.md`.
The adjacent-row window/render-reuse architecture is still a proposal. No new
production behavior or release belongs to this profiling step.


## 2026-09-08 — approved Android scrolling optimization

Implemented native lazy row windows/content IDs and coordinate rebinding,
Repository off-Main row reuse, direct local pixel scrolling and bounded hardware
row display lists. A warmup failure exposed missing complete-viewport overlap at
the physical cache edges; fixed the multi-page query owner within its original
margin budget and preserved default desktop behavior. Specs and regression tests
cover the cross-layer contract.

Full `just check`, final Android build/lint/JVM and six production pixel tests
pass. Native host integration, real IME/Chinese composition, cross-screen Copy,
Activity retention, raw TUI input and isolated Herdr acceptance pass. Herdr's
initial nested-instance rejection was a test environment issue; clearing inherited
HERDR variables in its owned launch produced a passing focused rerun.

Adjacent archived-old/new non-debuggable APK comparison on the same emulator:
gesture drawing p50 15.01 -> 1.79 ms, p95 20.11 -> 5.30 ms. Fling deadline misses
improve, but manual drag/reversal timing is not uniformly improved. No phone FPS
or total-stall-elimination claim. The ordinary debug APK is restored on the owned
AVD: version 0.1.27 / code 102799, SHA-256
`8c61cdf09c9e328495929d7737ad8480710c2f945296ee4a9e0f8691941fbd1b`.
Physical phone was absent; no phone update/new release or user daemon operation.
Detailed logs, failed-attempt explanations and limits:
[scrolling acceptance](research/android-scroll-optimization.md).

Implementation is reviewable; commit remains in the existing Phase 3.4 workflow.
Keep the overall presentation task active for the previously open visual work.


## 2026-09-08 — phone development build updated

User explicitly requested the phone update. Installed the tested scrolling APK
in place on device `1680fce0`; adb returned Success. Installed-package SHA-256
matches `8c61cdf09c9e328495929d7737ad8480710c2f945296ee4a9e0f8691941fbd1b`;
Dev 0.1.27 / 102799, phone lastUpdateTime 13:53:11. No uninstall/data clearing,
Session interaction, new release or daemon activation. Phone smoothness awaits
user acceptance; the existing commit workflow remains pending.


## 2026-09-08 — phone acceptance and v0.1.28 authorization

The user tested the installed 8c61cdf0 development APK and reported
“测试了一下非常棒”, then explicitly requested committing all changes and the
release workflow. This accepts phone scrolling experience; it does not establish
a phone FPS measurement or close the separate keyboard-close/history-refill and
desktop visual gaps. Publish the scoped scrolling improvements as v0.1.28 using
normal PR/CI/main-candidate/tag/protected-signing/publication checks. No running
Mac daemon update or interaction with its main Session is authorized.


## 2026-09-08 — v0.1.28 published

Phone scrolling accepted; all implementation changes committed in `022ffb7`.
PR #36, exact-main CI and signed publication passed. Immutable v0.1.28 has all
11 expected assets; Android code 102899. See `research/release-0.1.28.md`.
Keep prior keyboard-close/history-refill and desktop visual follow-ups open.
No real daemon/main Session operation or additional phone installation.
