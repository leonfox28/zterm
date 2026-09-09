# Shared service checkpoint — 2026-09-09

Implemented core upload contracts (decimal 50 MB, 64 KiB chunks, 1 MiB window),
typed wire 400–407, private effective-UID staging, host admission using the existing
Session actor/lifecycle and authorization commit boundary, and the shared bounded
full-duplex uploader. No automatic replay or retention policy was introduced.

Broker candidates retain capabilities from both Hello and Welcome. The local
tunnel forwards optional capability metadata while remaining opaque to inner
frames; Android's Iroh controller retains Welcome capabilities too. Desktop and
Iroh adapters expose independent reader/writer halves through UploadConnector.
The desktop tunnel decoder was extracted and reused, preserving its existing
framing, sideband and close semantics.

Public frontend API: zterm_client::upload::{UploadConnector, upload, UploadOrigin,
preparing}; core::upload owns metadata/progress/result constants and types.
LocalRuntime::upload_connector() binds desktop to its existing socket.
TerminalViewCommandWriter::upload_origin() captures the exact live resolved host
and attachment at the driver. write_upload_input(origin, bytes) rejects stale
origin locally without terminating the current view; frontends still own input
epoch/pause fences and mode-aware encoding.

Validation completed before desktop work:
- shared library suite: client 78, core 67, daemon 176 (+1 existing ignored),
  platform 23, proto 19 tests passed.
- added driver stale-paste rejection test passed separately after driver integration.
- clippy core/proto/platform/client/daemon, all targets/features, -D warnings passed.
- source-policy and terminal-dependency-policy passed.
- host duplex fixture tested 0, 2,000,013 and 50,000,000 exact bytes through 4 KiB
  buffers, original attachment survival, source length mismatch, invalid attachment,
  cancellation and detach. Protocol tests cover malformed IDs/chunks/path text;
  client tests cover old-host zero writes and lost final response without replay.

Pending parent integration evidence: real OS clipboard and Android picker/runtime,
selected inbound broker capability end-to-end, broader hostile stream state matrix,
final workspace checks/spec updates. No claim of actual remote Codex/Pi acceptance
has been made. Product work is on feat/remote-file-upload, uncommitted.
