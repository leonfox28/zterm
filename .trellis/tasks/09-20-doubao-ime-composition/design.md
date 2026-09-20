# Android IME composition diagnosis and proposed correction

## Boundary

[Research](./research/reproduction.md) reproduces Pinyin leakage with Doubao
26-key Pinyin in display-in-editor mode. Classification: local implementation
defect. `TerminalView.TerminalInputConnection` already owns preedit; its Editable
must be authoritative for characters, selection and composing range. IME queries,
selection notifications and cursor anchors must agree with that state. Rust and
the host should continue receiving only admitted terminal input.

## Approved correction

- Establish valid selection when creating/resetting the local buffer.
- Maintain composing spans and honor `newCursorPosition` during replacement,
  using BaseInputConnection editing primitives where compatible with the local
  terminal buffer.
- Keep before/after-cursor queries and deletion coherent; publish selection and
  composing changes at batch completion. Derive anchor state from that buffer.
- Preserve one-time admission, valid explicit finish, queue rejection, Unicode,
  modifiers, input-epoch fences and isolation of retired connections.

Expected changes: Android `TerminalView.kt`, focused input instrumentation near
`TerminalUiTest.kt`, and `.trellis/spec/frontend/android-app.md`. No transport,
protocol, host model or IME-brand detection is justified by this evidence.

## Acceptance and trade-offs

First establish a failing application-independent regression: after
`setComposingText("n", 1)`, selection is `(1,1)`, composing range is `[0,1)`,
before-cursor text is `n`, after-cursor text is empty, and the child receives no
bytes. Replacing with `ni` preserves those invariants; committing `你` sends it
once. Also validate actual Doubao input in both display modes, explicit finish,
deletion/Unicode, rejected admission, stale connections and healthy resize.

Suppressing all `finishComposingText` calls could lose legitimate finalization
while leaving invalid context. Candidate-bar mode is an interim workaround, not
the intended product fix. No migration is needed; rollback is an ordinary Android
code/APK rollback preserving package identity and user state.

## Status

The user approved this correction after reviewing the diagnosis and explicitly
requested a new branch on 2026-09-20. Work proceeds on `fix/android-ime-composition`.
