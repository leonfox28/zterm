# Single-file upload design

## Status and contract authority

Final planning draft, 2026-09-09. Product decisions are accepted; implementation
waits for review of this artifact set. Parent PRD owns R1-R14 and AC1-AC11.
This document owns cross-child contracts/constants; child designs reference them.

## Architecture and ownership

| Layer | Responsibility |
| --- | --- |
| core | Typed upload metadata/progress/outcome, IDs and shared limits; no OS/IO |
| proto | New bounded upload frames in wire v2, structural validation |
| client | One streaming uploader, correlation, acknowledgements/cancel; no clipboard |
| daemon | Authenticated service dispatch, Session-authority admission, host files |
| platform | Effective-UID paths and safe temporary file operations |
| desktop CLI | Clipboard acquisition, prefix commands, pause gate, status composition |
| Android Kotlin | System pickers, bounded content URI staging, dialog/lifecycle |
| Android Rust | Retained operation/input gate, shared uploader and native progress API |

Keep terminal model/driver/snapshot/history unaware of file contents. The desktop
broker continues forwarding opaque inner bytes. No new relay, SSH command, public
HTTP endpoint or remote graphical environment is required.

Flow: explicit gesture -> bind original attachment and pause input -> prepare
one source -> open authenticated upload stream -> bounded chunks/host progress ->
host final publication -> same input owner pastes the path once -> release pause.

## Input and operation state

States: Idle, Preparing, Uploading, Finalizing, Inserting, Succeeded, Cancelled,
Failed. Failure may additionally indicate that upload outcome is unknown or that
a file was saved but insertion did not occur. Do not label these cases success.

- Start only from an input-authorized remote attachment. Capture resolved host,
  SessionId, target-issued AttachmentId, frontend attachment/input epoch and exact
  owner handle before any asynchronous picker/clipboard/IO step.
- Acquire one local upload slot and pause gate in that input owner. Do not use
  transport Reconnecting/Synchronizing or change input epochs to represent upload.
- Already admitted prior input preserves existing ordering. While paused, new
  child text/keys/paste/pointer input is discarded/rejected, never replayed. Decoder
  fragments from a paused paste cannot leak after unpause. Preserve preexisting
  unsent mobile composition; do not claim rejected IME input was committed.
- Terminal output, snapshot ACKs, resize, route observations and local controls
  continue. Existing prefix timeout/quote/repeat/release rules still apply.
- Prefix+v during a transfer reveals current status, without rereading clipboard.
  Prefix+c cancels only the active upload. Prefix+. cancels then detaches locally.
  Child Ctrl+C is not an upload command; it is blocked by the ordinary pause gate.
- At completion revalidate origin and normal input readiness before internal path
  insertion. The upload's own insertion bypasses only its local pause, never the
  attachment authority/synchronization checks. Insert with zero modifiers and the
  existing bracketed-paste-aware route. No newline or Enter is appended.
- If a healthy same-attachment resize temporarily gates insertion, use its existing
  ordered synchronization path; do not retry against a reconnect replacement.
  Resume ordinary input only after insertion is admitted, or failure/cancel ends.
- Actual disconnect, host/Session navigation, takeover, detach and Session end
  retire the operation and queued insertion. Android background alone is not
  detach; visibility collectors do not own the network operation.
- Pause prevents user-driven changes, not independent child exits/UI changes.
  No AI-process detection, prompt parsing or native-attachment guarantee is added.

## Source contract

Desktop reads an explicit OS file-list representation first. Exactly one item
must open as a readable regular file. Reject multiple items/directories and failed
reads locally. If a file representation exists but is invalid, do not fall back to
an unrelated image. When no file list exists, read image pixels and encode PNG.
Never interpret plain text or HTTP URLs as upload sources. Copies remain copies;
no source deletion, clipboard rewrite or cut/paste move semantics.

Use arboard 3.6.1 as the researched desktop backend candidate, pinned only after
its feature/dependency policy and supported OS builds pass. Its Get API documents
file_list and image; runtime fixtures must establish actual Finder/X11/Wayland
interop. Keep it in desktop-only dependencies, not Android/core/client. Clipboard
work and PNG encoding run away from terminal presentation. Bound worker concurrency
and source lifetime, handle cancellation without reusing a late source. The encoded
file limit does not by itself bound the decoded-pixel allocation; validate image
shape arithmetic and release temporary buffers promptly. Never silently downsample.

Android uses PickVisualMedia(ImageOnly) or OpenDocument with MIME */*, no multiple
selection and no directory tree. Kotlin uses ContentResolver, not URI path guessing.
Stage selected bytes into a private cache file using bounded reads. The shared/native
limit is exposed to Kotlin rather than duplicating 50 MB constants. Metadata is a
hint; count actual bytes, stop at limit+1 and remove failed staging. Unknown/cloud
sources show Preparing; staged size is authoritative for upload. Do not convert
existing image/doc formats. Discard staging when its attempt ends, including after
success; this cleanup does not affect the retained remote file.

Both platforms supply a prepared readable source, exact length and safe extension.
Desktop opens the selected file once; premature EOF or growth/mismatched count is
an explicit source-change error. No directory packaging or file-content inspection.
Retry is explicit; mobile can reuse still-readable selected source access or request
selection again. Never pretend a retry can resume after process death.

## Wire and transfer contract

Use one bidirectional service stream over existing zterm/2. Add FILE_UPLOAD_SERVICE
capability at bit 3 (verify still unused when implementing). Proposed wire registry:

| Kind | Direction | Fields/meaning |
| --- | --- | --- |
| 400 UploadBegin | client -> host | SessionId, AttachmentId, size, safe extension |
| 401 UploadReady | host -> client | fresh TransferId, accepted size |
| 402 UploadChunk | client -> host | TransferId, absolute offset, bytes |
| 403 UploadProgress | host -> client | TransferId, accepted byte count |
| 404 UploadFinish | client -> host | TransferId, exact final byte count |
| 405 UploadCompleted | host -> client | TransferId, exact size, generated absolute path |
| 406 UploadCancel | client -> host | TransferId |
| 407 UploadCancelled | host -> client | TransferId, no completed path |

Reuse ServiceErrorResponse for terminal transfer errors; distinguish upload-specific
codes in one domain owner. Registry numbers are centralized in proto and covered
by exact registry tests. All frames use one nonzero stream request_id; transfer
IDs and offsets must agree. No OperationId lease or automatic replay window is
introduced: a broken attempt never transparently retries a mutation.

| Limit | Value |
| --- | --- |
| File | 50,000,000 bytes inclusive; zero is valid |
| Chunk payload | 65,536 bytes maximum |
| Unacknowledged source window | 1 MiB maximum |
| Extension | ASCII alphanumeric, at most 16 bytes; invalid/missing becomes empty |
| Returned path | ASCII allowlisted generated path, at most 1,024 bytes |
| Transfer ID | 16 random bytes, host-issued |
| Frontend operations | One per attachment, no queue |
| ACK cadence | At least every 256 KiB or 250 ms after accepted data; final immediate |
| UI cadence | At most four periodic refreshes/second; state transitions immediate |
| Setup/final response budget | Existing bounded control-operation budget |
| Transfer inactivity | 30 seconds without useful transfer progress; no whole-file 5s cap |

These bound in-flight work, not retained disk storage. Existing host service-stream
admission and authorized Session/controller counts remain authoritative. Stream
fairness and bounded disk workers prevent a transfer from blocking the Session actor.

Host enforces sequential offsets, declared/actual size and chunk limits before
writing. Reader and writer run concurrently so progress/cancel cannot deadlock
behind data writes. ACK only data accepted by the file writer; ACK is not final
publication/fsync. Speed uses a rolling/smoothed delta of host-accepted bytes over
monotonic time. Display decimal MB and MB/s consistently. Withhold a success/100%
state until host finalization; zero-byte files move directly to Finalizing.

## Authorization and host file lifecycle

Transport provides authenticated remote identity and current authorization
generation. Session service must also verify requested Session/Attachment belongs
to that principal and is the current synchronized controller. IDs alone grant
nothing. Extend the Session actor with read-only upload admission/revalidation,
returning its controller generation and existing lifecycle watch, not a second
controller registry. Hold stream/attachment lifetime, monitor lifecycle and recheck
before final publication. A resumed/replaced attachment cannot inherit an upload.

Filesystem root: /tmp/zterm-<effective-uid>/, mode 0700. Descendants:
<session-id>/<transfer-id>/file[.<safe-extension>]. All identifiers are generated or
validated hexadecimal/decimal components. File mode 0600, no executable bits. Host
creates a new transfer directory and incomplete file; same-directory rename publishes
the completed file after exact-size write/sync. Never accept a client destination.

Use effective account UID rather than HOME. Resolve the platform's system /tmp
alias (macOS /private/tmp) before validating managed components; reject symlinks,
wrong ownership and unsafe permissions inside the managed namespace. Exclusive
creation and descriptor-relative operations prevent following precreated targets.
Only actor authority checks run in the Session actor; disk IO stays outside it.

On failed/cancelled incomplete transfer remove its own partial file/empty transfer
directory. Preserve committed files on detach, Session end and cancellation races.
Do not scan/delete completed files or implement a cumulative quota. A daemon crash
can leave an incomplete artifact; startup may remove only recognizably unfinished
artifacts owned by this UID, with no age-based sweep of successful paths.

If cancel/revoke/disconnect races final publication, a complete orphan can remain;
local origin fences still forbid stale insertion. If the completion reply is lost,
report outcome unknown and do not resend under a fresh transfer automatically.
An explicit retry can create a second unique file. Avoid claiming atomicity across
filesystem commit, network acknowledgement and PTY input.

## Capability and compatibility

Advertise file upload only on host compositions with the service. Retain remote
capabilities from the authenticated Hello/Welcome in the actual candidate/connection,
including inbound candidates used for authorized outbound service requests. Android
retains Welcome capabilities in NormalConnection. Never substitute a saved peer's
stale capabilities for those of a newly selected connection.

Desktop LocalSessionTunnelOpened gains an optional remote_capabilities field. It
is transport handshake metadata; broker still does not decode inner upload frames.
Absent field from an old local broker means unavailable. Before sending UploadBegin,
require the selected stream's host capability. Old host/broker reports upgrade
required without sending an unknown new kind or disrupting the terminal. No major
bump, second terminal dialect, probing mutation or automatic fallback transport.

## Presentation and lifecycle

Desktop extends StatusRenderer/composition to reserve a right-aligned upload area
using display-cell widths. Truncate left metadata, then omit speed/size/bar as needed
while retaining percentage. Render only through the existing presenter and preserve
cursor/child dimensions. Status changes use the existing paced/atomic render owner,
not a second stdout writer. Success/cancel notices expire after three seconds;
errors stay until the next user action. A one-row terminal cannot start an upload
because it has no progress row; queue a local resize-required notice to show when
space is available. An existing upload continues over a temporary one-row resize,
with status restored when height allows and prefix cancel still available.

Android's first two buttons directly open the photo and document pickers, using
a photo icon followed by a paperclip. There is no choice menu. Dialog opens once
a URI is selected, with preparing or transfer content, filename/size/progress/speed,
and Cancel. Back cancels; outside taps do not silently dismiss an active transfer.
On success insert then close and restore terminal focus without automatically
showing IME. Error offers Retry and Close. AppRepository/native operation owns state;
Compose observes it. Preserve operation/source identity across Activity recreation,
ignore stale picker/dialog callbacks and close only handles/staging it owns.

## Risks and deferred items

No retention promise beyond the host OS /tmp lifecycle; no nested-SSH/container path
mapping; no native AI attachment guarantee; no file-format comprehension guarantee.
Actual clipboard OS/outer-terminal behavior and real Android/cloud-provider picker
behavior require runtime evidence. Backend library selection may change if builds
or interop fail, while preserving this source/UX contract. Such a change does not
permit a hidden file-type whitelist or Ctrl+V interception.
