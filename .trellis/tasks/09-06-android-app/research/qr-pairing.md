# QR Pairing Research

Researched on 2026-09-06 for the user-approved camera scan screen, lower-left
album selection, and lower-right direct ticket entry. This records a technical
direction for planning; no Android product code has been written.

## Recognition and Screen Ownership

Use an App-owned scanner screen so its bottom actions retain the exact required
positions. The proposed Android integration is CameraX preview/analysis plus
the bundled ML Kit barcode API, restricted to QR codes. Both camera frames and
selected images feed one recognizer; a decoded raw value enters the existing
ticket parser. ML Kit documents custom UI support, QR-only filtering, file/image
input, and releasing CameraX image proxies after asynchronous recognition.
Use latest-frame analysis rather than queueing every frame.

Source: [ML Kit barcode scanning for Android](https://developers.google.com/ml-kit/vision/barcode-scanning/android).

Prefer the bundled model: it is available with the installed App and does not
need an initial model download. The installation guide distinguishes this from
the unbundled path managed by Google Play Services. The size trade-off is
acceptable for this proposed first-release scanner. Offline QR decoding and
networked host pairing are separate operations; offline decoding does not
establish that a host can be reached. Actual phone compatibility remains a
device test, regardless of the model's packaging.

Source: [ML Kit model installation paths](https://developers.google.com/ml-kit/tips/installation-paths).

## Album Selection

The lower-left action uses the AndroidX `PickVisualMedia` single-image contract.
The Activity library falls back to `ACTION_OPEN_DOCUMENT` if the system photo
picker is unavailable. Decode the returned image URI; cancellation returns to
the scan screen. Use the system selector rather than building an album browser.

Source: [Android photo picker](https://developer.android.com/training/data-storage/shared/photo-picker).

## Shared Pairing Contract

The three entries converge before ticket validation. Pause camera processing
while choosing an image, entering a ticket, or executing a pairing attempt.
Ignore stale recognition callbacks after the user changes entry or leaves the
screen. Only one pairing operation may own the flow at a time.

The canonical QR payload remains the existing encoded text ticket. The roadmap
requires QR to carry the same encoding
(`.trellis/tasks/08-20-cross-platform-relay-terminal-mvp/prd.md:134`), and
`docs/remote-cli.md` currently documents only
text output from `zterm pair create`. Add host-side QR presentation without
changing the existing machine-readable text output contract. The precise CLI
presentation option and image export mechanism belong in the technical design.

Tickets and selected image contents must not enter diagnostic logs. Preserve
the current ticket expiry, one-time consumption, TLS identity binding, and
directional host authorization rules in
`.trellis/spec/backend/transport-auth.md:227-268`.

## Validation Still Required

- Scan a fresh host QR through the camera and decode a saved QR image.
- Exercise manual tickets, image cancellation, permission denial, invalid or
  expired tickets, and repeated recognition callbacks.
- Exercise the emulator camera with an actual QR fixture; direct decoder tests
  and album selection alone do not establish camera-flow acceptance.
- Repeat physical camera recognition on the Xiaomi 17 Pro Max.
- Check generated ticket size against the selected QR encoder's capacity and
  verify realistic terminal-display scan density before finalizing presentation.
- Pin scanner/camera/Activity versions with the App's eventual build toolchain.
