# Android single-file pickers and upload progress dialog

## Goal

Add two leftmost terminal buttons (photo, paperclip) opening single system pickers,
then show file size, progress and speed while uploading and inserting a host path.

## Requirements

- Own parent R2 and Android portions of R4/R5/R6/R8/R9.
- Reuse the system photo-picker pattern; add a single general document picker (*/*).
- Read selected URI bytes unchanged with actual 50 MB enforcement; raw unknown
  totals require a visible preparation stage, not invented size/progress values.
- Retain operation state across Compose recreation, and capture host/Session/input
  authority before picker/IO suspension. Progress dialog includes Cancel and Back cancellation.
- Pause child input through preparation/upload and preserve unsent IME composition.
- Insert only a completed remote path with no sticky modifier or Enter; failures
  and cancellation preserve input and do not affect completed host files.

## Acceptance Criteria

- First photo and second paperclip buttons directly launch the correct real system
  pickers, each single selection. Android Back cancels selection without relaunch.
- Image/PDF/arbitrary data fixtures preserve bytes; cancel/oversize/provider-read
  failure shows appropriate state without injecting a path.
- Window displays progress/size/speed, explicit preparation and accurate completion,
  remains usable with IME/insets and does not restart uploads on Activity recreation.
- Navigation/reconnect/takeover retire stale insertion authority.

## Dependency and Scope

Requires approved and validated 09-09-file-upload-service. Own Kotlin picker/dialog/
repository work, crates/android bridge integration and Android checks. Generated
bindings come only from the existing build tooling. Parent owns shared contracts
and cross-client acceptance. Task dependency is explicit, not implied by hierarchy.

## Implementation Status

Approved and implemented inline on `feat/remote-file-upload`. Validation is
recorded in `research/validation.md`; final work-commit review remains pending.
Explicit platform/AI runtime gaps remain documented rather than inferred.
