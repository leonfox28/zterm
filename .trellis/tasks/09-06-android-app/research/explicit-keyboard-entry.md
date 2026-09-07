# Explicit keyboard entry

The user reports that tapping anywhere in the terminal opens the keyboard.
`TerminalView.onSingleTapUp` sends a completed mouse click when declared, then
unconditionally calls `showSoftInput` for every input-ready frame. The previous
UI test even expected a child click to resize the terminal through IME opening.

Classification: local Android interaction-policy defect. The native gesture
owner and declared child pointer modes already distinguish clicks, scrolling
and selection. No host/protocol or terminal architecture change is needed.

User-confirmed correction on 2026-09-07 (rightmost bottom show/hide button): content taps keep their local/child
meaning and never request IME visibility. Keep the eight terminal shortcuts and
add a separate keyboard icon at the end of the same bottom row. It explicitly
shows/hides the native terminal InputConnection; system Back can still hide it.
No app-bar button, application-name recognition or guessed text-field/cursor hit
region. Keyboard visibility is observed from Android IME insets, including a
floating IME with zero bottom inset; do not introduce a second visibility flag.

Change boundary: TerminalView removes the tap side effect and exposes explicit
IME control; TerminalScreen connects the bottom control and native View, AppUi
draws its icon, localized resources supply accessibility labels. Update existing
UI regression cases and the matching task/spec/operator wording. No Rust or
network modifications, no physical-phone operation, no user Session closure.

Acceptance: plain shell and mouse/alternate-screen taps leave IME hidden and
geometry unchanged; mouse press/release coordinates and rapid taps still work.
Explicit keyboard show/hide retains the same Session and uses settled resize.
Scrolling, long-press selection and Herdr tab changes do not open the IME.

## Added font-slider request

The user additionally requested a horizontal 8–16 font slider with step 1.
Settings owns a local integer preview and commits through the existing atomic
preference update on gesture completion. AppStore must accept every integer in
that range on reload; retaining the old 12/14/16 whitelist would silently lose
odd/small sizes. The same range drives the slider and loader. Existing renderer
measurement handles font changes; no native/wire change is necessary. UI tests
cover both end gestures, every stored value, Activity recreation and real host
grid changes at smaller and larger sizes. Default 12 and valid prior presets
remain compatible.

## Verification and delivery, 2026-09-07

No Rust code or generated bindings changed for this work. Debug/test assembly,
Debug and Release lint, and the JVM test task complete successfully (the JVM
source set is empty). Release assembly also passes via `tools/android/apk.py`.

The existing Zterm_Acceptance AVD was started headlessly as emulator-5556, with
its existing hw.keyboard=no configuration, to test a full bottom software IME.
It was paired through the installed normal zterm 0.1.25 daemon; the rejected
wide_clip_fixture was never restarted. An initial stale test-package signing
mismatch was corrected by reinstalling only the test APK with the matching
standard debug signature. The app identity/store was not cleared.

- Old signed development 1012 fails the new regression at the exact assertion
  `child tap does not open IME` (8.709 s). Baseline evidence:
  `target/android-toolchain/explicit-keyboard-baseline.log`.
- New development 1013 passes all three TerminalUiTest methods in the combined
  run: ordinary content tap/no IME, explicit button show/hide, system Back,
  canceled IME animation, selection/Copy, history/cache, Activity retention,
  rotation and font geometry (8, 9, 13, 16, 12); raw child mouse coordinates,
  rapid taps, wheel, alternate-scroll, long-press ownership and stale-source
  rejection; actual isolated Herdr tab switching and pane scrolling without IME.
  Full keyboard show/hide observes exactly three input epochs including the
  initial epoch: one settled resize for each transition (53 -> 30 -> 53 rows).
- The combined run is recorded as 3/4 passing in `keyboard-font-ui.log`
  (55.905 s); its first Settings test failed because the injected drag began
  outside the active track. A second coordinate attempt also failed at the
  opposite edge. Starting each drag inside the track fixes test input without
  changing product code. `font-slider-ui-final.log` passes SettingsUiTest in
  3.052 s: touch both endpoints, all nine integer saves/reloads, and recreation
  at size 11. Do not relabel the earlier aggregate run as fully passing.
- Visual checks of native Settings and the actual Herdr fixture confirm the
  slider/current value/preview and rightmost keyboard button. Screenshots are
  ignored local evidence under `target/android-toolchain/keyboard-font-*.png`.

Signed release and standard debug APKs are under `target/android-apk/1013`.
Both pass signature verification plus native ELF/zip 16 KiB alignment checks.
Release SHA-256:
`fa0a21d6f2b543749c24963628e233226119b8de8f4eedc705c332bf1f381137`.
Development SHA-256:
`dc898fc91d1c4f01210a143feeca9fed31903af7d98aff92b907aa72549f40d7`.
The development artifact was installed over 1012 on the user's running
emulator-5554 at 16:58:15; PackageManager confirms versionCode 1013 and unchanged
firstInstallTime 15:10:59. Before/after identity and saved-state file hashes are
identical (booleans recorded in `dev-install.json`, no identity material logged).
The current Android versionName remains 0.1.24 from this WIP Cargo workspace;
1013 is the APK versionCode, not a new native published release.

Only test-owned Sessions were closed; the user's main remains available. No
physical phone operations, product data clears, firewall changes, commits or
publication. Broader Android acceptance and the separate pre-Session handshake
anomaly remain open. The temporary headless AVD is stopped after verification;
the user's existing Pixel_9 stays running with development 1013 for manual tests.

## Keyboard style follow-up: development 1014

User noted that the keyboard control did not match adjacent shortcuts. Its
standalone IconButton used the default neutral foreground and a 24 dp glyph,
while the surrounding TextButtons use the theme primary color and smaller
labels. Local presentation defect: TerminalScreen now uses the same TextButton,
zero content padding and existing equal-width/full-height slot, with an 18 dp
keyboard glyph inheriting the button foreground. No gesture/IME/network logic
changes and no new test suite is needed for this cosmetic correction.

Debug build, Debug/Release lint and signed Release build pass. Development and
Release signature/16 KiB APK checks pass. Development 1014 installed over 1013
on emulator-5554 with identical saved-state/identity file hashes. APKs, hashes
and installation evidence are in `target/android-apk/1014`; build logs use
`target/android-toolchain/keyboard-style-*`. Emulator visual/interaction checks
are recorded below after completion. No physical phone operation.

Visual acceptance completed on emulator-5554: before/after crops contain only
shortcut chrome (`keyboard-style-before.png`, `keyboard-style-after.png`) and
show the 18 dp keyboard glyph using the same primary foreground as Esc/Tab/etc.
No user terminal contents are retained in these images. Tapping the new
TextButton shows the current floating IME and changes the accessible action to
Hide keyboard; tapping again restores Show keyboard and `mInputShown=false`.
The same main remains Attached at 67x63. This validates control wiring and
floating visibility; the prior full docked-IME regression remains the 1013
result and was not repeated for the cosmetic-only change. User preferences,
including the selected font size, are retained. No child text or commands sent.
