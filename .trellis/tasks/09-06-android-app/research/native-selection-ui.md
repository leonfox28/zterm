# Android selection UI evidence — 2026-09-06

User requested normal system selection behavior and removal of selected-line
counts/instructional UI. The compact prototype demonstrates handles and a
floating Copy menu with no terminal viewport shift.

- [Compose text interactions](https://developer.android.com/develop/ui/compose/text/user-interactions)
  documents SelectionContainer around selectable Text composables. This does
  not establish automatic selection of a custom terminal Canvas.
- [SelectionContainer reference](https://developer.android.google.cn/reference/kotlin/androidx/compose/foundation/text/selection/SelectionContainer.composable)
  warns of undefined selection behavior for uncomposed text in lazy layouts.
  A virtualized terminal's offscreen rows therefore cannot be assumed to work
  by wrapping its visible content alone.
- [ActionMode](https://developer.android.com/reference/android/view/ActionMode#TYPE_FLOATING)
  provides a floating contextual action menu. This is the candidate system
  surface for the terminal's Copy action, backed by its semantic selection.

Design inference: reuse Android action UI and native interaction conventions;
keep terminal row identity, hit testing, selected-row pinning, and cross-screen
range extraction in the existing planned selection owner. Native menu/handle/
magnifier integration must be verified in the emulator and on Xiaomi, not
claimed from the Penpot demonstration. No product code was added in this pass.
