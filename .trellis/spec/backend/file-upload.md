# Single-file Upload Contract

## 1. Scope / Trigger

Read before changing remote upload, clipboard acquisition, Android pickers,
temporary publication or terminal input during upload. Desktop and Android use
one `zterm-client::upload` owner; only the daemon can publish host files. The
feature never detects an AI application's identity or accesses a remote clipboard.

## 2. Signatures

```text
Ctrl+] v                    start one desktop clipboard upload
Ctrl+] c                    cancel the current desktop upload
UploadConnector::open_upload(ResolvedSessionTarget) -> UploadConnection
TerminalViewCommandWriter::upload_origin() -> UploadOrigin
TerminalViewCommandWriter::write_upload_input(UploadOrigin, Vec<u8>)
NativeTerminal::reserve_upload(input_epoch) -> Arc<NativeUpload>
NativeUpload::{start(path, size, extension), cancel(), wait_for_progress(generation)}
```

`UploadOrigin` freezes resolved host, Session and attachment. Android's
Application-owned `TerminalUploads` reserves before launching one
`PickVisualMedia(ImageOnly)` or `OpenDocument(*/*)` result contract. Retain the
same operation through Activity recreation; a cancelled reservation future must
cancel its native reservation with a drop guard. Android uses separate photo and
paperclip buttons in that order. System picker Back retires the reservation
without detaching the terminal or relaunching selection.

## 3. Contracts

- Inclusive cap: **50,000,000 bytes**, including zero. Any regular file type;
  preserve bytes and up to 16 ASCII alphanumeric extension characters. Desktop
  explicit clipboard file lists take priority: exactly one regular file, else
  fail. Only a raw clipboard image is encoded as PNG; text paths/URLs are not
  file sources. Keep arboard in the CLI dependency graph only.
- Wire kinds 400–407: Begin, Ready, Chunk, Progress, Finish, Completed, Cancel,
  Cancelled. Begin binds Session/attachment/size/extension; Ready allocates a
  16-byte transfer ID. Subsequent frames retain the request ID, zero deadline
  field and exact transfer ID. Chunks are sequential and at most 64 KiB; client
  keeps at most 1 MiB unacknowledged. ACKs count completed host writes, every
  256 KiB or 250 ms. Begin/finish deadlines are 5 s; inactivity is 30 s.
- Advertise `Capabilities::FILE_UPLOAD_SERVICE` (bit 3) only on hosting daemons.
  The actual selected broker candidate retains remote Hello/Welcome capabilities.
  `LocalSessionTunnelOpened.remote_capabilities` is optional field 2; absent
  metadata or a missing bit prevents all UploadBegin/data writes. A separate
  stream reuses normal authentication, stream admission and opaque IPC envelopes.
- Storage is `/tmp/zterm-<effective uid>/<sessionhex>/<transferhex>/file[.ext]`.
  Validate ownership, private modes, non-symlink managed directories, create-new
  staging and transfer collision. Directories are 0700, files 0600. Canonicalize
  the system `/tmp` alias, not `$TMPDIR`. Count exact bytes, sync, rename, sync
  directory before Completed. Drop removes only incomplete attempts; successful
  uploads have no age or cumulative-cap cleanup. Never reuse a transfer directory.
- Upload admission checks the current authenticated remote controller without
  creating/dropping a SessionAttachment. Recheck authorization/controller generation
  at publication, then lifecycle before returning a pasteable result. Disk work
  stays off the Session actor. Debug/errors do not expose file contents or paths.
- Frontends pause and discard new child input through preparation, transfer and
  insertion. Keep output, resize, local copy/navigation and explicit cancel alive.
  Android uses separate odd/even upload input generations plus Repository queue
  versions, preserving NativeFrame.input_epoch and unsubmitted IME composition.
  Desktop drains old input at completion, preserving owned command-key releases
  while retiring the pending suffix/UTF-8 tail. Do not replay paused input.
- Validate original authority immediately before inserting a safe absolute path.
  `paste_bytes` honors bracketed-paste mode, adds no Enter/modifiers. A stale local
  upload-input command fails **without terminating the replacement view driver**.
  No automatic retry or path replay, including lost publication responses.
- UI progress is latest-only, at most 4 Hz for byte updates, with immediate
  transitions. Desktop reserves right-side status cells using the sole presenter;
  narrow rows keep percentage and do not change child dimensions. No status row
  means refuse start. Android displays filename/size/speed/cancel, prevents outside
  dismissal, treats Back as cancel, and offers explicit Retry/Close after failure.
  Source staging streams in 64 KiB blocks into private cache, enforcing both
  metadata and actual-byte caps. Bound cancelled-but-blocked source workers.

## 4. Validation & Error Matrix

| Condition | Result |
| --- | --- |
| Source larger than cap | `upload_too_large`; no publication |
| Multiple files, folder, text, unreadable/changing source | `upload_source_invalid`; no fallback to another clipboard representation |
| Missing host/broker capability | `service_not_implemented`; ordinary terminal remains usable |
| Wrong ID/order/offset/count/correlation | malformed-frame error; remove own staging |
| Wrong/retired controller or attachment | `lease_lost`; no insertion |
| Write/storage failure | `upload_storage_failed` |
| Publication cannot be confirmed or finish response lost | `upload_outcome_unknown`; file may remain, no replay |
| Explicit cancel/navigation/disconnect | retire operation, discard paused input; completed file may remain |

## 5. Good / Base / Bad Cases

- Base: copied PDF is streamed unchanged, then its safe path appears at the cursor.
- Good: exactly 50 MB succeeds, zero bytes succeeds, an old host receives zero
  upload-service bytes, Activity recreation observes the same transfer.
- Bad: interpreting clipboard text as a path, sending an Enter, importing arboard
  into the shared client, resetting the Android IME epoch just to pause input.

## 6. Tests Required

Core/proto validate caps, extension/path safety, DTO limits and redaction. Platform
tests assert modes, collision/symlink refusal, exact publication and own-staging
cleanup. `session_wire::tests::upload_tests` runs zero/50 MB and cancellation through
a 4 KiB duplex, malformed sequence/correlation and attachment admission. Shared
client tests assert no bytes to old hosts and no retry on unknown publication;
view tests keep a replacement driver alive after stale upload input. CLI tests
exercise clipboard priority/PNG pixels and legacy/enhanced prefix ownership.
Native tests cover abandoned reservations, source slot ownership and late progress.
`UploadUiTest` checks progress, size/speed, cancel/retry/close. `NativeUploadTest`
uses `uploadFixture=1` and a private fresh `upload-ticket.txt` from a disposable
`upload-` host to assert SHA-256, input pause, preserved epoch and no automatic Enter.
Record OS clipboard, picker, phone and AI-app runtime evidence separately from builds.

## 7. Wrong vs Correct

Wrong: reuse a Session attachment handle as upload admission, then detach it on
upload completion. Correct: a read-only admission token observes the original lease.

Wrong: put progress sleep inside a repeatedly rebuilt select loop, or start a
subscriber after the current generation. Correct: persistent interval plus latest
state observation; continuous terminal output and already-finished operations
must not starve completion.
