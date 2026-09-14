# v0.1.32 release handoff

On 2026-09-14 the user requested the release workflow, then authorized continuing
with unrestricted tool execution. This supersedes the implementation-turn
exclusion of commits, archive and release. Phone operations remain excluded.

## Product scope

Ship unified live-screen IME movement and early endpoint resize in both
directions, retained displayed-frame handoff, content-keyed bounded row layers,
empty-space glyph skipping, source-relative unchanged-row projection and
display-paced preparation. Preserve native semantic installation, ACKs and
input/source authority. No wire/schema/dependency change is included.

Text-run batching was evaluated and removed; its patch is research only.
The specs contain the accepted behavior and lifetime contracts.

## Validation

- `just doctor` passes on this macOS arm64 host.
- Android debug/test APK assembly, debug/release lint and the JVM test task pass
  on 2026-09-14. The JVM task is NO-SOURCE, not runtime test evidence.
- The debug APK SHA-256 is unchanged from the checked 2026-09-10 artifact:
  `eb3831a050cb3993149fa38bd6435672f769a5c48de07d0e3c844d68bdd53f03`.
  Native ELF and ZIP 16 KB alignment checks pass again.
- The unchanged artifact's earlier emulator evidence remains 25 passed Android
  checks across the focused suite and real-host FFI test, plus one explicit
  host-fixture assumption skip. See `peer-adoption.md`; no device was used in
  this release-preparation turn.
- Full local `just check` passes on 2026-09-14: portable policy, Clippy,
  workspace tests/docs, dependency checks and local relay checks. Log:
  `/tmp/zterm-release-0914-check.log`.
  Required PR/main CI and final signed installer/asset checks remain release
  workflow gates, not evidence supplied by the local debug APK.

## Deferred follow-ups

The tiny phone-specific IME endpoint overshoot was not reproduced on the owned
emulators and was explicitly deferred by the user. The remaining phone Main
thread end-of-animation burst and correlated host/child whole-transition scene
matrix remain open measurements. Do not claim universal smoothness or zero
resize latency. The three unrelated implementation-review findings (appearance
adoption, gesture retirement and old-API underline compatibility) remain separate
follow-ups, with original evidence in `review.md` and `implement.md`.

Archive this delivered review and the approved implementation with these gaps
preserved; publication is not additional physical-device acceptance.
