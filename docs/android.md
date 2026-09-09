# Android

The Android client uses the same semantic terminal protocol, pairing and Session
client as the desktop CLI. Kotlin/Compose owns navigation, CameraX, the system
photo picker, IME and a native Canvas terminal. Rust owns synchronized input,
terminal rows, bounded history pages and captured text selection. No shell,
PTY, daemon or ANSI parser runs on the phone.

Desktop CLI connections still use Unix IPC to the local daemon, which tunnels
remote Session traffic over Iroh. Android connects over Iroh directly. Both use
the shared Session protocol driver and outbound Hello/Welcome implementation;
desktop process recovery stays in its Unix adapter. Host authorization, inbound
services and connection arbitration remain on the daemon.

The first APK targets Android 8.0+ (API 26), arm64 devices, with target SDK 36.
Download `zterm-android-arm64.apk` from the [GitHub Release](https://github.com/leonfox28/zterm/releases/latest).
Install it directly, without a store listing. Updates retain the existing release
package and certificate. See [release operations](./releasing.md#android-package-and-signing)
for version codes and signing.

## Connect

On the host, create a one-time ticket using the existing `zterm pair create`.
`--qr` displays a camera-readable code in a sufficiently wide interactive
terminal; `--qr-image new.png` writes a private PNG without overwriting a file.
The ticket text remains available for manual entry. Treat both as credentials.

Use **Add host** in Android. Scan with the camera, select an image through the
system picker at the lower left, or open the manual dialog at the lower right.
An image containing several valid tickets offers a device choice. Successful
pairing saves the known host before opening its terminal.

A saved host or recent card restores its remembered Session when it is still
live. If it has ended (including after a daemon restart), the app checks the
host's live Sessions: one available Session opens directly, multiple or occupied
Sessions offer selection, and an empty host opens a new default `main`. The
terminal's Retry button follows the same rule. An ended terminal remains ended
until you retry or enter the host again. Taking over another controller still
requires confirmation. Tap the terminal title to expand the
Session list. Each row offers Rename and Delete; confirmed deletion ends its
remote processes. Back to Home detaches and leaves the remote Session running.
Long-pressing a saved host removes only that device's local saved connection.

## Terminal

The subtitle shows the machine name, selected connection route and round-trip
latency, for example `my-mac · Direct · 23 ms`. `Direct` and `Relay` always stay
English, regardless of language settings. Connecting, reconnecting and ended
states use the app language; unavailable latency is shown as `— ms`. A real
disconnect clears old route/latency measurements, while a healthy resize retains
them. The estimate comes from the selected QUIC path, not terminal history queries.

Swipe and fling to read retained output. Nearby history pages are cached and
prefetched; there is no separate history screen. Scrolling back to the bottom
uses the locally maintained live surface without another synchronization wait.
Taking control on another device resizes the same host PTY/terminal to the new
controller's measured rows and columns after synchronization. Terminal programs
can redraw and wrapped output can reflow; the shell process and Session ID remain.
The eight shortcut keys remain visible above the system IME. The four direction
arrows use the same 18 dp vector size and stroke as the upload and keyboard icons;
Esc, Tab, Ctrl and Alt remain text labels. Keyboard animation pans/clips locally and submits
one final terminal size after the animation settles.

The first two toolbar buttons are **Upload image** (photo icon) and **Upload file**
(paperclip icon). Each opens its system picker directly, with one selection per
upload. System Back cancels selection and returns to the same terminal. All file types are
accepted up to **50 MB (50,000,000 bytes)**; original bytes are preserved. A dialog
shows preparation, upload progress, file size and speed. Cancel or Back stops the
operation; tapping outside the dialog does not dismiss it. Failures offer Retry
and Close, and success restores terminal focus without forcing the keyboard open.

While selecting/preparing/uploading, new child input is discarded and unsubmitted
IME composition is retained. Output and connection updates continue. Uploads
survive Activity recreation and backgrounding while this app process remains
alive; leaving the terminal or losing its attachment cancels the upload. After
success the private remote path is inserted once without Enter. The host must
support uploads; storage and retention follow the [desktop upload behavior](./remote-cli.md).

On the live grid, mouse-enabled programs such as Herdr receive completed taps
and vertical swipes as clicks and wheel input. Programs declaring alternate-scroll
receive cursor steps on the alternate screen. Browsing cached output and active
text selection remain local; long press never first clicks the remote program.
Use the keyboard button at the far right of the bottom shortcut bar to show or
hide the system keyboard. Tapping terminal content does not open it; Android
Back can also hide it.

Long-press text, drag either handle across screen edges and use the system Copy
action. Copy preserves captured Unicode, combining marks, spaces and line wraps.
It is atomic up to 512 KiB. Reflow, screen changes or reconnection clear selection;
ordinary output keeps captured text. If retained history changes incompatibly,
existing selected text can still be copied, while unsafe extension stops.

Settings independently choose system/Chinese/English, system/dark/light and
text size through a horizontal 8–16 sp slider in steps of 1 (default 12). Remote program text and explicit terminal colors are preserved.
Backgrounding keeps a healthy application-owned connection on a best-effort
basis. Android can suspend or stop background apps; there is no foreground
service or wake lock. Actual disconnection suspends input rather than replaying
typed commands later. Phone-specific background behavior needs device acceptance.

## Build and test

Native test/embedding runtimes should await `NativeRuntime.shutdown()` before
disposing their handles when reusing the same identity immediately. It closes
Iroh on the still-running executor. Activity/background transitions retain the
application runtime and must not call shutdown.

Use JDK 17, Rust 1.98.0, cargo-ndk 4.1.2, Android SDK 36, Build Tools 36.0.0 and
NDK 28.2.13676358. Install Rust target `aarch64-linux-android`. The checked-in
Gradle 9.1 wrapper and dependency locks supply the Java/Kotlin dependencies.

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk --version 4.1.2 --locked
just android-build
just android-check
just android-install emulator-5554
sh tools/android/build.sh :app:connectedDebugAndroidTest
```

`ANDROID_HOME`/`ANDROID_SDK_ROOT` and `JAVA_HOME` configure tools outside the
checkout. On macOS the build wrapper also locates the standard SDK and JDK 17.
Bindings are generated from the Rust library by the workspace UniFFI generator;
never edit generated Kotlin or copy native libraries manually. Only the current
Android bridge is copied from Cargo output; stale dependency cdylibs are excluded.

Native host integration tests require an explicitly paired emulator and create
their own Sessions. Scanner tests require explicit disposable host ticket/image
fixtures. They skip without these prerequisites. The CI Android job proves
cross-building, lint and JVM checks; it does not claim emulator or phone runtime
acceptance. See the Android task's research evidence for actual runtime results.

## Development and release apps

| Build | Application ID | Launcher label |
| --- | --- | --- |
| Release | `io.github.leonfox28.zterm` | zterm |
| Debug | `io.github.leonfox28.zterm.dev` | zterm Dev |

Both variants can be installed together. Their identities, paired hosts and
preferences are independent, so pair each variant separately. `just android-build`
and `just android-install <serial>` build/install the development variant.
Its instrumentation runner is
`io.github.leonfox28.zterm.dev.test/androidx.test.runner.AndroidJUnitRunner`.
The Kotlin namespace is shared by the sources and does not identify the installed app.

Older debug builds used the release application ID. They remain a separate
signing lineage at that old ID; installing the new `.dev` package does not
remove or migrate them. The previously delivered signed release APK continues
updating normally under the stable release ID and key.

## Signed APK

```sh
just android-apk 1010
adb -s emulator-5556 install -r target/android-apk/1010/zterm-android-arm64-1010.apk
```

Choose a versionCode greater than the previous signed build. The helper creates
one durable private signing identity under `~/.local/share/zterm/android-signing/`
and reuses it. Keep that directory protected and backed up outside the repository;
losing it prevents updating existing installs with the same signature. Custom
SDK/signing locations are supported by `python3 tools/android/apk.py --help`.

The output directory contains the verified APK, SHA256SUMS, signing certificate
details and source/version metadata. The helper checks every packaged native
library and APK zip layout for 16 KiB alignment. An existing debug-signed app
cannot be upgraded with this separate acceptance signature. Uninstalling erases
the app's pairing identity; pair again after a fresh install. Ordinary updates
with the same signing key retain identity, hosts and preferences.

The app persists its controller seed encrypted with an Android Keystore key in
atomic no-backup storage. Backup/device transfer is excluded. A lost key or
corrupt identity produces an error instead of silently generating a new identity.
The signing key and the app's per-install controller identity serve different
purposes and must not be confused.

Xiaomi 17 Pro Max camera, system IME, touch and background behavior must be
verified on the actual phone. Local Pixel emulator results do not establish
Xiaomi-specific acceptance.
