# Desktop and Android single-file upload with path insertion

## Goal

Allow desktop and Android users to upload one local file to a remote zterm host
and insert its absolute path into the terminal, without a separate transfer tool.
Support screenshots, images, PDF, documents and other file types equally.

## Background and Confirmed Facts

- User reports Ctrl+V works for images in local Codex/Pi under zterm, but not in
  remote Sessions. This has not been independently reproduced during planning.
- Desktop is a terminal CLI; its current host-input codec handles keys and text
  paste, with no clipboard-image reader or image upload service.
- Desktop already owns a bottom status row outside the child PTY. `StatusRenderer`
  (`crates/cli/src/terminal_ui.rs:1627`) builds the target/route/RTT text, rendered
  by `crates/cli/src/terminal_ui/composition.rs:264`. Existing local commands use
  Ctrl+] and `BINDINGS` (`crates/cli/src/terminal_ui/prefix.rs:15`).
- Android currently coerces clipboard items to text
  (`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:612`).
- Android bottom toolbar starts with Esc
  (`apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalScreen.kt:134`).
- Installed Pi 0.85.1 already implements image Ctrl+V as OS clipboard read,
  temporary image write, and literal path insertion; see
  [research evidence](research/initial-assessment.md). This validates the proposed
  mechanism for that version. Remote versions and Codex path behavior require
  real acceptance checks.

## Requirements

- R1: In a desktop remote attachment, Ctrl+] followed by plain `v` uploads a single
  clipboard file/image and inserts its absolute host path at the terminal cursor.
  Ctrl+V keeps its current behavior in all Sessions. Preserve local AI clipboard,
  ordinary text input and other existing Ctrl+] commands.
- R2: Put two direct buttons at the left of the Android bottom toolbar: first a
  photo icon opens the system image picker, second a paperclip opens the system
  file picker with no type filter. Both are single-selection and share upload/path insertion.
- R3: Use host temporary storage with per-user isolation and collision-free names.
  Resolve the original `/tmp/zterm` proposal to `/tmp/zterm-<effective-uid>/`, with
  Session/transfer-specific descendants as specified in design.md. zterm
  imposes no retention deadline or cumulative stored-file quota and does not
  automatically delete successful uploads. Host OS temporary-directory cleanup
  still applies. Failed/incomplete upload cleanup is a separate concern.
- R4: Insert only after successful complete upload; never send Enter automatically.
  Cancellation/failure must not insert a nonexistent path.
- R5: Delayed completion remains bound to its original host, Session and input
  authority, even after navigation, disconnect or controller takeover.
- R6: Accept any readable single file within the size limit, independent of MIME
  or extension. Preserve copied/selected file bytes. Do not identify AI processes,
  scrape screens, write remote clipboards or execute uploads. AI tools retain
  ownership of file interpretation and native attachment UI.
- R7: Desktop source handling: prefer explicit OS clipboard file-list
  data, requiring exactly one readable regular file; multiple files/directories
  produce a notice. Otherwise accept raw clipboard image data and generate PNG.
  Plain text that looks like a file path or URL is not an upload source.
- R8: First release per-file limit is 50 MB, interpreted precisely as 50,000,000
  bytes inclusive. Enforce actual bytes as well as metadata. For raw clipboard
  images, limit the generated PNG file. This is not a cumulative storage quota.
- R9: Desktop progress bar appears on the right of its existing bottom status row.
  Android opens a progress window after selection, showing progress, file size
  and upload speed. Distinguish preparing, uploading, host finalization, success,
  failure and cancellation. Locally buffered bytes do not prove upload completion.

- R10: Pause new child keyboard, text-paste and pointer input from upload initiation
  through preparation, transfer and path insertion. Continue output, synchronization,
  resize and local cancel/detach. Discard attempted paused input; never queue/replay
  Enter or typed text after completion. Keep existing unsent Android composition
  intact. Restore ordinary input on success, failure or cancellation.
- R11: One active upload per attachment. Repeated upload commands expose the current
  operation. Desktop Ctrl+] then `c` cancels an active transfer; Android has Cancel
  and Back cancellation. Ctrl+C is not an upload command and follows R10 while
  input is paused. Explicit local detach/navigation cancels and retires insertion.
  Retry is an explicit new attempt, never an automatic replay/resume.
- R12: Android success inserts the path, closes the progress window and returns
  focus to the same terminal. Failure provides explanation and retry/dismiss.
  Cancel removes incomplete artifacts; completed host files are retained.
- R13: Desktop displays a right-aligned bar/percentage, transferred/total size and
  smoothed speed when width permits (e.g. `[====----] 42% 21/50 MB 2.3 MB/s`). Narrow
  widths omit detail while keeping percentage. No additional terminal row is used.
  Android displays original filename, total size, percentage and smoothed speed.
- R14: Android files without a trustworthy known total show Preparing until bounded
  local staging yields the real size. Upload speed excludes preparation/provider
  download. Display names never control the host path or inject terminal controls.

## Acceptance Criteria

- AC1 (R1/R4): Desktop prefix+v creates a readable remote file and inserts its
  correct absolute path once, without submitting the current prompt.
- AC2 (R1): Local/remote Ctrl+V, remote text paste, enhanced press/repeat/release
  input and Ctrl+] detach/quote/unknown-command handling still work.
- AC3 (R2): Android's first two buttons open the correct single-select system
  pickers directly; cancellation leaves input unchanged; selection uploads and inserts
  into the originating terminal.
- AC4 (R3/R4): Incomplete uploads never expose a successful final path; simultaneous
  uploads cannot overwrite one another. Successful files are not deleted by a
  zterm age/quota policy or by Session detach/end.
- AC5 (R5): Session switch, disconnect and control loss never insert into a newer
  or unrelated attachment.
- AC6 (R6): Real remote Codex/Pi validation records whether path insertion creates
  native image attachment state or requires the agent to read the referenced file.
  Image, PDF and arbitrary binary fixtures retain exact uploaded bytes without
  promising every AI tool can interpret every format.
- AC7 (R7/R8): Empty/text-only/multi-file clipboard and directories show actionable
  notices. Exactly 50,000,000 bytes is allowed; 50,000,001 is rejected, including
  unknown/dishonest size metadata. Oversize failure leaves the terminal usable.
- AC8 (R9): Progress is monotonic per transfer, uses host-confirmed received bytes,
  and shows success only after host finalization. Size/speed units are consistent;
  unknown-size preparation has explicit indeterminate UI.
- AC9 (R9): Desktop progress stays in the right side of the status row across
  narrow/wide windows, Unicode host labels, resize and main/alternate screens.
  Android dialog remains usable across IME/insets and Activity recreation.

- AC10 (R10/R11): Paused input reaches no later prompt, including Enter, text paste
  and pointer actions. Output/local controls continue; success/cancel/error restores
  input. Repeated upload commands never duplicate a transfer. Existing paused IME
  composition remains available after return.
- AC11 (R11/R12): Cancellation, explicit retry and recreation use one operation owner;
  completed files are not deleted by a cancel/commit race, and a stale completion
  cannot dismiss a newer dialog or insert into another attachment.

## Out of Scope

- Automatic prompt submission, AI-process detection, and automatic uploads of
  ordinary text that merely resembles a local path.
- General file manager, remote browsing, download and file synchronization.
- Retention scheduler, age-based deletion and cumulative stored-file quotas.
- Remote OS clipboard writes, virtual displays, Ctrl+V interception for upload,
  directory recursion, multi-selection, type whitelist, image editing/compression,
  conversion of existing files, and automatic transfer retry/resume.

## Task Map

| Child | Responsibility | Dependency |
| --- | --- | --- |
| `09-09-file-upload-service` | Shared protocol, host files, size enforcement, progress | First executable slice after approval |
| `09-09-desktop-file-upload` | Clipboard sources, prefix command, status progress, path input | Validated shared upload service |
| `09-09-android-file-upload` | Image/file pickers, progress dialog, bridge, path input | Validated shared upload service |

Parent owns requirements and cross-client acceptance. Children own implementation;
this task split does not imply multi-agent delegation. Work remains inline.

## Compatibility and Practical Limits

- Remote host needs the upload capability; an older client-side broker/host yields
  a recoverable upgrade-required notice without disrupting the terminal.
- Desktop clipboard support targets the project's existing macOS/Linux platforms;
  its runtime backend still requires access to the local graphical clipboard.
- The returned file is on the zterm host. Nested SSH, unmounted container paths,
  different users and AI-tool sandboxes do not automatically gain access.
- Pausing input prevents user-driven draft changes; it cannot stop an application
  exiting or advancing its own UI. zterm does not promise attachment chips or
  recognition of every format by the AI tool.

## Implementation Status

The user approved final artifact review with “开始吧”. Shared service, desktop and
Android implementation are complete on `feat/remote-file-upload`; integration
evidence and remaining platform acceptance are in `research/validation.md`.
Work commits await the workflow Phase 3.4 confirmation. No release was published.
