# Shared single-file upload service

## Goal

Provide one shared authenticated upload contract and private remote storage for
both desktop and Android. Source requirements: ../09-09-remote-file-upload/prd.md.

## Requirements

- Own R3, host/shared portions of R4/R5, R6 byte preservation, R8 and R9 progress.
- Accept one opaque file up to 50,000,000 bytes inclusive, enforced while reading.
- Stream bounded chunks on a service stream separate from terminal input/output.
- Return a unique absolute path only after full host completion; enforce originating
  authority, clean partial files and preserve completed files without retention/quota.
- Report host-accepted bytes, typed failure/cancel and final success for both UIs.
- Preserve terminal usability when an older host lacks the upload service.

## Acceptance Criteria

- Image, PDF and arbitrary binary payloads retain exact bytes on the target host.
- Empty file and exact cap succeed; cap+1 and mismatched totals do not publish paths.
- Partial interruption, stale authority, cancel/commit races and disk errors cannot
  produce a false completed result or overwrite another transfer.
- Progress and completion remain distinct; bounded transfer does not enter the PTY.

## Dependency and Scope

First implementation slice after parent planning approval. Desktop/Android children
consume this contract; parent owns cross-client acceptance. Own core/proto/client/
daemon/platform upload boundaries and their focused tests, without platform UI.

## Implementation Status

Approved and implemented inline on `feat/remote-file-upload`. Validation is
recorded in `research/validation.md`; final work-commit review remains pending.
Explicit platform/AI runtime gaps remain documented rather than inferred.
