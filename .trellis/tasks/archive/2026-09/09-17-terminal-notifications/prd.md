# 终端通知转发与安卓系统通知

## Goal

Deliver ordinary notification requests emitted by programs running inside zterm
to the currently connected controlling client. Desktop clients forward the
request to the outer terminal, with Ghostty as the primary acceptance target;
the Android app posts a system notification while its connection is valid.

## Background and Decisions

- Requested on 2026-09-17; branch `codex/terminal-notifications` starts from
  `main` at `71f41d3`.
- The user explicitly selected OSC 9 and OSC 777 only for the first version.
- The user explicitly chose no buffering while disconnected and no delivery of
  missed notifications after reconnect.
- Both local and remote desktop attachments use the generic terminal path.
  Herdr is a compatibility fixture, not an application-specific integration.
- The user confirmed Android system notifications while the connection is valid.
  Reuse the app's existing connection lifetime, including background execution
  while the OS permits it; do not add sustained background connectivity or push.

## Pre-implementation Evidence

- The OSC dispatch at `crates/terminal/src/ingress.rs:651` currently rejects
  notification commands as unsupported, so no notification payload is forwarded.
- `crates/terminal/src/ingress.rs:356` recognizes BEL as an `AudibleBell` side
  event, but `crates/daemon/src/terminal_driver.rs:393` does not forward side
  events to the outer terminal. Standalone bell forwarding remains out of scope.
- Clipboard writes already have a transient controller-targeted path through
  Session, the shared client, and the desktop presenter. Their latest-only
  replacement behavior must remain independent of distinct notifications.
- Herdr `0.9.1` is installed and its local config selects terminal delivery.
  Its Ghostty backend emits OSC 9, joining title and body into one message.
  Backend detection is environment-dependent and precedes notification emission.
- Android retains its NativeRuntime and attachment in the application repository
  when the Activity backgrounds. It has no notification backend, notification
  permission declaration, or foreground service today. Frame delivery conflates
  state and therefore cannot carry distinct notification events.
- Supporting anchors and primary protocol sources are in
  [notification-path.md](research/notification-path.md) and
  [herdr-notifications.md](research/herdr-notifications.md), with Android evidence
  in [android-notifications.md](research/android-notifications.md).

## Requirements

| ID | Requirement |
| --- | --- |
| R1 | Forward valid OSC 9 ordinary notification messages and OSC 777 `notify` requests from local and remote desktop sessions to the connected outer terminal. |
| R2 | Preserve the originating notification form and its supported message/title/body text, including UTF-8. Desktop display, sound, focus policy, and permissions belong to the outer terminal and OS; Android maps the validated text to an ordinary system notification. |
| R3 | Preserve ordinary terminal input/output, screen content, history, cursor, session lifecycle, and existing clipboard behavior. Notifications must not wait for a screen repaint. |
| R4 | Preserve the OSC 9 message form emitted by Herdr 0.9.1's Ghostty backend through the generic path. Verify application backend detection separately; forwarding support does not force an application to emit a request. |
| R5 | Deliver only to the eligible controlling attachment at request publication time. Drop requests without a controller. Clear pending requests on detach, actual connection loss, takeover, and session end; never transfer or replay them to a later attachment. |
| R6 | Keep notification processing and pending delivery bounded. Preserve distinct notifications in order within capacity; overload must not block PTY processing. Invalid/oversized requests must not create visible payload residue, unrelated outer-terminal controls, or content-bearing diagnostics. |
| R7 | An Android controlling attachment posts live notifications through Kotlin's system notification API, independently of screen frames and Activity recreation. Reuse the existing foreground/background connection lifetime; no connection or eligible controller means no delivery. |
| R8 | Android provides a notification channel and a user-initiated permission/settings flow. Permission denial or a disabled channel skips the event without disrupting terminal use or retaining it for later permission grants. Respect Android's display and sound settings. |

## Acceptance Criteria

- [x] A1 / R1, R2: Each supported form reaches the outer terminal once in local
  and remote attachment tests, with the correct message or title/body. Both main
  and alternate-screen application output can produce notifications.
- [x] A2 / R2, R6: Complete and fragmented requests, BEL/ST termination, and
  supported C1 framing have the same result within capacity. Chinese text and
  UTF-8 continuation bytes resembling C1 controls are preserved.
- [x] A3 / R3: Notifications do not appear as screen text, move the cursor,
  change history/frame baselines, or wait for synchronized-output publication.
  Clipboard requests and notifications in the same output burst both work.
- [x] A4 / R5: Requests generated while detached and pending requests retired by
  a connection loss do not appear after reattach/reconnect. Fresh requests after
  the replacement attachment becomes eligible still work.
- [x] A5 / R5: Takeover/session end retire pending notifications. A new controller
  receives no old controller's pending content; observers receive no broadcast.
- [x] A6 / R6: Multiple requests within capacity remain distinct and ordered.
  A flooded or slow consumer remains bounded and does not stop normal PTY drain.
- [x] A7 / R1, R6: OSC 9 progress/ConEmu subcommands, other OSC 777 extensions,
  malformed UTF-8, cancellation, and oversized/invalid payloads cause no ordinary
  notification or new outer-terminal action. Diagnostics redact message content.
- [x] A8 / R4: Herdr-format OSC 9 fixtures preserve `title: body`; Ghostty smoke
  verifies actual notification display with terminal/OS notifications enabled.
  Report any Herdr backend-detection limitation separately from wire delivery.
- [x] A9 / R2, R7: An attached Android controller posts each supported form as a
  system notification, preserving Unicode and OSC 777 title/body. Verify both a
  foreground Activity and a backgrounded Activity with a still-live connection.
- [x] A10 / R3, R5, R6, R7: Repainting or recreating an Activity does not duplicate
  notifications. Pending Android events are bounded and retired on connection
  loss, detach, takeover, and session end; reconnect does not replay them.
- [x] A11 / R8: Android 13+ permission grant/denial and disabled app/channel
  notifications behave correctly. Skipped notifications remain skipped after
  permission changes; the terminal remains usable. Older supported Android
  versions use the channel without a notification runtime-permission prompt.

## Out of Scope

- OSC 99, capability queries, buttons, activation/close reports, and notification
  update/close protocols.
- Standalone BEL/visual-bell forwarding. BEL remains a valid OSC terminator.
- Detached notification storage, catch-up, retry, acknowledgement, or a history UI.
- Desktop-native OS notification backends and external push.
- Android foreground services, wake locks, battery exemptions, dedicated
  keepalive, push delivery, or receipt while disconnected/process-terminated.
- Broadcasting to other devices or observers, notification actions beyond
  opening the app, or automatically reconnecting/taking over from a notification.
- tmux DCS passthrough, arbitrary escape forwarding, notification protocol
  translation, per-application inference, or changing the hosted TERM identity.

## Execution Status

The user approved the final desktop-and-Android summary with “开始执行吧” on
2026-09-17. Task status is `in_progress`; implementation and validation follow
[design.md](design.md) and [implement.md](implement.md). No product scope question
remains. Validation evidence and platform limits are recorded during execution.

## Validation status

Implementation and all acceptance criteria are complete. See
[validation.md](validation.md) for automated evidence and the user's manual
Ghostty result: “试过了，两个都正常”. Android API 36 live and permission/channel
tests passed, together with API 32 ordinary-posting and disabled-channel tests.
The user authorized the proposed commit, PR creation and merge with
“提交然后pr、合并吧”. Task archival follows the feature commit.
