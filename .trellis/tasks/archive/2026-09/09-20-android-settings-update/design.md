# Android settings and application updates — technical design

Startup checking and the 24-hour same-version reminder policy were approved on
2026-09-20 with `按你说的来`. This design incorporates the decision recorded in
[startup research](research/startup-update-check.md).

GUI revision 2 was approved on 2026-09-20. The editable design and its limitations
are recorded in [research/ui-design.md](research/ui-design.md). This document
plans application implementation; no implementation is recorded as complete.

## Change boundary

Settings currently has notification permission text/actions and a standalone
version label, but no repository or update actions. The terminal's failure
Surface wraps short content. Add the approved settings controls and stable
failure/retry layout, retaining existing terminal behavior.

The update path needs a new Android operation owner. Reuse the existing official
release format and Rust trust owner instead of defining another signature
format or making Android call the daemon updater. This is one integrated task:
its notification and failure-card work can be checked independently, but the
final settings screen and approved GUI are the common acceptance target.

Expected changes:

| Owner / files | Responsibility |
| --- | --- |
| `apps/android/.../AppUi.kt`, resources | Settings footer, notification card, app-level update modal host, native Toast effects, Chinese/English text |
| `apps/android/.../TerminalScreen.kt` | Responsive failure/retry card; preserve occupied/ended/retry/Sessions branches |
| `apps/android/.../AppStore.kt`, `AppRepository.kt`, `TerminalNotifications.kt` | Persist the app notification preference and gate delivery at its existing owner |
| New Android `AppUpdates.kt`, `ZtermApplication.kt`, `MainActivity.kt` / root UI | One application-owned startup/manual update operation, first-foreground trigger, bounded network/file work, state and cancellation |
| Android manifest and narrow FileProvider paths XML | Allow user-confirmed installation of the verified cached APK |
| `crates/core/src/release.rs` and a focused Android release module if needed | Shared checksum signature verification and Android release validation |
| `crates/android/src/updates.rs`, bridge exports | Narrow UniFFI verification API; regenerate bindings with the existing tool |
| `tools/release/src/assets.rs` | Call the shared verifier; preserve publishing format and existing release checks |
| Focused tests and existing Android/distribution specs | Verify state, trust boundaries, notification behavior, and responsive UI |

No changes to terminal protocol, daemon/CLI updating, identity storage, signing
secrets, periodic scheduling, or release publication are required.

## Evidence and contracts

- Settings and its current version row: `AppUi.kt:199`; wrapping failure card:
  `TerminalScreen.kt:93` under the Android application package.
- App preferences live in schema-1 atomic state; `AppRepository.kt:125` owns
  serialized saves. Notification delivery occurs at `AppRepository.kt:255`.
- `crates/core/src/release.rs:337` verifies the official release manifest and
  owns the reviewed public key. Its bounded hash helper is at line 392 and
  immutable asset URL construction at line 424. The artifact ceiling is 128 MiB.
- `tools/release/src/assets.rs:428` currently verifies signed checksum bytes;
  line 437 validates Android metadata. Extract shared logic without changing
  accepted release bytes or the inventory contract.
- Android release metadata is emitted by `tools/android/release.py` and binds
  schema/product, version/version_code, source_commit, package, ABI, SDK levels,
  certificate_sha256, signed, length, and APK sha256. The version code allocation
  is defined at `tools/android/release.py:23`.
- Existing release assets include `zterm-android-arm64.apk`, `zterm-android.json`,
  `SHA256SUMS`, and its raw Ed25519 signature. The native manifest does not have
  an Android artifact target. Preserve that schema; bind Android through the
  signed inventory and common release metadata.

Read `.trellis/spec/frontend/android-app.md`,
`.trellis/spec/backend/distribution-lifecycle.md`, and
`.trellis/spec/backend/terminal-notifications.md` before implementation.

## Shared update flow

`AppUpdates` owns one coroutine job and observable state independent of the
terminal repository's busy flag. Rotation and theme/language recreation attach
to that owner rather than starting another request. The owner records startup
versus manual intent and promotes an in-flight startup check when a user taps
Check for updates. No periodic worker or background service is introduced.

| State | Trigger / result |
| --- | --- |
| Idle | First usable cold-launch UI or manual tap starts one check |
| Checking | Verify release metadata; automatic mode has no visible busy UI; manual mode has pending feedback |
| Already current | Manual: emit one native short Toast; automatic: silently return to Idle |
| Available | Apply automatic reminder policy, then show verified version and Later / Download and install |
| Downloading | Stream only after confirmation; display actual progress and Cancel |
| Ready to install | Validate APK, then perform permission/installer handoff from a resumed Activity |
| Failed | Manual or post-confirmation failure: localized feedback; startup-check failure: silent; release busy state |

1. Discover the latest published stable release from the fixed repository
   `https://api.github.com/repos/leonfox28/zterm/releases/latest`. GitHub JSON is
   only an untrusted tag selector; validate a canonical stable `v<SemVer>` tag,
   reject draft/prerelease results, and never trust arbitrary asset URLs.
2. Fetch `zterm-release.json`, its signature, `SHA256SUMS`, its signature, and
   `zterm-android.json` from that same immutable tag. Keep network work off the UI
   thread, require HTTPS including redirects, bound redirect count and reads,
   use connection/read timeouts, and make cancellation close active streams.
   Discovery is capped at 256 KiB, each metadata/checksum body at 64 KiB, and
   each raw signature at exactly 64 bytes. Metadata checking has a 60 s total
   deadline; download gets a separate finite deadline suitable for a 128 MiB APK.
3. Rust authenticates the raw manifest and checksum bytes before interpreting
   their contents. Reject malformed/duplicate inventory paths. Require exactly
   one entry for Android metadata and APK; verify raw metadata bytes against
   their signed checksum. Cross-bind version, tag, source commit, official
   package, arm64 ABI, signed flag, pinned certificate, SDK compatibility,
   positive bounded length, and APK digest. Bind the signed manifest bytes to
   their inventory entry too. Missing or inconsistent assets are check failures.
4. Compare verified SemVer and Android versionCode against installed build
   identity. A candidate must be newer in both orderings to be offered. A valid
   release that is not newer yields `已是最新版` for manual checks and silence
   for startup checks; inconsistent orderings are a failure, not an invitation
   to downgrade. Unofficial/development package or signing identity cannot use
   the official in-place installation path: manual checks show an unsupported
   build message and automatic checks remain silent. GitHub remains usable.
5. On confirmation, stream the fixed-tag APK into an operation-owned private
   cache `.part` file. Enforce signed length and the 128 MiB ceiling while
   reading. Check complete length/hash through shared Rust verification, then
   inspect APK package/version/certificate against the installed official app
   and signed metadata. Android remains the authority for actual APK signature
   validation and installation; archive metadata inspection is not sufficient
   evidence that installation succeeded.
6. Give the system installer a read-granted `content://` URI through a
   non-exported FileProvider whose path exposes only the update cache directory.
   Use the APK MIME type and a user-visible platform installation intent.
   Declare `REQUEST_INSTALL_PACKAGES`; check `canRequestPackageInstalls()` and
   guide to the package-scoped unknown-app-sources settings when required.
   Returning with permission granted may continue the user's pending install
   once; denial returns to an actionable state without silently retrying.

The bridge should return a verified candidate handle (or an equivalently
non-forgeable internal representation) rather than letting UI code fabricate
trusted URLs/digests. Keep release validation in core and expose only the fields
needed for presentation and bounded download verification through UniFFI.

## Startup trigger and reminder policy

Trigger the automatic check once after a resumed Activity has rendered its
initial usable UI and loaded local update policy. Do not run it synchronously
in Application/Activity creation. The application owner holds the process-wide
startup gate, so rotations, theme changes and foreground returns cannot reset
it. A manual check already initiated in that startup window satisfies the gate;
otherwise manual taps join and promote the pending automatic check. One result
is delivered, with manual feedback whenever a user explicitly requested it.

Keep the result/modal host at the app root so startup updates can be presented
without visiting Settings. A newer-version candidate may wait for a resumed,
unobstructed UI; do not stack it over permissions or connection-error dialogs.
The resumed callback can present an existing eligible result but cannot launch
another automatic network check. Once dismissed or consumed, recreation cannot
present the candidate again. A startup check never posts an Android notification
or requests notification permission.

Add optional, bounded update-reminder metadata to the existing schema-1 atomic
state: dismissed version and dismissal epoch milliseconds. Missing metadata
means no reminder suppression. Keep it separate from Preferences while using
the same serialized storage owner, so updates do not overwrite hosts or user
preferences. Local reminder state is a UX preference, not a release trust input.

On Later (and equivalent dialog dismissal), save that candidate's version and
time and suppress its automatic reminders for 24 hours. The same action has the
same meaning when opened manually, but manual checks always bypass suppression.
A higher version is not suppressed by an older version's record. At expiry the
next eligible launch may remind; no timer or scheduled task opens a dialog.
Ignore obsolete version records after an installed-version advance; the single
bounded record is replaced on the next dismissal, avoiding an unnecessary write.
Inject a clock for policy tests and handle invalid/future timestamps with a
bounded fallback so clock changes cannot suppress reminders indefinitely.

Suppress immediately in memory when dismissed, then persist through the atomic
owner. A failed save must not reopen the dialog in that process or corrupt other
state; record a bounded diagnostic. If persistence fails, suppression across
process restarts cannot be guaranteed. Silent startup failures must not leak
through the repository's global error UI. Download/install failures after the
user confirms are visible even when discovery began automatically.

No-update, offline, rate-limit, invalid metadata and unsupported-build outcomes
are distinct internal results even though they all have silent startup feedback.
The terminal notification preference is independent of this check/modal flow.

## Cancellation, lifecycle, and errors

- Every operation has an identity. A cancelled or superseded operation cannot
  publish state, Toasts, progress, or install effects to the current operation.
- Dismissing Available starts no download. Cancelling Downloading closes the
  connection and removes only that operation's partial file. Settings remain
  unchanged on cancellation or any update error.
- Consume Toast/launch effects once from a resumed UI. Activity recreation must
  not repeat a permission prompt, installer launch, or completed-check Toast.
- Keep a verified APK readable while the installer may consume its URI; do not
  delete it immediately after launching the intent. Separate partial-download
  cleanup from handed-off file retention. Clean stale app-owned cache on later
  startup/operations after a bounded retention window, never user data.
- Update work can survive Activity recreation while the app process exists.
  Process death discards in-flight work; the next cold launch can check again,
  with reminder metadata retained. Downloads still require confirmation; resumable background
  transfers are out of scope. Never infer successful installation from intent
  launch or an Activity result alone; the installed build reports its version.
- HTTP/rate-limit/offline, missing assets, bad signatures and incompatible builds
  have bounded internal failure outcomes: quiet on startup, localized on manual
  checks. Storage, permission and installer failures after user confirmation
  have localized feedback. None may fall through to “already latest”.

## Application notification switch

Add `notificationsEnabled: Boolean = true` to existing Preferences and write it
through the current atomic store. Missing fields default to true, preserving
existing users' app preference; Android permission/channel state remains a
separate gate. No schema bump or new preference store is needed.

Effective On means local preference AND runtime authorization AND app/channel
notifications enabled. The rounded row shows this effective state. Retain the
existing notification channel ID and refresh OS state on resume.

- Off immediately closes the delivery gate and persists false. Serialize the
  gate change with posting so events cannot slip through after Off takes effect.
  Use the current notification owner, not a second event consumer.
- On checks OS state, uses the native permission launcher when appropriate, or
  guides to system settings if blocked. Persist successful enablement; denial
  stays visibly Off. Returning from settings must not override a newer user Off
  action. Consume permission/handoff results only for the pending request.
- Serialize preference saves with existing repository writes; do not overwrite
  language/theme/host changes with a stale settings snapshot. On save failure,
  report the storage error and reconcile the delivery/UI gate with committed
  state. Suppress overlapping toggle requests while a save is unresolved.
- Drop disabled events at the existing delivery point. Do not queue or replay
  them when enabled again, clear already delivered notifications, change OSC
  parsing, or disconnect the terminal.

## UI implementation and compatibility

Use existing Compose theme tokens and localized resources. The About card has
GitHub and Check for updates rows; move the standalone installed version into
the update subtitle. Only GitHub retains a navigation indicator. Checking uses
the approved simple download/tray icon and a pending indicator, without a
trailing navigation chevron.

Use `Toast.makeText(..., LENGTH_SHORT)` for “already latest”. Penpot's toast is
an illustration, not an app-owned surface, placement, or animation contract.

The approved 390 px design has 24 px outer margins around a 342 px failure card.
Implement available-width sizing with a sensible tablet maximum, a consistent
minimum height for failure/retry, 28 dp corners, and shared typography/spacing.
Allow growth/scrolling for large text and constrained landscape layouts rather
than fixing all devices to prototype pixels. Preserve closed/ended/lease-lost,
takeover, Retry, and Sessions semantics; use a connection-error title only for
the matching error. Never paint fake terminal output behind initial failure.

Android support remains API 26+ and arm64. Test API 26 certificate inspection
fallback as well as current signing-info APIs, runtime permission on API 33+,
and installation on the target SDK. Debug's `.dev` package must not silently
install the official application alongside itself as though it were an update.

## Rollout and rollback

No production release or device install is part of planning. Implementation
will not change signing keys or publish artifacts. The additive preference and
reminder metadata are compatible with older state; rollback removes the new
UI/gate and ignores the extra fields. The shared verifier extraction preserves
release-tool fixture results and asset bytes. An updater failure must leave
the installed binary, saved hosts, identity, and terminal session intact.

## Primary documentation checked 2026-09-20

- [GitHub latest release endpoint](https://docs.github.com/en/rest/releases/releases#get-the-latest-release): public stable-release discovery; not an authenticity mechanism.
- [Android text Toast](https://developer.android.com/guide/topics/ui/notifiers/toasts): native transient message API and OS-owned presentation.
- [Notification permission](https://developer.android.com/develop/ui/compose/notifications/notification-permission) and [channels](https://developer.android.com/develop/ui/compose/notifications/channels): app delivery and OS authorization are separate.
- [FileProvider](https://developer.android.com/reference/androidx/core/content/FileProvider): content URI and temporary read grant for the cached APK.
- [PackageManager.canRequestPackageInstalls](https://developer.android.com/reference/android/content/pm/PackageManager#canRequestPackageInstalls()): install-source permission gate and package-scoped settings handoff.
- [PackageInstaller](https://developer.android.com/reference/android/content/pm/PackageInstaller): platform owns user-confirmed installation and its result.
