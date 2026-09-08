# Android terminal keyboard visibility after resume

## Goal

Keep the terminal soft keyboard hidden after the user dismisses it and returns
to zterm from the background, without disrupting terminal input or the Session.

## Background and confirmed facts

The user reports this sequence on Android: enter Terminal, show the keyboard,
dismiss it, background zterm, and return. The keyboard unexpectedly appears.

The Android contract makes keyboard opening an explicit bottom-button action.
`TerminalView.setKeyboardVisible` hides the IME without removing editor focus
(`TerminalView.kt:545`), and the View always identifies as a text editor
(`TerminalView.kt:552`). The Activity specifies only `adjustResize`
(`AndroidManifest.xml:18`). The background/recreation test checks the Session
and input, but not keyboard visibility (`TerminalUiTest.kt:300`).

Runtime reproduction on the unchanged v0.1.29 build confirms a local Android
window-policy defect: after both button dismissal and Android Back, task return
produces a server-origin `SHOW_AUTO_EDITOR_FORWARD_NAV` request. See
[investigation](research/keyboard-resume.md) for the causal trace and evidence.

## Requirements

- **R1 — Dismissal survives resume:** a hidden terminal IME stays hidden when
  zterm returns from the background. Apply equally to dismissal with the bottom
  keyboard button and Android Back, including repeated app switches.
- **R2 — Explicit input remains usable:** the bottom button can reopen the IME
  after resume, and keyboard input still reaches the selected terminal. Content
  taps continue to leave the IME hidden.
- **R3 — Preserve neighboring behavior:** retain the same Session, input-epoch
  fences, hardware-keyboard focus, healthy composition and settled viewport
  resizing. Preserve the existing behavior when backgrounding with the keyboard
  still visible; do not substitute unconditional hiding on every resume.
- **R4 — Limit policy to its owner:** respect ordinary editable dialogs and
  other screens. Keep focus/IME behavior in the Android presentation layer,
  using actual IME visibility rather than keyboard height as the signal.

## Acceptance criteria

- [x] **R1:** open → bottom-button hide → background → foreground leaves the
  IME hidden after window focus and layout settle, for two successive cycles.
- [x] **R1:** the same sequence with Android Back leaves the IME hidden and
  retains the Terminal route.
- [x] **R1/R2:** a fresh Terminal with no prior keyboard opening, and one focused
  by a content tap, remain keyboard-hidden across app switching.
- [x] **R2/R3:** after each dismissal path, explicit reopening accepts text,
  keyboard dismissal restores the settled grid, and the Session is unchanged.
- [x] **R3/R4:** visible-before-background behavior, composition/geometry checks,
  terminal content taps, hardware-key input, and an editable dialog retain
  their behavior.
- [x] A focused instrumentation regression demonstrates the old failure and
  passes with the correction; Android build/lint checks pass. Record emulator
  evidence separately from physical-device verification.

## Scope and constraints

This is a bounded Android lifecycle bug fix with focused instrumentation and
an Android spec update. No Rust/protocol changes, reconnection redesign,
keyboard-layout redesign, release publication, or installation on the user's
phone are included.

The user approved the terminal-scoped stateUnchanged proposal on 2026-09-08
("好 按你说的改吧"). Planning is PRD-only with a supporting research note.
Implementation and runtime comparison are authorized within this boundary.

Final implementation and acceptance evidence: [verification.md](verification.md).
All runtime checks use the isolated emulator; physical-device limits are recorded there.
