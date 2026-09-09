# Shared upload service design

## Ownership and dependencies

Implement parent design.md Wire/transfer, authorization/storage and capability
sections. Their limits, message numbers and byte accounting are canonical; do not
redeclare alternatives here. This child has no UI dependency. Desktop/Android
children consume the tested uploader and host result contract.

## Affected boundaries

- core: add upload domain values, limits, typed terminal outcomes and content-redacted
  Debug. Keep OS/prost/async dependencies outside core.
- proto: upload.proto, build inclusion, kinds 400-407, capability bit 3 and structural
  conversions. Add remote_capabilities to LocalSessionTunnelOpened as optional
  handshake metadata; retain current framing caps and unknown protobuf fields.
- client: upload module exposing one async bounded transfer over a service-stream
  adapter, progress/cancel handle and prepared source. A failed stream is terminal
  for that attempt; do not use SessionUnaryClient's mutation retry path.
- daemon client facade/Unix transport: expose the selected target's upload stream
  through the opaque tunnel and return negotiated remote capability metadata.
- IrohController: retain current Welcome capabilities and open the same authenticated
  connection's bounded service stream. Android never gains an inbound service.
- connection_broker: retain Hello/Welcome capability values per candidate; expose
  them from the chosen AuthenticatedBiStream. Do not decode upload content there.
- daemon Session wire dispatch: route UploadBegin on its own service stream before
  ordinary unary processing. Reject transfer kinds on terminal attachment streams.
- Session service/actor: one read-only admission method checks exact remote principal,
  auth generation, Session, Attachment and current controller generation, returning
  a lifecycle subscription. Revalidate finalization through that same authority.
- platform/daemon upload store: private effective-UID directory, exclusive transfer
  creation, bounded asynchronous/off-actor file writes, final sync/publication and
  incomplete cleanup. Reuse existing safe path helpers before adding new ones.

## Proposed API shapes (implementation names may follow local patterns)

UploadMetadata { size, extension }; UploadBinding { target, session, attachment };
UploadProgress { phase, accepted_bytes, total_bytes }; UploadResult { path, size }.
One transfer owner consumes a readable source and async IO stream plus cancellation.
Metadata/progress/errors contain no image/file payload. UI original filename remains
local and is not a host destination. Native/desktop code receives no arbitrary
remote-command execution helper.

Host UploadAdmission contains immutable controller generation and a subscription
to the existing attachment lifecycle. It is not a new lease or retained authorization
registry. A resumed attachment needs a new deliberate upload. Read-only admission
must not own a SessionAttachment drop path that detaches the terminal accidentally.

## Protocol state and failure

Begin -> Ready -> (Chunk/Progress)* -> Finish -> Completed. Cancel is valid after
Ready until completion; aborting the stream before Ready cancels setup. Server
accepts only exact in-order offsets/IDs/request_id and declared size; no sparse
writes, arbitrary seek, compression or type conversion. Zero-byte Finish is valid.
Ordinary upload admission/file errors end only the transfer attempt. Existing
connection-level malformed-framing and authentication rules retain their owners.

Run progress reader concurrently with bounded writer. Host progress only follows
successful writes; publication requires exact size and final checks. Use existing
control budgets for begin/finish and a separate 30s useful-progress idle budget for
transfer. Global frame/control limits remain fixed regardless of file size.

On uncertain completion, return an explicit unknown outcome. No replay/resume or
completed-file deletion is added. Successful file naming is server-generated and
client-validated as safe absolute text before any future PTY insertion.

## Test ownership

This child proves file bytes, cap boundaries, protocol framing/correlation, storage
modes/symlinks/collisions, authority and cancellation/commit outcomes. It also proves
capability propagation and no unknown frame on old hosts, including an inbound
selected candidate and absent optional local metadata. It does not claim OS picker,
clipboard interop or actual AI image recognition.

## Migration/rollback

Additive optional service bit and message fields; no terminal dialect migration,
database schema or persisted replay table. Clients detect missing capability before
UploadBegin. Reverting service publication preserves terminal support and all
completed files. Failed/stale uploads never become successful path results.
