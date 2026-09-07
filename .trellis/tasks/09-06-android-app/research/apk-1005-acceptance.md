# Android APK 1005 acceptance

Superseded for installation by 1006, which separates development/release IDs
and fixes initial resize when attaching to a retained Session. See the latest
entry in implementation-progress.md; this file preserves the 1005 evidence.

Artifact: `target/android-apk/1005/zterm-android-arm64-1005.apk`.
Version name 0.1.24; versionCode 1005; arm64-v8a; Android API 26+; target 36.
Source: `holy-zebra`, refreshed mainline `de25a38` plus current Android changes.
The source changes are local and uncommitted. No store publication or push.

SHA-256:
`9e59d011299eaa8e26950d2909c446703f66c7f6eb44a4b9d7da9b9fbce20b42`

Signed with the durable external acceptance key. Signature plus all six native
libraries' ELF/zip 16 KiB alignment are verified; certificate/build metadata and
SHA256SUMS accompany the APK. Previous APK codes are intermediate.

## Observed runtime evidence

| Case | Result |
| --- | --- |
| Mac and Linux Session lifecycle, Unicode, scroll/copy, resize | Pass on API36 arm64 emulator with real hosts |
| Actual CameraX virtual scene, system photo picker, manual credentials | Pass against real Mac pairing |
| Direct and forced relay transport | Pass; actual path asserted in relay run |
| Native touch, 3-screen selection, Android floating Copy, canceled copy | Pass, signed 1005 |
| Full system IME shown/hidden, persistent shortcuts, stale IME rejection | Pass, signed 1005 |
| All nine language/theme pairs, 12/14/16 font, portrait/landscape | Pass; exact Session retained |
| Title overlay and rename/create/cancel/delete/switch | Pass, signed 1005 |
| Background/Activity recreation and first return input | Pass, signed 1005 |
| Desktop takeover -> command -> Android explicit takeback | Pass, 4.689s, same exact Session |
| UID-scoped network interruption and recovery | Pass, 55.902s; no disconnected-input replay |
| Hidden cursor movement on main and alternate screens | Pass, latest native source, 4.348s |
| JNI lifecycle, encrypted identity failure behavior, signed identity retention | 8 tests pass on signed 1005 |
| Signed upgrade / fresh install identity | Upgrade retains identity/host; reinstall generates a new identity |

The complete signed UI run passes in 21.033s. Its observed cache peak is
15,380,161 bytes (<16 MiB), with 432 local hits, one miss and 15 bounded queries.
These figures describe the test fixture, not general device performance.

## Quality and remaining acceptance

Workspace all-feature tests/rustdoc, strict Rust Clippy, dependency policy,
license/advisory checks, relay static checks and Android lint pass. The latest
native final-frame change has a deterministic regression and all seven Android
Rust tests pass. The JVM Gradle unit-test task is NO-SOURCE; no JVM suite is claimed.
Trellis reference validation passes; execution stayed inline with no sub-agents.

Xiaomi 17 Pro Max physical camera/IME/touch/background behavior, OS/page size and
network acceptance remain pending. Broader fault injection and stress permutations
are explicitly unchecked in implement.md. The task is not archived.

Install the APK directly on the phone. Updates require the same signing identity;
installing over a different debug signature requires a separate fresh installation
and pairing. On a configured host, `zterm pair create` supplies a ticket for the
scanner's lower-right manual dialog. The new CLI source also supports `--qr` and
`--qr-image`; those flags require the new CLI build.

Temporary ticket/QR fixtures and test-owned Sessions were cleaned up. Both emulator
firewall chains were restored. The user's existing Linux main Session was retained.
