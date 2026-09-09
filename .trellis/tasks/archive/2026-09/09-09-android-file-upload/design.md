# Android pickers and progress dialog design

## Dependency and ownership

Requires verified shared service child. Kotlin owns picker/URI/UI lifecycle, Rust
owns transfer and native input authority. Parent design is canonical for limits,
wire/status semantics and input pause; do not duplicate uploader logic in Kotlin.

## Entry and source preparation

Add two leftmost icons (photo, then paperclip) using the existing equal-width
48dp-high toolbar button style and accessibility labels. Keep the keyboard toggle rightmost and all existing
shortcuts visible. The buttons directly launch PickVisualMedia(ImageOnly) or
single OpenDocument with */*, without a menu. No broad storage permission or directory picker.
Use 18 dp LineIcon vectors for both upload entries, all four direction arrows and
the keyboard toggle, with one shared stroke style. Direction arrows use localized
accessibility descriptions; Esc/Tab/Ctrl/Alt remain 13 sp monospace text.

Before launching selection bind the current repository navigation epoch and native
terminal handle; reserve one upload intent/pause through the native input owner.
A stale picker result is ignored. Cancellation releases the reservation. After a
URI is returned open the progress dialog in Preparing and stage selected bytes
with ContentResolver on IO, enforcing the native-exported 50,000,000-byte bound.
Keep unknown totals explicit; no URI-to-filesystem guessing or whole-file FFI copy.
Original filename is local display metadata, sanitized for controls and bounded for
UI, not host destination. Normal file types are transferred without conversion.

## Native bridge and operation lifetime

Extend NativeTerminal with an upload reservation/start surface which captures its
current input epoch, resolved host, exact attachment and shared connection adapter.
NativeUpload is a retained cancellable object/operation with monotonic generation
and wait-for-progress API, following existing frame-observer conventions. It exposes
Preparing/Uploading/Finalizing/Inserting/final result, total and accepted byte counts.
Feed a private prepared cache-file source to the native reader; do not pass 50 MB
as one UniFFI Vec or run file reads on Main. Internal names follow established APIs
when implementing; the operation semantics are fixed here.

Rust actor owns the input pause and final path injection. Kotlin immediately gates
new input for visual feedback, but native actor validation remains authoritative.
Old IME callbacks/queued inputs cannot be relabeled by fresh frames. Keep transport
state/input epoch separate from upload UI state and preserve unsent composition.
Export only metadata; do not duplicate semantic frame rows on each progress update.

AppRepository retains operation, URI/staging owner and observation job. Compose
collects state and never starts a second upload because it recomposed/recreated.
Close native handles/source files deterministically. Background visibility alone
keeps the current operation; navigation/home/detach, Session change and real input
authority loss cancel/retire it. Process death does not resume automatically.

## Dialog behavior

Active dialog: filename, total size, progress/percentage and smoothed speed, plus
Cancel. Preparing uses indeterminate progress until size is known. Finalizing shows
that host completion is pending instead of false success. Back cancels; outside tap
does not dismiss an active upload. Keep callbacks bound to the displayed operation.

On successful upload and same-authority path insertion, close the dialog and restore
terminal focus without forcing IME to show. On error, release pause and show a typed
explanation with Retry/Close. Retry is explicit and rechecks current authority/source
access, acquiring a new attempt; no silent upload replay. Cancel cleans incomplete
staging but preserves any completed remote file. Saved-but-not-inserted/unknown
outcomes are explained without claiming rollback or prompting automatic submission.

## Checks and compatibility

Generate Kotlin from the pinned UniFFI build; no manual generated source edits.
Kotlin image/file actions respect current locale/theme and the terminal insets.
Use the existing app-wide runtime, without daemon/PTY/desktop dependencies. Verify
narrow devices with eleven toolbar controls and a full IME; no new toolbar row.

Runtime tests cover actual photo/document picker single-selection, URI cancellation,
unknown/cloud size, cap boundary, progress and byte equality, Activity recreation,
background/return, navigation/takeover, preserved composition and stale callbacks.
Native tests prove gate/origin/insert-once rules; build success alone is insufficient.
