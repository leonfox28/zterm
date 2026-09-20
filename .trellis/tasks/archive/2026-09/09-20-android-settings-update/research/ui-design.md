# Penpot GUI review — 2026-09-20

Status: GUI revision 2 approved by the user on 2026-09-20: `好 就这样吧`.
Application implementation has not begun.
The follow-up connection-failure styling request is included in the same task.

## Editable source and preview

- File: Zterm Android · UI v1.
- Page: 02 · 设置更新与连接提示.
- [Edit the page](https://design.penpot.app/#/workspace?team-id=c828d3cf-7d4e-8145-8008-98f2df5d9705&file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc).
- [Update available: tap Check for updates](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc&section=interactions&frame-id=bd580feb-afb2-80e5-8008-aabbed749c5e).
- [Already latest: tap Check for updates](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc&section=interactions&frame-id=bd580feb-afb2-80e5-8008-aabbf295a065).
- [Connection failure: dark](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc&section=interactions&frame-id=bd580feb-afb2-80e5-8008-aabca1343895).
- [Connection failure: light](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc&section=interactions&frame-id=bd580feb-afb2-80e5-8008-aabca1d85977).
- [Light settings](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=bd580feb-afb2-80e5-8008-aabb538508fc&section=interactions&frame-id=bd580feb-afb2-80e5-8008-aabbef4cc1e0).

## Proposal

Settings depicts the end of the scrollable content. The language group remains
above it. Existing theme, font slider/preview, and notification controls establish
context. The About card contains two 72 px rows: GitHub with repository subtitle,
and Check for updates with installed version. The current duplicate standalone
version row is consolidated into the update subtitle.

No update: Android's standard text Toast says 已是最新版. The neutral illustration
in Penpot does not specify its real dimensions, theme, position, or duration;
these are system-owned. The prototype delay only simulates dismissal.
Available update: a centered 342 × 324 confirmation, Later and
Download and install buttons. Download illustrations show a progress bar and
Cancel; no real download or installation is triggered.

The current connection-failure Surface in TerminalScreen.kt:93 wraps its content.
The proposed status card is 342 × 284 at a 390 px viewport, with 24 px page margins
and inner padding, 28 px corners, a 22 px title, 14 px body, and 46 px buttons.
It shares visual tokens with the update dialog but remains a terminal status
card, preserving the existing Retry/Sessions semantics. Retrying retains identical
dimensions and position. Use responsive width constraints in implementation,
not a hardcoded 342 dp width for all devices.

## Review 2 — native feedback and consistent settings

The user required a native system Toast, a simpler update icon, and no trailing
update chevron. The user explicitly chose an application-level notification
switch over a link that only opens Android notification settings.

- Use Toast.makeText with LENGTH_SHORT; no custom Compose surface or Snackbar.
- Check for updates uses the same simple download/tray icon as its confirmation.
  Remove the trailing chevron; retain the pending spinner while checking.
- Replace the loose notification paragraph and text links with a 358 × 74 rounded
  preference row: terminal notification title, concise state, and a 52 × 32 switch.
- Off stops future app notification posts and persists. On checks the OS gate;
  request permission if possible or guide to Settings if blocked. Denial stays
  visibly off; resume refreshes authorization. This does not directly toggle the
  system permission. Keep disabled-event dropping and terminal continuity.
- Existing posted notifications are not cleared by turning the switch off.
- Added dark/light off states and a system-blocked → settings-guidance example.
  Existing idle links now demonstrate On → Off → On. The new off/permission
  branches are independent examples; their update actions and OS Settings handoff
  are not simulated. OS permission prompts remain native, not branded dialogs.

Official evidence checked on 2026-09-20:
[system text Toast](https://developer.android.com/guide/topics/ui/notifiers/toasts),
[notification permission](https://developer.android.com/develop/ui/compose/notifications/notification-permission),
[user-controlled notification channels](https://developer.android.com/develop/ui/compose/notifications/channels).

## Evidence and limitations

Startup policy was approved on 2026-09-20 with `按你说的来`: cold-launch check
once after the initial UI is usable, quiet no-update/failure, and reuse of the
existing update modal for an eligible newer version. Later suppresses that
version's automatic reminder for 24 hours across launches; manual checks bypass
this and keep explicit feedback. The terminal notification switch does not
control update dialogs. See [the decision record](startup-update-check.md).
The startup flow is an editor annotation: the prototype does not simulate
launch events, persisted clocks, or real release discovery.
The saved annotation is `bd580feb-afb2-80e5-8008-aac8465ca867`, named
`Review v3 / Startup update policy`, placed beside the screen grid. Readback
verified its text bounds and no board overlap; its PNG export was visually
reviewed successfully. See [revision-3 audit](penpot/audit-v3.json).

- 18 screen states and 8 named review flows; saved IDs in
  [manifest.json](penpot/manifest.json).
- [Readback audit](penpot/audit.json): no text overflow or missing interaction
  destinations after first-revision layout polish. The
  [revision-2 audit](penpot/audit-v2.json) confirms all 14 notification rows fit,
  no update chevrons remain, the legacy custom toast is gone, and notification
  cards remain beneath modal scrims.
- Browser preview verified saved settings and update confirmation rendering,
  the checking transition, connection-failure card, and retrying transition.
  The separate viewer was reloaded to pick up saved page revisions.
- Second-review browser verification confirmed the new settings row/icon and
  tapping Terminal notifications changes the illustration to Off with 已关闭.
- Native Penpot export returned an export-server error/empty SVG for the new
  page. Visual review therefore used the real browser viewer, not a fabricated
  screenshot or a code-only rendering.
- Versions 0.1.35 and progress 42% are design samples; no release lookup occurred.
- The light entry jumps directly to its modal; the complete pending transitions
  are demonstrated in the dark flows. Spinners are static illustrations.
- Existing language/theme/font controls and the Sessions button are
  visual context; this task's prototype does not implement those pre-existing
  behaviors. The failure example uses an empty initial terminal and a simulated
  retry that returns to failure; it does not claim live network behavior.
- No product code, Android tests, release signing, or installation was performed.

## Construction record

Scripts 01–08 in penpot/ were run via the connected Penpot MCP in order. They
are a construction record, not an idempotent synchronization command. Do not
replay creation scripts on the populated page. The original mobile-design page
was preserved. Future changes should use the saved IDs and incremental edits.
The browser suspended the background editor during the revision-2 audit. Its
mutation portion was read back after waking the editor: notification rows sit
below modal scrims and GitHub links were preserved. Do not blindly replay the
creation scripts after a timeout. The full first-revision audit is retained;
revision-2 verification records the changed-control checks separately.

## Planning handoff

The GUI is approved. Release discovery, version comparison, signed APK
validation/download, platform installation, notification persistence, and
lifecycle behavior are captured in [design.md](../design.md). Ordered work and
validation are captured in [implement.md](../implement.md). The task remains in
planning until the implementation review; do not replay Penpot creation scripts.
