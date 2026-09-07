# Android UI design workspace

Status: planning. Product implementation has not started.

## Tool decision (2026-09-06)

The user switched the UI design tool from Figma to Penpot and explicitly
requested project-scoped MCP configuration. `.codex/config.toml` contains only
this project's registration. The existing hosted Penpot MCP credential is
stored in an ignored, mode-0600 local file; no global registration was changed.
Setup instructions are in `.codex/PENPOT.md`.

## Design document

- Document: **Zterm Android · UI v1**.
- [Open in Penpot](https://design.penpot.app/#/workspace?team-id=c828d3cf-7d4e-8145-8008-98f2df5d9705&file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=c828d3cf-7d4e-8145-8008-98f4fa037d49).
- File ID: `c828d3cf-7d4e-8145-8008-98f4fa037d48`.
- Page ID: `c828d3cf-7d4e-8145-8008-98f4fa037d49`.
- Page name: **01 · 移动端首版**.
- This is the user's initially empty Penpot draft, renamed for this work.
- First editable, clickable prototype created on 2026-09-06 after the user
  reloaded the agent and requested a prototype.
- [Start the prototype](https://design.penpot.app/#/view?file-id=c828d3cf-7d4e-8145-8008-98f4fa037d48&page-id=c828d3cf-7d4e-8145-8008-98f4fa037d49&section=interactions&frame-id=89af369c-c4a2-806f-8008-98fcc53e4a71).

## Current revision v8 — complete first-release states (2026-09-06–07)

The user accepted the post-v7 completeness review. PRD CORE-1/4, SCROLL-3,
and UI-9/10 own the resulting behavior. Scripts 13–20 record the incremental
prototype changes; 19 is a repeatable read-only audit, rerun after 20.

- Settings adds terminal font size with 12/14/16 choices (initial default 12)
  and immediate preview. All nine language/theme settings states open matching
  Chinese/English and dark/light font dialogs. Twelve dialog variants mark
  exactly one choice each. The original language/theme groups remain independent.
- A landscape terminal at 844 × 390 illustrates increased columns, thirteen
  dense terminal rows, and all eight bottom keys. Its full-width title panel
  overlays the grid and ends above the keys; selecting the current row dismisses.
- Long-press saved-device removal has a concise confirmation and a result that
  removes MacStudio plus its recent card, retaining build-server. Normal home
  card/resume taps still go directly to the previous terminal. A named review
  flow opens the confirmation after a short delay to stand in for long press.
- First connection without a previous ID has three review flows: no Sessions
  offers New; one Session targets it; multiple automatically expand the title
  list in an empty terminal container. The first multi-Session list has no
  selected row. The zero-Session create branch enters scratch and displays only
  scratch in its title list. Pairing success and the no-recent home example
  use the multi-Session branch. A stale previous ID still follows the existing
  ended state, rather than silently choosing another Session.
- Added short reconnect/failure/retry, revoked authorization, camera-denied,
  image-without-QR, and invalid/expired credential illustrations. Camera denial
  retains Album and Ticket. Credential errors retain the field and add one
  error line; correction clears it, then Connect shows a disabled pending state
  before the simulated result. No dialog title or Paste button returns.
  Reconnecting retains the previous terminal frame and disabled shortcut row.
  Fresh connection and empty/ended states have no fabricated output.

Native readback records 90 boards: 39 screen states, 50 overlays, and one
editor-only OS note, all on the same Penpot page. There are 354 interactions
with no invalid destinations, 15 named review flows, no text-boundary overflow,
and nineteen terminal states each with exactly one eight-key row clear of the
grid. The normal home targets, removed-device result, and twelve font selections
were also checked from saved objects. Browser review verified font-size preview
in Chinese/dark and English/light contexts,
landscape expansion/dismissal, first multi-Session selection, empty → create →
sole scratch list, removal of card/recent reference, and expired credential →
correction → pending → pairing success. It also verified that camera denial
retains a working credential entry, and retry goes through retained-output
reconnecting before returning to the same terminal illustration.

These are static state illustrations. Font selection is demonstrated within
the popup; reopening starts at the default illustration, and App-wide persisted
preferences are not simulated. Landscape secondary branches reuse portrait
result/form illustrations; native Android must retain the real orientation.
Long press, image selection/cancel, camera permission, typing, network recovery,
clipboard, and first-input preservation require native runtime evidence.
The OS picker remains an external handoff note; no custom picker/cancel screen
was added. Secondary Session mutation forms retain the v4 limitations. Delays
only simulate completion and are not network timing promises.

The configured MCP client transport was already closed; editing/readback used
the same official MCP server through the Python MCP SDK and existing ignored
credential, without configuration changes. One font batch completed after a
client timeout and was read back before further work. A removed-shape proxy
stopped the initial landscape polish; its partial state was inspected before
continuation. Preview reload after the editor saved confirmed the current
objects. No product code or Android runtime tests ran; task remains planning.
The consolidated scope, acceptance, deferrals, and execution prerequisite are
recorded in [final-planning-review.md](final-planning-review.md).

## Revision v7 — minimal credential dialog (superseded by v8)

The user limited the credential dialog to a text field, Connect, and top-right
close. `12-minimal-credential-dialog.js` removes the title and dedicated Paste
action from both empty/filled overlays. The centered dialog is now 342 × 272;
Connect is 294 × 48 at x=24, matching the field width and side margins.
Normal text-field paste remains an Android requirement; it needs no App button.

Browser review verified the empty and filled layouts, field-click simulation,
and closing back to the same scanner. Readback confirms matching 24 px margins,
no title/Paste controls, preserved empty-input disabling and filled-submit link,
197 interactions, and zero text-boundary overflow. Board count remains 55.
PRD, design, implementation plan, and manifest are synchronized. Planning only.

## Revision v6 — credential dialog and preferences (superseded by v7)

The user requested manual credentials in a dialog, plus language and theme
settings defaulting independently to Follow system. Construction is recorded
in `11-ticket-dialog-settings.js`.

- The scanner's lower-right entry opens a centered 342 × 300 credential modal.
  The existing empty/filled board IDs are retained as overlays, removed from
  standalone screen navigation. The dialog contains the credential field,
  Paste, Connect, and close. Empty Connect is disabled. Click input or Paste
  simulates a filled value; explicit close/open overlay actions keep the same
  scanner beneath it. Connect continues the existing pairing walkthrough.
- Settings directly presents two three-option single-choice groups: language
  Follow system / Chinese / English and theme Follow system / Dark / Light.
  Both default to Follow system. Nine combined states preserve the other
  preference when one changes; there is no Save action. English and light
  states include translated labels and contrasting light surfaces/system icons.
- In this static illustration, system values resolve to Chinese and dark.
  Settings state changes are demonstrated within that page; leaving Settings
  returns to the existing Chinese/dark home illustration. Actual persistent
  preferences, OS-follow updates, App-wide localization/theme, native editable
  dialog/IME, and connection retention are Android acceptance in UI-7/8 and
  PAIR-3, not functionality implemented by the Penpot preview.

Native readback: 55 boards (25 screen states, 29 overlays, one editor-only OS
note), 199 saved interactions, three flows, no invalid destinations, and zero
text-boundary overflow. All 54 preference targets across nine states preserve
independence and have exactly two selected radio controls per state. Browser
review verified default system/system, English with system theme, English with
light theme, and changing to Chinese while retaining light. It also verified
scanner → empty credential dialog (disabled Connect) → simulated Paste → filled
dialog, with exactly one overlay and the scanner's frame unchanged; Close
returns to the scanner and filled Connect reaches pairing success.

The batch exceeded the MCP client's 180-second wait, but finished in the editor.
Do not replay it: live readback and preview reload verified the saved objects.
Readback resumed through the same official server using the existing local
credential and Python MCP SDK, without changing project/global configuration.
The task remains planning; no product code or Android runtime tests were run.

## Revision v5 — persistent keys and one viewport (superseded by v6)

The user clarified that the terminal should naturally scroll as one surface,
with the shortcut row present during scrolling. Album selection should use the
OS directly. `10-persistent-shortcuts.js` applies these changes.

- All 12 terminal states now have exactly one eight-key row. With IME hidden,
  it sits at y=764, below a grid ending at y=754 and above the system gesture
  area. With IME shown, the existing row stays at y=478 above the keyboard.
  Scroll/selection keeps the same bar and content origin. Not-ready/ended
  states show disabled controls. Newer prompt rows remain visible at the bottom.
- Renamed illustrated boards to Terminal / Bottom and Terminal / Scrolled to
  make them positions in one route. They remain click simulations in Penpot;
  actual Android scrolling must be continuous, with bottom following and a
  stable reading anchor away from the bottom. UI-6 records that contract.
- Removed the custom album page from the App preview and erased its gallery
  drawing. Its existing board now holds an editor-only OS handoff note.
  The scanner's Album click simulates a successful system-picker image result
  and continues pairing. It does not invoke a real picker or imply pairing
  happens before image selection; cancellation remains an Android requirement.
  System layout is owned by the phone OS and is not a Zterm gallery design.

Evidence and the distinction between mutable terminal cells and retained output
are in [terminal-scrolling-model.md](terminal-scrolling-model.md). Internal data
sources and synchronization do not require current/history pages or UI modes.

Checks: exported terminal and scrolled states have matching controls and clear
grid boundaries. Native readback reports 47 boards (19 screens, 27 overlays,
one editor-only system note), 153 saved interactions with valid destinations,
no links to a custom album screen, zero text overflow, and exactly eight keys
on one row in each terminal state. No terminal text extends into its key row.
The task remains planning; no Android runtime validation is claimed.

## Revision v4 — direct gestures and per-row management (superseded by v5)

The user removed the remaining terminal-level tools menu and wants direct
scrolling, long-press selection, and Copy. Every Session row's overflow should
contain only Rename and Delete. `09-scoped-session-menus.js` records the edit.

- Removed the terminal overflow icon/hit targets on all terminal states and
  removed the old tools overlay. There is no Select text, History, or Disconnect
  action in the terminal or Session menus. Back to hosts retains its existing
  detach semantics; OS background retention is unchanged.
- All 18 rows across the six illustrated picker states have a separate 48 dp
  overflow target. Tapping a row switches; tapping its overflow opens a sheet
  headed by that Session's name and does not switch or request takeover.
- Each row menu contains exactly Rename and Delete. Deletion confirmation
  names that Session and states that its processes end. The existing current
  android-app rename/delete walkthrough remains connected to its results.
- Additional Session menus open their correctly scoped rename initial forms
  and delete confirmations and support dismissal/cancellation. These secondary
  forms/confirmations are visual endpoints: typing and confirmed mutation
  results are not wired. No global mutable Session store is simulated. Unchanged
  rename initial forms correctly show disabled Save.

The product touch policy now reserves scrolling/long press for App browsing
and selection even with program mouse reporting enabled; program control uses
the keyboard. This replaces the former mouse-mode routing/menu fallback in
SCROLL-2. Only retained history can be browsed; alternate-screen content not
retained by the host cannot be reconstructed. This is an implementation
contract, not a claim about native gestures running inside Penpot.

Validation: exported and inspected the picker, then checked browser preview
for the clean terminal header, all three row menus, a non-current dev-server
menu without switching android-app, and the correctly named deletion dialog.
Native readback finds 47 boards (20 screens, 27 overlays), 156 saved interactions,
valid targets, zero text-bound overflow, equal row/menu counts in all six
pickers, no terminal overflow hit targets, and none of the three removed labels.
Task remains planning; PRD/design/checklist are updated for UI-5 and SCROLL-2.

## Revision v3 — sessions expand from the title (superseded by v4)

The user removed the standalone Session page and the terminal's top-right
keyboard button, and proposed an app bar that expands downward when its title
is tapped. `08-title-session-picker.js` applies this review to the existing
document. The title gains a small chevron and a full-width touch target.

- Tapping the title opens a compact top overlay with the same title position.
  The terminal grid remains at x=8/y=91; expansion does not push it downward.
  Tapping the title, outside the panel, or the current row closes the overlay.
- Session rows mark the current selection and occupancy. Selecting dev-server
  changes the terminal and title; opening it again marks dev-server. Occupied
  deploy retains explicit takeover. New Session stays at the panel's bottom.
- Original Session and renamed-list boards are now overlay-only components,
  removed from the standalone preview sequence. Renaming returns directly to
  the renamed terminal. Closing leaves a terminal shell with a title picker
  containing the remaining two Sessions, without choosing one automatically.
- All terminal states lose the top keyboard toggle. The main walkthrough uses
  a bottom terminal input tap to show the IME and a system-area down chevron to
  simulate hiding it. Android will use its actual system Back behavior.
- Session actions still demonstrate selection/history, rename, detach, and
  confirmed close. Management mutations are illustrated on android-app;
  secondary terminals demonstrate title switching and current-state highlights,
  with their detailed menu/typing interactions not wired. The prototype still
  does not maintain a global Session store across illustrated mutation branches.

The product contract additionally requires Android Back to dismiss the panel
first, bounded internal list scrolling, and no attachment/scroll/selection or
PTY-resize changes merely from expanding/dismissing it. Penpot's 180 ms top-slide
is a motion sketch; actual anchored expansion, native Back/IME behavior, and
long-list gestures remain implementation checks, not runtime evidence.

v3 verification: inspected exported terminal and expanded panel; browser
preview confirms device-to-terminal, title expansion, outside dismissal without
triggering the underlying terminal, switching to dev-server and back, its
current-row highlight, input-tap/IME dismissal, and rename returning directly
to android-work with its updated title panel. Native readback reports 32
screen/overlay boards (20 full screens, 12 overlays), 127 valid saved click
interactions, no navigation to picker-only boards, no old keyboard-button
targets, zero text outside board bounds, and the same three flows.

Task remains planning; PRD/design/implementation checklist now reflect UI-4.

## Revision v2 — compact UI (superseded by v3)

Applied the user's first visual review directly to the existing Penpot boards,
preserving their IDs and incoming prototype links. `07-compact-ui.js` records
the revision; the original numbered files remain a construction history.

- Home: replace ANDROID with a settings icon. Remove heading and slogan.
  A compact recent-connection card is the first content below the app bar.
  Keep the saved hosts and Add host action. A separate no-recent state omits
  the card and moves the list up.
- Device and resume-card taps now go directly into the terminal. Management
  remains in the terminal menu. MacStudio's demo targets android-app; this
  does not decide the unresolved multi-session/no-previous target policy.
- Added a settings destination with only About / development-version content.
  This establishes navigation, not new configurable features.
- Terminal: compact app bar, grid at x=8/y=91, 12 px monospace with 17 px rows.
  No persistent connected/history/live labels, footer, distance counter, or
  scroll instructions. Shortcuts reserve a row only with the IME visible.
  Scroll and selection keep the same content origin and usable viewport.
- Selection: Android-style handles and a floating Copy action only. Remove
  line counts, offscreen-start prose, dedicated selection toolbar, and app
  copy-success toast. Existing cross-screen continuity requirements remain.
- Scanner: only Scan, Album, and Ticket labels. Camera preview occupies the
  body. Album stays at lower left and Ticket at lower right.
- Shortened ticket, pairing success, and management copy throughout. Retain
  terse destructive-action consequences and actionable error messages.

The prototype still simulates gestures with clicks: click terminal text to
show scrolled output; click again for selection, then a handle to extend.
In history, click the lower content area to simulate reaching the bottom.
Actual Android scrolling is continuous, not a screen-to-screen navigation.
Native selection integration evidence is in [native-selection-ui.md](native-selection-ui.md).

v2 checks: exported and inspected home, live terminal, selection, keyboard,
and scanner; all 28 screen/overlay text bounds fit inside their boards. Saved
navigation/overlay actions have valid destinations. Browser preview confirms
Settings and return, and tapping the saved MacStudio device opens the terminal
without the Session list. No product implementation or real runtime test ran.
The revised terminal menu also opens correctly and its Select text action
reaches the floating-Copy state without changing the terminal content origin.
The current design has 102 saved click interactions, 28 screen/overlay boards,
and the same three flows.

## Initial prototype v1 (superseded by v2)

One page contains 20 phone screens/states at 390 × 844, six overlay boards,
and an editor-only walkthrough board. The 91 native click interactions comprise
67 navigation actions, 18 overlay openings, and six overlay closings. Three named
flows start at hosts, scanner, and terminal. Text, shapes, icons, and 13 local
library colors remain editable in Penpot.

The draft uses a dark green-black background, pale green primary actions,
clear host/session cards, Noto Sans SC Chinese text, Inter Tight Latin UI text,
and JetBrains Mono terminal output. 390 × 844 is a layout reference, not a
claim about the physical pixels or final insets of Xiaomi 17 Pro Max.

Included walkthroughs:

- Hosts → resume terminal or choose a host's sessions.
- Add host → QR scan, **album at lower left**, or **ticket at lower right**
  → pairing success → sessions. Empty ticket confirmation is disabled;
  tapping Paste switches to the filled state.
- Session list → new session, switch session, occupied-session takeover;
  android-app menu → rename, disconnect, or confirm termination.
- Terminal → keyboard expanded/hidden; eight fixed shortcuts: Esc, Tab,
  Ctrl, Alt, and four arrows.
- History → select text → extend the same range beyond the viewport →
  copy feedback → return to live output. Start and end handles and an
  offscreen-start indicator make continuous selection explicit.
- Simulated connection interruption and retry; separate unavailable-host dialog.

This is a design prototype. Clicking substitutes for camera recognition,
typing, long press, dragging, and scrolling between illustrated states. It
does not access a camera, album, clipboard, terminal, network session, or host.
Names, paths, QR artwork, tickets, and terminal/test output are synthetic.
New/switch/rename/takeover destination terminals demonstrate the resulting
identity and provide Back navigation; their detailed actions are only wired
on the main android-app walkthrough. Prototype navigation does not maintain
a global session store. Background retention, caching/prefetch performance,
and real gesture behavior must be validated in the Android implementation.

## Prototype verification (2026-09-06)

- Exported and visually checked hosts, scanner, keyboard, and selection boards;
  also inspected the actual browser preview for layout and overlays.
- Clicked through resume → history → selection → cross-screen extension →
  copy feedback → live; keyboard expansion and dismissal both work.
- Clicked each of the QR/album/ticket pairing paths to success, then sessions.
  Empty ticket Confirm stayed on the current state.
- Clicked session actions from both terminal and list; rename → updated list;
  new session → scratch terminal; termination Cancel returned to the menu,
  and Confirm led to the two-session result.
- Read back all 91 saved interactions: no missing navigation/overlay targets.
- Read back all 26 screens/overlays and all three named flows. Text containment
  check reports zero text elements outside their board boundaries.
- Corrected new-session and deploy terminal subtitles to match their paths.

The task remains **planning**, awaiting visual/interaction review. The PRD
continues to own product requirements. No Android implementation has started.
Construction scripts and stable board IDs are in [penpot-prototype](penpot-prototype/README.md).

## Setup acceptance

- Codex discovers the enabled project `penpot` server.
- MCP initialization negotiates protocol `2025-11-25` and preserves server
  instructions.
- Tool discovery returns `execute_code`, `high_level_overview`,
  `penpot_api_info`, and `export_shape`.
- The browser's target file reports MCP connected.
- MCP successfully reads the actual page and its initially empty shape tree.
- MCP renames the page and creates a temporary styled rectangle; a subsequent
  call reads its stored geometry/style and removes the exact test shape.
- The original setup conversation used the configured transport directly;
  after the user's reload, native Penpot MCP tools became available and were
  used to construct and inspect this prototype.
