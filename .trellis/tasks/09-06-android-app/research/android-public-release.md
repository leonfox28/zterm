# Android public release addition — 2026-09-07

The user authorized the normal release workflow with Android added to the
existing macOS arm64 and Linux arm64/x64 publication scope. Next version is
0.1.26, based on current origin/main v0.1.25. Work remains inline, without agents.

The missing behavior is public APK delivery from the same exact green main
candidate as the native archives. Owners are CI candidate assembly, the protected
signing job, and tools/release inventory verification. Android build tooling owns
package/version/certificate/16 KB validation. Preserve the existing release APK
certificate so installed acceptance packages upgrade without losing state.

Changes: integrate Android build/lint/JVM checks into ci.yml; retain the unsigned
APK in the main candidate; sign it in the existing release environment; include
APK and build metadata in the exact published inventory. Sign SHA256SUMS with the
existing Ed25519 release key to authenticate this complete inventory, while
preserving the schema-v1 native manifest and installer unchanged. APK platform
signing remains the Android installation authority. No Android updater, store
publication, Intel macOS/Windows/relay-image publication, or physical-phone
operations are part of this release step.

Formal Android version codes are deterministically derived from Cargo SemVer;
local acceptance overrides remain available. Main CI embeds the exact source
commit into a packaged build identity and Rust. The tag job only adds the APK
signature, never rebuilds its payload. Release signing credentials remain outside
Git and are provisioned into the existing protected release environment.

Validation: portable release policy and inventory tests, full native workspace
checks, Android release/debug build/lint/JVM checks and an emulator installation
upgrade preserving data. Hosted CI owns Linux runtime and final exact main
candidates; publication must finish with verified immutable release assets.

Local validation before PR: `just check` passed (613 Rust tests, seven intentional
platform ignores across 53 suites; Clippy/docs/dependency/source/secret/release
policy gates passed). Android debug and release APK builds/lint plus the
debug JVM task completed with NO-SOURCE (runtime tests are Android instrumentation,
not a separate JVM suite). The real APK signing helper passed with
the retained certificate and exact ZIP-payload equality; all six native libraries
and APK ZIP alignment pass 16 KB checks. Formal GitHub signing secrets were
provisioned in the existing protected release environment without exposing values.

The dedicated Zterm_PublicRelease API 36 emulator accepted an update from the
existing signed 1016 APK to the candidate (versionCode 102499 for the pre-version-
bump local test). Identity SHA-256 and firstInstallTime were unchanged; the
updated App launches. The formal v0.1.26 asset will be checked separately after
publication. Daily emulator-5554's release fixture has a debug certificate and
was deliberately preserved.
