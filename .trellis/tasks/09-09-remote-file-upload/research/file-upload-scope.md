# General single-file upload: current planning evidence

Date: 2026-09-09. Supersedes the image-only media/shortcut proposals in
`initial-assessment.md` and clipboard alternative in `remote-clipboard.md`.

## Repository fit

- `crates/cli/src/terminal_ui/prefix.rs:15` owns local bindings. Add Upload there;
  legacy/enhanced input already passes this owner before child encoding.
- `crates/cli/src/terminal_ui/session.rs:226` currently treats all LocalCommands
  as terminal completion because only Detach exists. Separate continuing upload/
  cancel commands from Detach without changing the codec or child key encoder.
- `crates/cli/src/terminal_ui.rs:1627` owns StatusRenderer; `composed_text` emits
  target/route/RTT. `composition.rs:264` builds its full-width styled row. Reserve
  right-side display-cell width for transfer state, preserving sole presenter,
  child dimensions and transactional frame output.
- `composition.rs:21` already reserves the last row on main/alternate screens
  when physical height > 1. No additional resize on upload start. A one-row
  terminal has no status row; design an explicit tiny-terminal behavior without
  overwriting child content.
- Android `ScannerScreen.kt:81` supplies the existing photo-picker pattern.
  Add a separate single OpenDocument contract with `*/*`, not directory selection.
  Both return content URIs; Kotlin owns their reads.
- `AppRepository.kt:327` admits text with the current epoch; completion must first
  validate the original upload epoch/handle. `crates/android/src/terminal.rs:926`
  already handles bracketed text paste.

## External API evidence

- arboard currently documents `Get::file_list()` and `Get::image()`:
  https://docs.rs/arboard/latest/arboard/struct.Get.html
  Candidate only; verify released version and macOS/X11/Wayland runtime behavior
  before pinning it. Its image getter returns decoded pixels; encoded PNG size
  does not itself bound decoded-image memory.
- macOS has explicit pasteboard file URL objects separate from text:
  https://developer.apple.com/documentation/appkit/nspasteboard/readingoptionkey
  Distinguish copied Finder files from ordinary path-looking text.
- Android Storage Access Framework provides user-selected provider file access:
  https://developer.android.com/training/data-storage/shared/documents-files
- `OpenableColumns.SIZE` may be null:
  https://developer.android.com/reference/android/provider/OpenableColumns
  Bound actual reads; stage unknown-total streams locally while showing Preparing,
  then upload the known size. Never turn unknown total into zero/false completion.

## Shared design direction

Use authenticated service streams separate from terminal input/output. Shared
client owns upload lifecycle/correlation/progress; platform adapters own source
reads. Host chooses private unique destinations and enforces inclusive 50,000,000
bytes. Keep existing wire/control frame caps unchanged; bounded chunks and
backpressure avoid one giant protobuf payload.

Progress reports host-accepted bytes rather than local socket writes. Coalesce
ACKs/UI updates to a few per second, but publish completion/error immediately.
Use smoothed bytes/second. Byte transfer and final host publication are distinct;
insert only after final success. Existing five-second unary deadlines are not
a complete-file timeout: define separate transfer inactivity/cancellation rules.

No retention scheduler/cumulative quota. Clean only unfinished staging. Same-name
files receive independent destinations. Cancellation losing a race to host commit
must not delete a completed file or claim the completed effect was rolled back.

Treat file contents as opaque; keep copied/selected originals unchanged. Only raw
image clipboard data needs PNG generation. Do not convert PDFs, compress photos,
or reject HEIC because an AI model might not support it. Display original names
separately from safe generated paths, preserving useful normal extensions.

## Resolved UX decision (2026-09-09)

The user accepted pausing child input during preparation/upload, with output and
local controls active. Do not buffer/replay attempted keys or Enter. Desktop cancel
uses Ctrl+] then c; Android uses Cancel/Back. Ctrl+C is not repurposed as upload
cancellation and is subject to the pause gate. Operation state outlives Compose
instances; navigation/reconnect retires insertion even when host commit wins a race.
See parent design.md for the final state/authority contract.

## Validation direction

- Shared/host: zero/cap/cap+1, inaccurate or unknown total, interruption, cancel/
  commit ordering, disk errors, old-host behavior, authorization and unique paths.
- Desktop: one/multiple files, directories, text-only, raw image, unchanged local
  behavior, enhanced/legacy key events, progress alignment/width and cursor safety.
- Android: real single-selection picker routes, cancel/oversize/unknown-total,
  dialog speed/size/progress, URI errors, recreation/navigation and bridge build.
- Byte equality for image/PDF/arbitrary binary fixtures; separate actual remote
  Codex/Pi consumption evidence, without claiming universal AI format support.

No clipboard content reads, uploads or product tests performed in this planning
turn. Parent/child task split supplies sequential ownership, not agent dispatch.
