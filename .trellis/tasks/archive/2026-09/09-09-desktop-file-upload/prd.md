# Desktop clipboard upload and status progress

## Goal

Upload one clipboard file or raw image through Ctrl+] then v and insert its remote
path, with progress in the right side of the existing bottom status row.

## Requirements

- Own parent R1/R7 and desktop portions of R4/R5/R8/R9.
- Preserve Ctrl+V and existing prefix commands; use the existing typed command owner.
- Ingress: one explicit clipboard file first; otherwise raw image to PNG.
  Reject multiple files, directories and plain text with recoverable status notices.
- Use the shared 50 MB contract and show progress within the sole composed status
  row, preserving child dimensions, cursor and Unicode display-cell alignment.
- Recheck original target/attachment/input authority before path insertion, no Enter.
- Pause/discard new child input through preparation/transfer/insertion; output and
  local prefix cancel/detach remain active. Ctrl+] c cancels; no deferred key replay.

## Acceptance Criteria

- Image/PDF/binary copied-file fixtures and a raw screenshot upload and insert once.
- Text-only/multiple/directory/oversize/unavailable-clipboard inputs remain local errors.
- Legacy/enhanced prefix, key repeats/releases, local behavior and Ctrl+V regressions pass.
- Progress stays at the right across narrow windows/resize/alternate screen without
  corrupting child pixels; stale completion cannot reach another attachment.

## Dependency and Scope

Requires the approved and validated 09-09-file-upload-service contract. Own desktop
CLI input/composition, local clipboard acquisition and desktop integration tests.
Do not activate independently before shared-service readiness. Parent owns real
cross-client Codex/Pi acceptance; work runs inline, not via dispatched agents.

## Implementation Status

Approved and implemented inline on `feat/remote-file-upload`. Validation is
recorded in `research/validation.md`; final work-commit review remains pending.
Explicit platform/AI runtime gaps remain documented rather than inferred.
