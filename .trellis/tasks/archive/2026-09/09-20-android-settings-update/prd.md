# Android settings updates and connection-failure presentation

## Goal

Let Android users discover official application updates through a quiet startup
check or a manual check at the bottom of Settings, and open the project's GitHub
repository there. The user explicitly requested
a Trellis task and a Penpot GUI review before application implementation.
The user's follow-up also requests fixing the undersized, inconsistent
connection-failure popup in the same task.
In the second GUI review, the user required a native Android Toast and a simpler
update entry, then explicitly selected an application-level notification switch.

## Confirmed facts

- Settings is a scrollable Compose screen with language, theme, font size and
  preview, terminal notification settings, and a version row
  (`apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt:199`).
- Current source version is 0.1.34 (`Cargo.toml:17`). The prototype's 0.1.35 is
  illustrative, not a claim that a newer release exists.
- The existing Penpot document is `Zterm Android · UI v1`, file
  `c828d3cf-7d4e-8145-8008-98f4fa037d48`; its old settings drawing predates
  the current font slider and terminal notification controls.
- The terminal failure UI is an in-place Surface whose width wraps its content
  (`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt:93`).
  Its short message and text buttons explain the small footprint. Retry and
  Sessions are existing actions; this is not an Android Dialog.

## Requirements

- SETUP-1: Put GitHub and Check for updates below the existing settings controls.
  GitHub opens the project repository in a browser.
  Check for updates is an action row without a trailing navigation chevron;
  use the same simple download icon family as the update confirmation.
- UPDATE-1: Manual Check for updates is initiated by a tap. Give visible feedback
  while checking and prevent duplicate requests from repeated taps.
- UPDATE-2: If a manual check finds no newer update, show `已是最新版` using Android's
  standard text Toast (`Toast.makeText`, short duration). Do not recreate it as
  a Compose surface, custom toast, or snackbar. The OS controls its presentation.
- UPDATE-3: If a newer update is available, ask whether to download and install
  it in a modal, subject to the automatic reminder policy in UPDATE-7. Provide
  a clear dismiss action and `下载并安装` confirmation.
- UPDATE-4: Download starts only after confirmation; installation follows the
  Android platform flow. Cancelling must preserve the current settings.
- UPDATE-5: Use the existing official stable Android release and signing
  contract. Offer only a verified newer compatible version; failed checks must
  not claim the app is already current. Do not install an official release as
  an in-place update of an incompatible development package/signing identity.
- UPDATE-6: Once per cold app process launch, asynchronously check after the
  first foreground screen is usable. Do not delay rendering or terminal work.
  Returning from background, rotating, or recreating the Activity does not
  initiate another check. An automatic check with no update or any check failure
  produces no loading overlay, Toast, notification, or error dialog. Show an
  eligible newer-version modal only in the foreground, without stacking over
  permission or error dialogs.
- UPDATE-7: After `稍后再说` (the approved GUI's `稍后` action), persist a 24-hour
  suppression of automatic reminders for that same version. After expiry,
  another eligible cold launch may remind again; no timer wakes the app. A higher
  version is eligible independently. Manual checks bypass suppression and
  always give explicit result/error feedback. Coalesce concurrent startup and
  manual checks, with one manual result when the user joins an ongoing check.
- DESIGN-1: First deliver editable Penpot screens and clickable review flows
  for the settings footer, checking, latest-version toast, and update dialog.
  Preserve existing colors, typography, and rounded surfaces; preview light
  theme as well as the primary Chinese dark theme.
- FAILURE-1: Give the connection-failure status card a stable, screen-relative
  width and visual hierarchy consistent with the update confirmation. Preserve
  existing retry/session actions and state semantics. Match dark/light themes.
- FAILURE-2: Keep the same card dimensions while retrying so a short pending
  label does not collapse it. Never fabricate terminal output on initial failure.
- NOTIFY-1: Replace the mismatched notification paragraph/actions with a rounded
  preference card containing `终端通知`, concise status text, and an application
  notification switch. This interaction was explicitly selected by the user.
- NOTIFY-2: Switching off immediately stops future notification delivery and
  persists that preference. Switching on checks OS permission/channel state,
  requests permission when possible, and otherwise guides the user to system
  settings. A denied or blocked permission must not display an effective On
  state. Refresh actual authorization when returning to the app.
- NOTIFY-3: This preference controls app delivery, not the Android permission
  itself. Preserve existing terminal operation and drop notifications received
  while disabled without later replay. Existing delivered notifications are not
  automatically cleared by toggling off. This terminal notification setting
  does not disable update checking or in-app update dialogs.

## Acceptance criteria

- [x] GitHub and Check for updates appear at the end of Settings, with the
  installed version visible near the update entry (SETUP-1).
- [x] Tapping GitHub opens the correct repository (SETUP-1).
- [x] Manual checking visibly transitions to a pending state (UPDATE-1).
- [x] Manual no-update results use the native system text Toast, without an app-drawn
  notification surface or confirmation modal (UPDATE-2).
- [x] An available update shows its version and explicit dismiss/download
  choices; dismissing does not download (UPDATE-3/4).
- [x] Approved download can hand off to Android installation without deleting
  saved application data (UPDATE-4; implementation acceptance).
- [x] Invalid or incompatible release data never reaches installation; network
  and verification failures are distinguishable from no update (UPDATE-5).
- [x] Cold launch performs at most one nonblocking check; recreation and resume
  cause no extra request, and automatic no-update/failure produces no visible
  feedback (UPDATE-6).
- [x] An eligible automatic update uses the approved modal only when foreground
  UI is ready; it never downloads without confirmation (UPDATE-3/4/6).
- [x] Dismissing version A suppresses its automatic reminder for 24 hours across
  restarts; a newer version B and manual checking remain eligible (UPDATE-7).
- [x] A manual check joining a startup check receives one explicit result,
  without duplicate requests or dialogs (UPDATE-1/7).
- [x] Penpot review links, board IDs, and illustrative-data limitations are
  recorded in task research before product code is edited (DESIGN-1).
- [x] Connection failure and retrying use consistent wide cards, readable
  title/body and full-size actions in both themes (FAILURE-1/2).
- [x] Notification controls match the settings cards in both themes; off stops
  new deliveries, is persisted, and does not disrupt terminal use (NOTIFY-1/3).
- [x] Enabling, permission denial, system-blocked state, and return from system
  settings display the effective state correctly (NOTIFY-2).

## Out of scope

- Periodic or app-closed background checks, automatic APK downloads, forced
  updates, and GitHub account sign-in. The foreground startup check runs
  asynchronously; it does not add a background service or schedule.
- Remote daemon/CLI updates; this task concerns the Android application.
- Product-code changes, actual downloads or APK installation during GUI review.

## Review status

- GUI revision 2 was approved by the user on 2026-09-20 with `好 就这样吧`.
  See [the GUI review and clickable links](research/ui-design.md).
- Startup checking and the 24-hour same-version reminder policy were approved
  by the user on 2026-09-20 with `按你说的来`; see
  [the decision record](research/startup-update-check.md).
- [Technical design](design.md) and [execution plan](implement.md) incorporate
  the complete scope. No unresolved product decisions remain.
- Implementation was authorized with `好 开始执行吧，另外你没新创建branch` on 2026-09-20.
  Branch `feat/android-settings-update` was created before coding and the task activated.
  Implementation and checks are complete; see [verification](research/verification.md).
  The user confirmed the local commit and archival with `确认提交并归档`.
- Actual production upgrade verification needs an official installed build and
  a newer signed release. The prototype version is not a release claim; use
  test-owned fixtures for platform handoff tests when production prerequisites
  are unavailable. This does not waive implementation acceptance.
