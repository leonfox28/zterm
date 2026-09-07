# Penpot Android prototype

Planning artifact for `.trellis/tasks/09-06-android-app`; not application code.
The live Penpot document is the editable design source. Links, walkthroughs,
validation, and prototype limitations are recorded in [ui-design.md](../ui-design.md).

`manifest.json` records the saved page, board IDs, dimensions, visibility,
named review flows, links, shortcut rows, and the final text-boundary check.

## Construction record

The numbered JavaScript files were executed through Penpot's `execute_code`
tool, in order, while connected to the target document:

1. `01-foundation.js`: palette, typography, shape/icon helpers, primary boards.
2. `02-pairing-sessions.js`: album, ticket, success, and session details.
3. `03-terminal.js`: live terminal, keyboard, history, selection, and recovery.
4. `04-session-overlays.js`: management dialogs and resulting session states.
5. `05-links-and-review.js`: click interactions, flows, guide, and layout fixes.
6. `06-final-polish.js`: path labels, default-flow cleanup, and readback audit.
7. `07-compact-ui.js`: user-reviewed compact home/terminal/scanner, settings
   entry, conditional resume card, direct device navigation, and native-style
   floating selection UI. Rebuilds children while preserving original boards.
8. `08-title-session-picker.js`: replace standalone Session screens with top
   overlays anchored to the terminal title; remove the app-bar keyboard toggle.
   Keep switching/current-state variants, explicit creation, and management.
9. `09-scoped-session-menus.js`: remove terminal tools menu, give every Session
   row a correctly scoped Rename/Delete menu and confirmation/form, preserve
   the main mutation walkthrough. Secondary form/result limitations are in
   the current revision of `ui-design.md`.
10. `10-persistent-shortcuts.js`: keep the eight-key row with IME hidden and
    during scrolling/selection; rename viewport states; replace the custom
    album page with an editor-only system-picker handoff note.
11. `11-ticket-dialog-settings.js`: replace credential pages with modal states;
    add independent system/Chinese/English and system/dark/light single-choice
    settings, including nine combined states and English/light previews.
12. `12-minimal-credential-dialog.js`: remove the credential title and Paste
    action; align a wider centered Connect with the field in both modal states.
13. `13-font-settings.js`: 12/14/16 font previews in Chinese/English and
    dark/light contexts, reachable from all nine settings states.
14. `14-landscape-device-removal.js`: landscape terminal, local-device removal
    confirmation/result, and empty saved-host state.
15. `15-first-session-entry.js`: zero/one/multiple first-connection branches
    using the terminal container and its title list.
16. `16-recovery-scanner-states.js`: reconnect/retry, revoked authorization,
    camera denial with fallback entries, and no-QR image feedback.
17. `17-credential-error-states.js`: pending and invalid/expired states of the
    existing minimal credential dialog, with correction/retry illustrations.
18. `18-v8-polish.js`: full-width landscape title expansion and editor guide.
19. `19-v8-audit.js`: repeatable read-only audit; run again after preview fixes
    to generate the final v8 manifest.
20. `20-v8-preview-fixes.js`: pending-label width, neutral first-entry rows,
    and title chevron alignment found during browser review.

Files 01–05 create objects and share temporary helpers in `storage.zd`. They
are a construction record, not an idempotent synchronization command. **Do not
rerun them on the populated design** or erase the user's work to replay them.
After a plugin/session reconnect, temporary storage may be gone; use the saved
IDs or `penpotUtils` to find the existing objects for incremental edits. File 06
is safe to repeat on the recorded page. Rebuilding in another empty document
requires deliberately adapting the page guard first.

Files 07–18 and 20 require the original helpers in `storage.zd` and have one-run guards.
They are also construction records, not synchronization commands. Use current
objects/IDs for subsequent user edits; do not blindly replay them. File 13
finished after a client timeout and was verified by readback. File 18 initially
stopped after removing a shape; readback located the partial state before the
corrected continuation. Avoid accessing removed proxies in later conditions.

The prototype contains only generated demo content. The visible terminal test
results are design samples and are not evidence that product tests ran or that
any Android feature has been implemented.
