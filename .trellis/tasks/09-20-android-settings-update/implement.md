# Execution plan

Status: GUI and startup/reminder policy approved. This plan incorporates
[startup research](research/startup-update-check.md) and the complete accepted
scope. The user explicitly authorized implementation and requested a new branch;
`feat/android-settings-update` was created before coding. Implementation and the
quality gate are complete; the user has confirmed the local commit and archival. Work is inline;
JSONL dispatch manifests are intentionally empty (`start --allow-empty-context`).

## Before activation

- [x] Record the user's native Toast, app notification switch, icon/chevron,
  and connection-failure decisions in the PRD and Penpot research.
- [x] Inspect release assets, trust owner, Android storage and delivery owners,
  APK version/signing metadata, and platform installation constraints.
- [x] Complete the original manual-only PRD, design, and execution plan.
- [x] Resolve startup-check scope/reminder policy and reconverge all three
  planning artifacts; plan lifecycle/silent-result/reminder-suppression tests.
- [x] Present the final implementation summary and record its approval before
  `python3 .trellis/scripts/task.py start android-settings-update`.
- [x] Load `trellis-before-dev`, package/spec indexes and their relevant
  checklist documents. Recheck working tree; preserve unrelated user changes.

## Ordered implementation

### 1. Shared release validation

- [x] Extract signed checksum verification from release tooling into core,
  keeping current signed bytes/format and reviewed key ownership.
- [x] Add bounded Android release metadata validation and immutable candidate
  construction, including all manifest/inventory/version/certificate bindings.
- [x] Add a narrow Android UniFFI bridge and regenerate bindings using the
  repository native build script. Do not hand-edit generated Kotlin.
- [x] Validate known-good signed fixtures and rejections: altered bytes,
  malformed/duplicate checksum entries, missing APK/metadata, mismatched tag or
  commit, bad package/certificate/ABI, invalid versionCode, and oversized data.
- [x] Prove the release-tool extraction preserves existing fixture behavior.

### 2. Shared update owner, startup policy and Android handoff

- [x] Add one application-scoped update owner with explicit operation identity,
  StateFlow state, startup/manual intent and once-consumed foreground effects.
- [x] Trigger one nonblocking check after the first usable foreground UI, with
  a process-wide gate. Manual checks join/promote an in-flight startup check;
  a manual request already started in the startup window satisfies that gate.
- [x] Persist optional dismissed-version/time metadata through existing atomic
  state. Apply 24-hour automatic reminder suppression across restarts; manual
  checks bypass it and higher versions remain eligible. Cover clock skew and
  failed saves without disturbing host/preferences data.
- [x] Keep startup no-update/failure quiet, including the global error surface;
  host eligible update modals at app level and defer them while backgrounded
  or another permission/error modal is active. No periodic scheduler is added.
- [x] Implement fixed-repository discovery, HTTPS-only bounded fetching,
  verification, version comparison, and differentiated error outcomes.
- [x] Add confirmation-controlled streaming download, true progress, bounded
  temporary files, cancellation, and complete APK verification.
- [x] Add private FileProvider paths and installation permission/intent flow.
  Keep handed-off files alive long enough for the installer to read them.
- [x] Handle rotation, stale completion, denied permission, missing installer,
  and process restart without duplicate work or false installation success.
- [x] Test repeated taps, no download on dismiss, cancellation cleanup,
  failed-check versus current-version distinction, newer-version ordering,
  and one-time UI effects with fake transport/installer boundaries.
- [x] Test one check per cold launch, rotation/resume deduplication, silent
  startup success/failure, manual promotion, deferred foreground presentation,
  23:59/24:00 reminder boundaries, new-version eligibility and restart persistence.

### 3. Notification preference and delivery

- [x] Extend Preferences with a backward-compatible optional boolean.
- [x] Route saves through the current atomic store and serialize app delivery
  gating with notification posts. Cover save failure and concurrent settings.
- [x] Replace the notification paragraph/actions with the approved switch row;
  keep OS permission requests native and refresh effective state on resume.
- [x] Test migration, persisted Off, no posts/replay while Off, enable/deny,
  system-disabled app/channel, and return from settings without overriding Off.

### 4. Approved UI

- [x] Add the bottom GitHub/update card, consolidate version display, and wire
  the pending state, native Toast, confirmation, progress, and cancel actions.
- [x] Apply responsive failure/retry card geometry, preserving terminal state
  and takeover/Retry/Sessions actions.
- [x] Add Chinese/English resources and accessible action/switch labels.
- [x] Compare dark/light rendering with the approved Penpot screens; inspect
  compact width, long English text, large font, and landscape bounds.

## Validation

Run relevant checks after each meaningful implementation slice, then the final
combined checks once. Extend testing only for failures or unresolved concerns.

```sh
cargo test -p zterm-core
cargo test -p zterm-android
cargo test -p zterm-release-tool
just android-check
just android-build
git diff --check
```

Also run `:app:lintRelease` through `tools/android/build.sh` because installation
and official/debug identity differ between variants. Before choosing exact Rust
filters or instrumentation selectors, inspect actual test names/build tooling.
Keep network tests deterministic with local fixtures, without release secrets.

Use a disposable arm64 emulator/test app for Android instrumentation. Reuse
existing SettingsUiTest and TerminalNotificationsTest patterns and test-owned
fixtures. Do not install onto, replace, or alter settings on a personal device
merely to obtain test evidence.

| Area | Required observable verification |
| --- | --- |
| Settings | Correct repository opens; update row has no chevron; installed version and pending feedback remain visible |
| Native feedback | Manual current-version result calls platform Toast once; startup no-update/failure creates no visible feedback |
| Update consent | Available modal shows verified version; dismiss starts zero APK transfers; confirm starts one |
| Trust/download | Tampered or incompatible release never reaches installer; cancelled download leaves no partial artifact |
| Lifecycle | Rotation cannot duplicate check/download/Toast/installer; old operations cannot publish effects |
| Startup/reminders | Cold launch checks once without blocking; Later suppresses that version for 24 hours across restarts; manual checks and higher versions bypass suppression |
| Install | Denied source permission is handled; a test-owned matching-signature APK reaches the system confirmation using readable content URI |
| Notifications | Existing state migrates; Off survives restart and stops future posts without replay; OS-blocked UI stays effectively Off |
| Failure card | Short failure and retry share stable width/minimum height; EN/ZH and both themes fit; takeover and Sessions still work |
| Regression | Font/theme/language persistence, terminal attachment, release fixtures and protected identity storage remain intact |

Actual production APK upgrade acceptance requires an official installed build
and a genuinely newer, correctly signed release. If those prerequisites are
unavailable, report the limit explicitly; use test-owned signed fixture upgrades
for platform handoff coverage without inventing a production update result.

## Review and completion

- [x] Run `trellis-check` after coding; verify cross-layer release data flow,
  resource ownership, cancellation, notification persistence, and UI state.
- [x] Update existing Android, notification, and distribution specs to reflect
  the implemented contracts, including quiet startup checking, reminder policy,
  confirmation-only downloading/installation, and manual native Toast feedback.
- [x] Record commands/results and any device-only limitations; mark PRD
  acceptance complete only where evidence supports it.
- [ ] Follow repository commit/finish workflow while preserving unrelated
  changes. Do not archive before implementation and validation are complete.

## Risky boundaries and rollback points

- Trust verifier extraction: existing release-tool tests must stay green before
  wiring the Android candidate path. Never loosen the reviewed signing policy.
- Atomic preferences: isolate the new boolean and gate from host/identity writes;
  a failed toggle must not leave displayed and committed state contradictory.
- Installer URI lifetime: distinguish cancelled partials from files already
  handed to Android. Never clean another operation's active file.
- Terminal status layout: change presentation in place, retaining state-specific
  actions. Avoid broad terminal rendering refactors.
- Planning introduced no production schema migration, release publication, or
  installed APK replacement. Each implementation slice can be reverted without
  deleting saved application data.

## Verification outcome

See [verification.md](research/verification.md) for exact tests, simulator evidence,
implementation clarifications and remaining production/device boundaries. API 26
and a physical phone were not exercised; compact-height/large-font coverage uses
Compose constraints on the API 36 emulator. No release or remote branch was pushed.
