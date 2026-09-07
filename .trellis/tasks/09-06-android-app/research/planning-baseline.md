# Android Planning Baseline

Recorded on 2026-09-06 during initial scope discovery. The first release now
also includes Session management, selection/copy, and scrolling; current scope
is owned by `../prd.md`, with expansion evidence in `session-selection-scroll.md`.

## Local Build Environment

Read-only inspection found:

- Android Studio is installed at `/Applications/Android Studio.app`.
- The Android SDK includes platforms `android-36` and `android-37.0`, build
  tools `35.0.0` and `36.0.0`, NDK `30.0.15729638`, command-line tools, and
  an `android-36` system-image directory.
- `/usr/libexec/java_home -V` reports Temurin JDK `17.0.20.1` for arm64.
- `adb devices -l` reports no attached devices.
- No standalone `gradle` command was found on PATH. The App project and its
  reproducible build configuration have not been created.
- `emulator -list-avds` lists `Pixel_9`. Its `config.ini` references
  `system-images/android-36/google_apis_playstore/arm64-v8a/`, a 1080 x 2424
  display at density 420, 2048 MiB RAM, and enabled hardware keyboard input.
  The AVD was inspected, not booted or runtime-validated in this planning pass.

The user selected local emulator development and identified the final phone as
Xiaomi 17 Pro Max, correcting an earlier Xiaomi 17 Pro message. The phone's
installed Android/HyperOS version has not been reported or queried.

These observations do not establish a compatible Gradle/AGP/Kotlin/NDK
combination or a successful Android build. Versions must be selected and
verified during technical planning; installed SDK platforms do not determine
the product's minimum supported Android version.

## Existing Client Boundary

- `crates/core/Cargo.toml` and `crates/proto/Cargo.toml` do not depend on the
  host terminal or platform crates.
- `crates/daemon/Cargo.toml` directly depends on both `zterm-platform` and
  `zterm-terminal`; linking the whole daemon would import host dependencies.
- `crates/daemon/src/client/transport.rs` opens a Unix socket for both its
  direct local attachment and opaque remote-tunnel modes. That frontend
  transport is not an App-owned Iroh client ready for direct Android reuse.
- `docs/remote-cli.md` defines the existing text-ticket pairing entry,
  frozen-Session reconnect target, synchronization before input, and explicit
  controller takeover. Preserve these contracts when selecting the App seam.

The design must distinguish shared protocol/state behavior from Unix IPC and
host-runtime ownership. This finding does not yet choose a bridge or extraction
strategy.

## Mobile Lifecycle Evidence

- The existing roadmap's R9 cold-tab rule releases the attachment/lease and
  restores a snapshot before input
  (`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:181-184`).
  That tab policy is evidence for reuse, not a decision about Android OS
  background execution.
- `docs/remote-cli.md:219-236` defines preparing/synchronizing/active/reconnecting,
  reconnecting to the frozen Session ID, and discarding input during recovery.
  Session termination or daemon restart must not silently create a replacement
  Session with the old name.
- The connection-multiplexing research gives the App process ownership of its
  mobile connection manager. Process loss removes transport, while the host
  Session remains owned by the remote daemon.
- The user chose to leave the active connection alone when backgrounding the
  first-release App. The PRD's LIFE requirements own this decision; the earlier
  proposed background-detach policy is superseded.
- Keep the connection/attachment owner independent of Activity visibility.
  Stopping UI rendering or lifecycle-bound camera capture must not cancel that
  owner. While execution is available, shared Session processing retains the
  existing bounded-state and synchronization rules. Background suspension must
  not create an unbounded output queue or backpressure the host PTY.
- On foreground return, retain a healthy synchronized attachment. An actual
  lost or uncertain connection uses normal liveness/synchronization recovery;
  no unconditional teardown is justified by `onResume` alone.

Android's official documentation says Doze restricts network access, while
cached processes can be killed and may receive limited or no execution time.
Consequently, leaving a connection open is best-effort behavior, not a guarantee
that it remains usable throughout backgrounding. The first release does not add
a separate background keepalive mechanism. These are platform constraints;
Xiaomi-specific behavior has not yet been measured.

Sources checked on 2026-09-06:
[Doze and App Standby](https://developer.android.com/training/monitoring-device-state/doze-standby),
[Processes and App Lifecycle](https://developer.android.com/guide/components/activities/process-lifecycle).
