# Visible frame followed by connection failure

Xiaomi development 1011 receives a Herdr frame after explicit takeover, then shows
Connection failed with last pixels retained. Read-only host listing shows the
same Session Detached at 54x49. Host logs confirm takeover, attach, then
transport_closed approximately 223 ms after takeover, while the primary host
connection closes over two minutes later. This is not solely a stale error label.

Classification: undetermined until the mobile actor's typed terminal error is
observed. Current Logcat omits the application state/error; direct JDWP attach
was attempted twice but the device closed the debugger handshake. Add content-free
DEBUG-build state transition/error-code diagnostics at the existing Repository
owner, and retry the user-requested connection without implicit takeover. Do not
change protocol validation, recovery, timeouts or host Session ownership based
only on a generic Connection failed label. Existing task specs have been loaded.

The user retried on diagnostic development 1012: connection reaches Active, but
host grid remains 80x24 while the phone requires 54x49. Trace: initial retained
snapshot is 54x49, then Active is 80x24. Code identifies a local implementation
defect at the asynchronous attach boundary: connectTerminal captures Repository
viewport before the View finishes measuring. While connect awaits, the size
consumer records the new grid but has no terminal to resize; after attach returns
that measurement is not reapplied and View lastSize suppresses a duplicate report.

Fix scope: keep latest desired viewport synchronously in AppRepository.measure;
the existing conflated size consumer is the sole post-attach resize submitter.
Signal it when attach finishes with a different requested/current desired size.
Do not overwrite newer queued sizes with an older captured value; fence late
resize errors against the retired attachment. Add one deterministic real-host
regression that changes measurements while attach is suspended. The original
immediate post-takeover closure is not yet classified as resolved.

The user opened/closed the keyboard on the diagnostic phone build and confirmed
normal layout. Content-free logs show Active 80x24 -> Synchronizing -> Active
54x27 -> Synchronizing -> Active 54x49; a later second toggle also returns to
54x49. A real Reconnecting event recovered normally before these toggles. The
initial takeover closure has not reproduced, so it is not attributed to the
measurement race without further evidence.

AttachGeometryTest fails on 1011 at final-grid convergence, and passes on signed
1012 (5.221 s). Both expanded Herdr takeover attempts fail with
`deadline_exceeded` before any test Session is created; contemporaneous host
logs show relay path churn. The retry waited for the prior primary connection
to close. Neither run reached takeover, so neither provides acceptance evidence
for the original post-takeover closure. No deadlines, transport policy or
takeover validation were changed.

The final signed 1012 terminal UI regression passes (27.419 s), including keyboard
geometry, scrolling, selection/copy, preferences, Activity retention and Session
management. Debug/release builds and lint pass; both APK signatures and native
ELF/zip 16 KiB checks pass. Evidence is in target/android-toolchain/
attach-geometry-1012.log and attach-terminal-ui-1012.log.

Final development 1012 was installed over the earlier diagnostic 1012 on the
Xiaomi via adb install --no-incremental -r (Success). PackageManager confirms
versionCode 1012, lastUpdateTime 2026-09-07 14:45:01 and unchanged firstInstallTime
2026-09-07 11:51:23. Final APK SHA-256:
79ea6eb81189fe1a46eb8d410a97020b022fa51d15778cff12183f9d2bf7c233.
The final artifact includes the size fix; the diagnostic artifact used for the
user's keyboard workaround did not. No uninstall or data clear was performed.
Phone cold-attach sizing still awaits physical acceptance. The original
post-takeover closure remains unclassified and has not reproduced on retry;
content-free development diagnostics remain intentionally available for it.

The user supplies the repeatable path: attach the local Session on Mac first,
then explicitly take it over on Android. Final development 1012 Logcat at
14:46:20 reports Synchronizing 140x39 -> Closed, code=malformed_frame. Host logs
again confirm takeover then transport_closed, with the retained Session now
54x49. This rejects a generic network outage as the immediate failure cause:
the mobile semantic consumer or its protocol owner rejects an event during the
transition. Classification remains undetermined until the rejecting validation
site is identified. Reproduce using a disposable Session with a real local CLI
controller and phone-sized takeover, retaining the Herdr case as a smoke fixture.
Do not weaken validation or introduce retry/delay policies to mask rejection.

Validation constraint: the user explicitly requests emulator-only Android
verification. Do not install, inspect, instrument or operate the physical phone
again for this investigation. The attempted test-package install before that
instruction was rejected by the device; no physical test ran. The original
reported phone Logcat above remains the diagnostic input.

Temporary emulator-only classification codes distinguish surface projection,
history-cache admission and a whitelist of static client protocol errors. They
contain no payload/detail dump and must be removed before delivery. Production
error semantics and validation remain unchanged during localization.

User manual validation handoff: emulator-5556 had been closed. The remaining
running Pixel_9 is emulator-5554. At the user's request, the final development
1012 artifact (not the temporary diagnostic build) was installed there and its
MainActivity opened. PackageManager confirms versionCode 1012 and first install
at 2026-09-07 15:10:59; this development package has no prior device data there.
Physical phone was not accessed. Pause automated interaction for the user's test.

Before handoff, emulator-5556 passed desktop-first ordinary-shell takeover
(6.454 s), isolated Herdr native takeover (8.371 s), full App UI takeover
(10.081 s), and Herdr with Unicode/long lines through the App UI (10.398 s).
These successes do not resolve the user's reproducible original error. Current
new tests and temporary classification codes are still investigation work;
remove the temporary codes before any newly built APK delivery.

Exact failing Session reproduction on emulator-5554: ordinary local CLI attach
was acquired only after read-only listing confirmed the user's main Detached.
The diagnostic CLI sent no child input; emulator explicitly took over that
diagnostic attachment. At 15:18:45, the temporary whitelist identified
`TerminalSurfaceError::InvalidWidePair` from protocol semantic decoding. The
local diagnostic controller exited on lease loss; the Session was not closed.

Application-neutral regression reproduces the same category: enter alternate
screen at 2x8, put a styled CJK pair at columns 3/4, then resize to 2x3, cutting
away the spacer. `snapshot.validate()` fails with InvalidWidePair. Alacritty
0.26's non-reflow shrink truncates the row without repairing the retained wide
head; `zterm-terminal::projection::project_row` independently copies raw cell
flags and exports that orphan. This is a local implementation defect at the
engine-to-semantic projection boundary, not a missing client recovery policy.

Change boundary: project only complete adjacent wide pairs in the shared row
projection; orphan fragments become styled blanks. Cover clipping, regrowth,
complete pairs, and snapshot/delta/history consistency with model regressions.
All consumers retain strict wire validation. No application-name recognition,
new engine/state owner, transport change or Android retry workaround. The
terminal projection and its tests/spec are necessary affected files. The host
daemon will need the corrected build for real integration acceptance; do not
force-stop the user's daemon/Session to activate it.


### Wide-cell clipping fix and acceptance boundary

The host projection fix and deterministic regression are implemented. The
regression fails on the old projection with `InvalidWidePair` and passes after
repair. All `zterm-terminal` tests and strict Clippy pass. The real Herdr 0.8.2
black-box passes alternate-screen resize (47x123), detached progress and resync;
attachment-resync also passes. The ordinary no-argument terminal black-box run
is an explicit skip, superseded by `sh tests/foundation/terminal-blackbox.sh
--mode herdr` passing.

A temporary `wide_clip_fixture` executable started a task-private production
network daemon to test the corrected host without ending the user's main.
The emulator paired with this host (3.925 s), but two following desktop-first
runs failed with `unauthorized` before creating a disposable Session; neither
is a passing takeover test. macOS displayed its incoming-connections prompt.
The user explicitly rejected it. The fixture was immediately stopped through
its private Unix socket, its executable and private state removed, and the
emulator's pre-test address book restored. Do not bypass the firewall choice,
restart this helper, or attribute `unauthorized` to the firewall without proof.

All temporary Android `diagnostic_*` error mappings were removed. Emulator
5554 was restored to the final signed development 1012 artifact. No physical
phone operation was performed. The corrected desktop binary is built at
`target/debug/zterm`; the running product daemon/main still uses its old build.
Actual emulator takeover against the corrected host remains pending host
update/restart. Restarting the user's daemon ends its current Zterm Sessions,
so that final disruptive activation requires user confirmation.

Evidence: `target/android-toolchain/wide-clip-{baseline,terminal-tests,clippy,
herdr-blackbox,daemon-tests,takeover-tests,host-build,fmt-final}.log` and the
emulator-pair / native-fixed / native-fixed-retry logs. The optional desktop-
first Android fixture now accepts `hostName`; `wideClip=1` checks an exactly
clipped pair at 54/56 columns. Its fixed-host Android assertion path has not
passed, due to the pre-Session admission failures above.

Final local checks: controller_lease 4/4 and local_session_ipc 7/7 pass; cargo fmt --all --check and git diff --check pass. Emulator development 1012 was opened at Home for the next manual run.

### Installed v0.1.25 acceptance, 2026-09-07

After the user updated the host, the installed `~/.local/bin/zterm` reports
0.1.25. Tests used the normal running product daemon and emulator-5554 with the
unchanged final development 1012 APK. No physical device was accessed, no
product source or APK was changed, and the rejected network fixture was not
restarted.

Desktop-first tests against the installed host:

- Exact alternate-screen wide-cell clipping/native takeover: PASS, 7.159 s.
  Mac 140 columns to Android 54 columns, styled orphan becomes a valid blank;
  subsequent input/output succeeds in the disposable test shell.
- Exact clipping through the actual App Take over UI: PASS, 11.831 s.
  Mac 140 columns to Android 56 columns; host viewport matches 56x53, with no
  post-takeover error.
- Installed Herdr 0.8.2/native takeover with Unicode and long output: PASS,
  9.665 s, including subsequent disposable-shell input/output.
- Automated Herdr App UI fixture: two FAILURES (5.310 s and 5.332 s), both
  `unauthorized` before a disposable Session was created. Neither is counted
  as a passing takeover test. All owned test Sessions/Herdr servers were cleaned.

The user's actual new `main`, already running Herdr with Pi on Mac at 140x39,
was then selected through the emulator App and explicitly taken over. At
16:18:19.958 local time, content-free Logcat reports
`state=active code=none rows=53 columns=56`; the installed CLI independently
reports the same Session Attached at 56x53. It remained attached without a new
error through the subsequent checks (over ten minutes). Touch scrolling in the
actual Herdr/Pi pane responds in both directions and the shortcut bar remains
visible. No text or commands were entered into the user's child agent, and its
Session was not closed. The emulator is left connected for manual testing.

This verifies the original post-takeover InvalidWidePair fix on the updated
host, including the real user Session. It does not classify the two separate
pre-Session `unauthorized` failures: `read_controller_welcome` also maps most
incomplete transport/deadline errors to Unauthorized, so the error code alone
does not prove a rejected authorization or a firewall cause. Keep that cold
connection/handshake anomaly open; do not add speculative retries or weaken
authorization/semantic validation.

Standard docked-keyboard show/hide resize was not accepted in this run. The
emulator's Gboard opened a physical-keyboard/handwriting toolbar and floating
keyboard, which do not consume normal bottom IME insets. The existing
`show_ime_with_hard_keyboard` setting was already 1 and was not changed. Android
Back hid the floating keyboard; final `mInputShown=false`, `mImeWindowVis=0`,
and the Session remained 56x53. This is an explicit test limitation, not proof
of a product resize regression or success.

Evidence: `target/android-toolchain/v0.1.25-{wide-native,wide-ui,herdr-native,
herdr-ui,herdr-ui-retry}.log`, matching `-driver.log` files, and
`v0.1.25-main-ui-state.log`. User terminal screenshots were only used locally
for visual checks and are not retained in the repository.
