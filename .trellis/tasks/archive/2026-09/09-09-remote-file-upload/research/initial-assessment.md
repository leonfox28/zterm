# Remote image paste: initial assessment

> Historical research: current decisions are in ../prd.md and file-upload-scope.md.
> The image-only scope, Ctrl+V proposal and remote clipboard alternative are superseded.

Date: 2026-09-09. Repository: main at e8cb05e. Planning evidence only;
no live clipboard content was read, no image was uploaded, and no AI prompt was
submitted during this investigation.

## Existing behavior and root cause

Desktop zterm is a CLI nested in an outer terminal. `HostInputEvent`
(`crates/cli/src/terminal_ui.rs:2507`) represents bytes, enhanced keys, framed
text paste, mouse and terminal replies. There is no OS image clipboard input
or file-upload wire service. Forwarding Ctrl+V to a remote PTY does not forward
the local operating system clipboard.

The locally installed Pi package is `@earendil-works/pi-coding-agent` 0.85.1.
Its implementation is available at:

`/Users/huyuanzhe/.nvm/versions/node/v24.18.0/lib/node_modules/@earendil-works/pi-coding-agent/dist/modes/interactive/interactive-mode.js:2334`

`handleClipboardPaste` reads the OS image clipboard, saves the bytes under
`os.tmpdir()` with a UUID filename, and calls `editor.insertTextAtCursor(filePath)`.
Its Ctrl+V handler is wired at line 2312. The clipboard reader is
`dist/utils/clipboard-image.js`, which contains native clipboard access and Linux
Wayland/X11 helpers. This directly supports the user's explanation for this Pi
version; it does not establish the installed version on the remote server.

Official Codex documentation confirms interactive image paste and `--image` file
inputs, including PNG/JPEG. It does not establish that arbitrary inserted path
text always becomes a native image attachment. Validate that behavior with the
actual remote Codex build before claiming native attachment equivalence:

- https://learn.chatgpt.com/docs/image-inputs?surface=cli
- https://learn.chatgpt.com/docs/codex/cli

The requested product contract is upload + path insertion. Keep native attachment
UI compatibility as validation evidence, not a new unrequested integration scope.
If the tool treats it as plain text, the user can explicitly ask it to inspect
the referenced image, subject to that tool's model/tools/filesystem permissions.

## Reusable boundaries

| Area | Evidence | Consequence |
| --- | --- | --- |
| Desktop command ownership | `crates/cli/src/terminal_ui/prefix.rs:9`, `:15`; `.trellis/spec/backend/terminal-input-commands.md` | Extend the existing typed input/command owner; never identify AI processes or reparse child-encoded bytes. |
| Desktop target routing | `crates/client/src/protocol.rs:66` | Bind uploads to the resolved device rather than re-resolving an alias at completion. |
| Shared remote connection | `crates/client/src/iroh_controller.rs:459` | Reuse authenticated Iroh service streams for a bounded upload channel. |
| Desktop broker | `crates/daemon/src/remote_tunnel.rs:31` | Existing tunnel forwards bounded opaque bytes; keep upload semantics out of the broker. |
| Wire extension | `proto/zterm/v2/wire.proto`, `crates/core/src/domain.rs:493` | Add a bounded transfer contract and deliberate capability handling; do not send an unknown kind on the terminal attachment. |
| Android picker | `apps/android/app/src/main/java/io/github/leonfox28/zterm/ScannerScreen.kt:81` | Existing `PickVisualMedia(ImageOnly)` usage can guide the new toolbar entry. QR downsampling is not suitable for arbitrary screenshots. |
| Android text paste | `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalView.kt:612` | Current paste only coerces clipboard content to text; picker must read image bytes from the returned content URI. |
| Android input admission | `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:327`, `:348`; `.trellis/spec/backend/shared-client.md` | Capture original host/Session/handle/navigation epoch/input epoch before suspension. Calling `repository.text` against whatever is current after upload would incorrectly admit stale input. |
| Paste encoding | `crates/android/src/terminal.rs:926` | Insert with zero sticky modifiers and existing child-mode-aware text paste semantics, with no Enter. |
| Managed paths | `.trellis/spec/backend/effective-user-state.md` | Apply UID ownership, private files/directories and symlink-safe creation; do not route arbitrary destination paths from the client. |

Android's Photo Picker returns a content URI, not a remotely meaningful filename.
Use ContentResolver and bounded IO. `PickVisualMedia` includes system/document
picker fallback; no general photo-library enumeration is needed.
Official reference: https://developer.android.com/reference/androidx/activity/result/contract/ActivityResultContracts.PickVisualMedia

## Recommended design direction (not yet approved)

1. Platform ingress owns image acquisition. Desktop reads the local OS clipboard
   only after an intentional gesture; Kotlin owns the system picker and content
   URI. Shared Rust owns transfer and host response validation.
2. Upload through a separate stream on the existing authenticated connection,
   in bounded chunks, so image bytes never become PTY input and do not queue
   ahead of terminal input/output on the same stream. Shared connection bandwidth
   still applies; streaming is not a guarantee of zero network contention.
3. Host chooses a unique, ASCII, whitespace-free absolute filename and writes an
   incomplete file. Publish the final path only after successful full completion.
   Use private permissions, bounded transfer buffers/concurrency, cancellation and
   partial-file cleanup. No cumulative stored-image quota (user decision below).
   No shell/base64 command injection, public upload server or SSH
   credentials are needed.
4. Completion returns to the original attachment's input owner, which rechecks
   its authority and performs one mode-aware path paste. Never queue completion
   across a real reconnect/takeover/Session change. A lost acknowledgement must
   not trigger blind upload-and-paste replay.
5. Show a small uploading/success/error state. Successful file upload and terminal
   path insertion are distinct outcomes; do not claim insertion merely because
   bytes reached disk. If the target changed, report that insertion was skipped.
6. Use host feature capability discovery (or another explicit supported-service
   check), returning an upgrade-required message for older hosts while preserving
   the terminal connection. Do not introduce a second terminal wire dialect.

## Product trade-offs to resolve

- **Desktop gesture:** remote-only image-aware Ctrl+V best matches the user's
  current habit. Only consume it when an image is found; local Sessions and normal
  framed text paste retain their behavior. However, Ctrl+V is also a meaningful
  key in generic terminal programs. A stale image in the clipboard makes the
  distinction impossible without app-specific detection. Options are an explicit
  setting/escape route for Ctrl+V ownership, or a dedicated prefix command such as
  Ctrl+] then v. The latter avoids ownership conflicts but adds a gesture.
- **Outer terminal:** zterm can only handle gestures the outer terminal sends.
  Cmd+V/Ctrl+Shift+V/menu paste may be consumed there and may generate text, a file
  path, or no bytes for an image. Do not promise universal interception. Never
  automatically upload arbitrary pasted path-looking text.
- **Temporary storage:** prefer private per-user/per-Session storage and generated
  filenames. Exact shared `/tmp/zterm/<uid>` requires a deliberate parent-directory
  ownership policy; simply creating `/tmp/zterm` as 0700 breaks other users.
  A per-user root such as `/tmp/zterm-<uid>` avoids that issue. Return the actual
  host path. macOS system `/tmp` alias handling must distinguish platform aliases
  from attacker-controlled managed symlinks.
- **Retention — user decision, 2026-09-09:** no zterm retention deadline or cumulative
  stored-image quota. Leave completed uploads in place, including after Session
  detach/end. The earlier seven-day/quota proposal is withdrawn. Host OS `/tmp`
  cleanup still applies; this is not a durable-storage guarantee. Cleanup of failed
  partial uploads and bounded in-flight buffers remain separate implementation
  concerns, not successful-file retention policies.
- **MVP media scope:** propose single-image picker, PNG/JPEG/WebP and a 20 MiB
  encoded-byte limit. Preserve screenshot quality; avoid default lossy compression.
  HEIC/other formats need explicit conversion or a clear unsupported-format result;
  do not silently rename bytes to `.png`. Numeric/format policy is pending.
- **Filesystem reachability:** path is on the zterm host. Nested SSH, an unmounted
  container `/tmp`, a different UID, or an AI sandbox may not see it. Automatic
  discovery or copying into those environments is a separate integration.

## Validation plan for later implementation

- Host file service: size boundaries, chunk failure, private permissions, unique
  names, cancellation, stale authority and old-host behavior.
- Desktop: legacy/enhanced Ctrl+V ownership, press/repeat/release, local unchanged,
  ordinary text paste unchanged and original-attachment completion fences.
- Android: real system picker and cancel flow, bounded URI reads, toolbar layout
  with IME, navigation/recreation/disconnect during selection/upload, modifiers.
- Real remote Codex/Pi: verify received image bytes and ability to inspect image
  content; separately record whether a native attachment chip appears.
- Existing focused Rust/protocol/input checks and Android lint/build/instrumentation
  appropriate to the eventual change. No tests ran in this planning-only turn.

Potential execution slices: shared upload/host storage; desktop image ingress;
Android picker/toolbar. Decide task-tree shape when the UX contract converges.
