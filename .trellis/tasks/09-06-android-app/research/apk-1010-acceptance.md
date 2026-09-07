# APK 1010: IME, local bottom scroll and Herdr touch

User follow-up: keep keyboard animation local, resize once after each settled
transition, remove the unnecessary synchronization when scrolling cached output
to the bottom, and make Herdr respond to taps and vertical touch scrolling.
Implementation and verification ran inline without sub-agents.

## Delivered behavior

- An Activity-level IME animation lifecycle fences grid measurement. TerminalView
  pans/clips the current grid locally and commits final geometry at one pre-draw
  after animation completion, including interrupted opening. The shortcut bar
  remains above the keyboard.
- Scrolling to offset zero immediately projects the continuously updated live
  surface. It does not request a fresh synchronization; late history replies do
  not replace the live view. A selected view stays pinned. Missing older cache
  ranges still load as needed under the existing bounded-cache policy.
- On the active live grid, completed taps and vertical gestures reach programs
  that request mouse input. Alternate-scroll emits cursor keys only in the
  declared alternate-screen mode. Long press and selection remain local.
  Pointer admission checks the exact rendered source, mode, epoch and geometry.
- Disable the otherwise unused local double-tap recognizer: a 100 ms tap pair
  previously lost its second click. The raw fixture reproduced this on 1009.
  No host/wire change or Herdr-specific process detection was introduced.

## Final evidence

All logs below are in `target/android-toolchain/`.

- `release-1010-ui.log`: PASS, 3 tests, 63.074 s, signed release on API 36 arm64
  emulator-5556 with full Gboard and concurrent 55-second screen recording.
  The existing full UI test now starts with 400 lines and observes exactly
  three epochs (initial + two resize admissions) across show/hide. Interrupted
  opening converges to final host/View geometry. Cached bottom remains Active
  with unchanged epoch. Selection/copy across three screens, canceled copy,
  Activity retention, rotation/font changes and Session management pass.
- The same run launches an isolated Herdr 0.8.2 against the Mac host, scrolls its
  pane history both directions and taps its responsive chooser to switch Alpha
  and Beta, confirming each pane's distinct output. The outer history offset
  remains zero. Test sessions are closed in cleanup; user sessions are untouched.
- Raw child fixture in that run verifies one press/release per completed tap,
  rapid taps, cell coordinates, canceled gesture suppression, both wheel
  directions, alternate-scroll, local long press and stale-source rejection.
- `release-1010-native-identity.log`: PASS, 9 tests, 3.724 s. Real Mac terminal
  input/resize/selection/rename/detach and bridge/identity tests pass. Updating
  retains the release controller identity and its one known host.
- `dev-1010-identity.log`: PASS, 8 tests, 0.635 s. Development retains its separate
  controller identity and zero hosts. Both packages coexist at versionCode 1010
  with distinct UIDs. No app data was erased.
- `touch-native-final.log`: 9 Android Rust tests pass, including local bottom and
  late-history reply regression. `touch-encoding.log`: shared mouse encoder
  passes. `touch-desktop.log`: 69 terminal UI tests pass, 3 preexisting ignored.
- Strict client/Android/CLI Clippy, debug/release lint, dependency policy and
  Trellis context validation pass. `git diff --check` is clean. Context-size
  warnings for the existing large PRD/design are informational.
- Both signed APKs pass certificate verification and every native ELF/APK zip
  16 KiB alignment check. Manifest IDs remain release `io.github.leonfox28.zterm`
  and development `io.github.leonfox28.zterm.dev`.

Visual evidence: `ime-1010.mp4`, `ime-1010-contact.png`,
`herdr-1010-touch.png`; machine-readable install evidence and hashes accompany
APKs in `target/android-apk/1010/`. Recording and Herdr screenshot were inspected.

## Artifacts

- `zterm-android-arm64-1010.apk`
  SHA-256: d721e5b4afb1e9f897f3cac6dc68ddd8810c2d1fb71739438865f988a443ab58
- `zterm-dev-arm64-1010.apk`
  SHA-256: c75d02b91cf7dacfdef8e8b60e326c08ca9882f2bf42ff62841f53348601720f

1010 supersedes the interim 1007/1008/1009 builds. The Xiaomi phone was unplugged
for this follow-up: its installed 1006 was not updated, and these fixes still
need physical Xiaomi IME/touch acceptance. Existing broader untested cases stay
explicitly pending; the Android task remains in_progress, with no commit/archive.
