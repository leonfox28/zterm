# Android notification delivery: scope research

Date: 2026-09-17. User follow-up: how should the Android app deliver notifications?
Confirmed scope: "首版先做连接有效时发系统通知". No background service or push
infrastructure is included.

## Current repository evidence

- `apps/android/app/src/main/AndroidManifest.xml` declares Internet/network/camera
  permissions and one Activity, but no notification permission or service.
- `apps/android/app/build.gradle.kts:22` targets compile SDK 36; minimum SDK is
  26 and target SDK is 36 (lines 27-28).
- `ZtermApplication.kt:7` owns the retained `NativeRuntime` and `AppRepository`.
  `AppRepository.kt:27` creates an application-owned coroutine scope; attachment
  frame collection begins around line 250 and explicit retirement detaches the
  prior terminal around line 303.
- `AppRepository.kt:363` routes terminal visibility independently of connection
  teardown. `.trellis/spec/frontend/android-app.md:319` explicitly says that
  background visibility stops gestures/speculation, not the connection.
- The existing Android task design at
  `.trellis/tasks/09-06-android-app/design.md:559` specifies that background entry
  keeps the runtime/attachment while the OS allows execution. It explicitly did
  not add a foreground service, wake lock or dedicated keepalive subsystem.
- `crates/android/src/terminal.rs:915` receives typed shared-client events in
  the retained actor. Its frame publication is state-oriented; existing clipboard
  effects are discarded at line 962. There is no notification consumer today.

## Reusable data path

```text
Hosted program -> OSC 9/777 -> host parser -> typed transient Session event
  desktop: shared client -> DesktopPresenter -> outer terminal
  Android: shared client -> NativeTerminal event bridge -> Kotlin notification API
```

The same host parser/domain/wire work serves both platforms. Android has no
outer terminal for re-emitting ANSI. Kotlin owns NotificationManager/channel/
permission/presentation behavior; Rust owns validated event content and
attachment lifetime. An independent bounded event consumer should be owned by
the application repository, not by an Activity or the conflated NativeFrame
stream. Activity recreation or screen repaint must not duplicate a notification.

The current controller-target policy remains applicable: an attached Android
controller can receive its Session's live requests. Sending the same request to
a phone that is not attached while a desktop controls the Session would require
additional subscription/routing scope and is not implied by this proposal.

## Android platform evidence

- Android 8+ notification channels are required for this app's supported SDK
  range. Channels give users control over sound/importance. Source:
  [Android notification channels](https://developer.android.com/develop/ui/compose/notifications/channels).
- Android 13+ ordinary notifications require the runtime `POST_NOTIFICATIONS`
  permission. Denial disables ordinary notification display and must not break
  the terminal connection. Permission requests need a user-facing Activity flow;
  receiving a background event is not a reason to launch UI. Source:
  [Android notification permission](https://developer.android.com/develop/ui/compose/notifications/notification-permission).
- A foreground service is a separate user-visible background-execution feature,
  with its own ongoing notification and lifecycle/start/type restrictions. Adding
  notification posting alone does not keep a connection alive indefinitely.
  Source: [Foreground services overview](https://developer.android.com/develop/background-work/services/fgs).

These docs describe platform constraints; they are not runtime evidence for this
app on any particular handset or Android vendor build.

### Instrumentation finding

On the disposable API 36 emulator, three ordinary notifications caused a fourth
system-generated group summary to appear asynchronously (flags 1808, including
`FLAG_GROUP_SUMMARY`, with no notification body). This is not duplicate event
consumption. Instrumentation counts the content notifications separately from
summaries. The platform documents automatic grouping and device-dependent behavior:
[Create a group of notifications](https://developer.android.com/develop/ui/views/notifications/group#automatic-grouping).

## Confirmed initial behavior

The user selected system notification delivery when the app owns a live eligible
attachment and actually receives the event, reusing the existing connection
lifetime. While the process and connection remain active, this also allows
background receipt. A disconnected/killed process receives no new events and
there is no catch-up on reconnect, preserving the user's prior decision.

Foreground/background display uses Android's ordinary notification channel and
permission policy. The delivery consumer belongs to AppRepository, and the
permission request belongs to a user-initiated foreground Settings action.
Denied/disabled events are skipped, not held until permission is granted.

Sustained background/lock-screen connectivity, foreground services and delivery
after process termination remain outside this task. Building an APK alone does
not verify notification display or background receipt; targeted runtime evidence
is required for those claims. Final contracts and acceptance are in the task's
PRD/design/implementation plan.
