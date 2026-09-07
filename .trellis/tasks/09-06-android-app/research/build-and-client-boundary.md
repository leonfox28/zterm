# Build and Shared Client Evidence

Researched on 2026-09-06. These are source/publication checks, not a completed
Android build. The selected configuration is authoritative in `../design.md`.

## Repository Evidence

| Evidence | Consequence for the design |
| --- | --- |
| `crates/daemon/src/client/session.rs:1-190` | One existing attachment/reconnect owner, currently mixed with socket paths, local mutation helpers and daemon DTOs; extract behavior and inject adapters |
| `crates/daemon/src/client/transport.rs:1-80` | Both direct and remote desktop attachments begin with UnixStream; the Android adapter must open Iroh streams directly |
| `crates/daemon/src/client/view.rs:1-100` | Typed view state exists, but is Unix-gated and uses DaemonError; separate portable events/state from platform support |
| `crates/daemon/src/pairing_service.rs:613-804` | Existing controller pairing path includes transcript and normal confirmation; reuse its behavior rather than reimplementing authentication in Kotlin |
| `crates/daemon/src/connection_broker.rs` | Broker combines outbound handshake and host authorization/network/store ownership; extract the needed shared handshake components rather than moving the whole broker |
| `crates/cli/src/terminal_ui/surface.rs` | Small validated surface owner can move without moving the ANSI compositor |
| `crates/cli/src/terminal_ui/keyboard.rs:285-452` | Mode-aware functional/modifier encoding exists; distinguish it from outer-terminal input decoding and local shortcuts |
| `proto/zterm/v2/terminal.proto` | Input is bytes; semantic surface modes control encoding. Local state/path events are explicitly forbidden on remote wire |
| `tests/terminal-dependency-policy.sh` | Existing dependency invariant has one owner; extend it for the mobile-facing crates |
| `README.md:78-82` | Product SemVer comes from Cargo workspace, including future mobile Apps |
| `.trellis/tasks/09-02-migrate-alacritty-terminal/implement.md:1350-1353` | macOS/Linux local/direct/relay evidence is still unchecked in the current task record; verify before Android implementation |

## Android Toolchain

AGP 9.0.1's documentation lists Gradle 9.1.0, JDK 17, build tools 36.0.0, and
default NDK r28c, and supports up to API 36.1. Its built-in Kotlin uses KGP
2.2.10. Use its public Android Components/source APIs; the old variant APIs are
not a valid new integration baseline.

Source: [AGP 9.0 release notes](https://developer.android.com/build/releases/agp-9-0-0-release-notes).

Use the Compose compiler plugin aligned to the Kotlin version. The UI toolkit
does not require Android's terminal protocol/state to move into Kotlin.

Source: [Compose dependencies and compiler](https://developer.android.com/develop/ui/compose/setup-compose-dependencies-and-compiler).

The following candidate versions were found in their published Google Maven
metadata: Compose BOM `2026.01.01`, CameraX `1.5.2`, Activity `1.12.2`, and
Lifecycle `2.10.0`. These metadata reads establish publication, not mutual
compatibility or runtime results. The first scaffold build must confirm both.

Primary metadata:
[Compose BOM](https://dl.google.com/dl/android/maven2/androidx/compose/compose-bom/maven-metadata.xml),
[CameraX](https://dl.google.com/dl/android/maven2/androidx/camera/camera-core/maven-metadata.xml),
[Activity](https://dl.google.com/dl/android/maven2/androidx/activity/activity-compose/maven-metadata.xml),
[Lifecycle](https://dl.google.com/dl/android/maven2/androidx/lifecycle/lifecycle-runtime-compose/maven-metadata.xml).

## Rust/Kotlin Bridge

UniFFI generates Kotlin bindings and exposes Rust futures as suspend functions.
Its async guide calls for library-specific cancellation. Therefore the bridge
has explicit operation cancellation and separate App/session lifetime ownership,
not an assumption that canceling a screen coroutine shuts down Rust work.

Sources: [UniFFI overview](https://mozilla.github.io/uniffi-rs/latest/),
[Async support](https://mozilla.github.io/uniffi-rs/latest/futures.html).

The Kotlin integration requires an Android JNA AAR. The guide's legacy Gradle
example is not compatible with the AGP 9 variant API and should not be copied
literally. Avoid high-frequency per-cell native calls and manual editing of
generated bindings.

Source: [UniFFI Kotlin integration](https://mozilla.github.io/uniffi-rs/latest/kotlin/gradle.html).

Registry metadata reported stable UniFFI `0.32.0`, QR encoder `0.14.1`, and JNA
`5.19.1` during this pass. Pin generator/library together. JNA's release-page
HTML did not provide usable release entries, so the Maven artifact metadata was
used to establish the published version; packaged ELF properties still require
inspection in the build gate.

Primary metadata:
[UniFFI registry](https://crates.io/api/v1/crates/uniffi),
[QR encoder registry](https://crates.io/api/v1/crates/qrcode),
[JNA Maven metadata](https://repo.maven.apache.org/maven2/net/java/dev/jna/jna/maven-metadata.xml).

`cargo-ndk` supports Android ABI selection, native library output directories,
and explicit NDK selection. Release `4.1.2` is published. Select the intended
NDK explicitly rather than inheriting the newest installed revision.

Sources: [cargo-ndk usage](https://github.com/bbqsrc/cargo-ndk),
[cargo-ndk 4.1.2](https://github.com/bbqsrc/cargo-ndk/releases/tag/v4.1.2).

Android documents 16 KB alignment defaults for NDK r28+, and separately requires
checking prebuilt libraries. Audit every `.so` in the APK, including JNA and
scanner dependencies, plus APK zip alignment; compiler selection alone is not
evidence that all bundled libraries load on a 16 KB device.

Source: [16 KB page-size support](https://developer.android.com/guide/practices/page-sizes).

## Native Input and Identity

`InputConnection` distinguishes composing text from committed text. The
terminal's local composition buffer must not emit every preedit update into
the remote process. Terminal modes still determine emitted key bytes.

Source: [InputConnection API](https://developer.android.com/reference/android/view/inputmethod/InputConnection).

Android Keystore can hold a non-exportable wrapping key. The proposed seed
encryption uses that facility while acknowledging that Iroh needs the unwrapped
seed in-process. Keystore operations belong off the UI thread. Store identity
and pairing data under no-backup storage with explicit backup/transfer rules;
normal updates preserve state, but reinstall must not restore the old identity.

Sources: [Android Keystore](https://developer.android.com/privacy-and-security/keystore),
[Backup behavior and exclusions](https://developer.android.com/identity/data/autobackup).

## Technical Validation Deferred to Execution

The plan fixes product behavior and module boundaries. It intentionally does
not claim results for the combined AGP/UniFFI/Rust build, device page size,
physical QR readability, Android Iroh DNS/route behavior, or renderer frame
cost. The first build gate and subsequent emulator/phone checks own that
evidence. If a technical failure would require changing the agreed scope or
protocol architecture, revise the planning artifacts before proceeding.
