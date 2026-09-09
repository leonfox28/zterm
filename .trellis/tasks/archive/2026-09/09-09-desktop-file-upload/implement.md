# Desktop upload implementation plan

## Prerequisites

- [x] Parent review approved; shared service child validated and its API available.
- [x] Activate this child; load trellis-before-dev and input-command/local-daemon-IPC/
  shared-client/core-wire/terminal-color specs and cross-layer guides.

## Ordered work

1. Verify/pin desktop clipboard dependency and supported OS features; inspect reuse.
2. Implement injectable file-list/raw-image source preparation and bounded jobs.
3. Add Upload/CancelUpload to existing prefix owner and continuing command executor.
4. Add single operation/pause state, cancellation/origin fences and internal path
   insertion using the existing synchronized writer; ordinary Ctrl+V remains routed.
5. Extend status composition with right-aligned progress and paced updates, preserving
   child geometry, clipboard-copy priority and sole presenter output.
6. Integrate shared uploader, typed failures and source/partial cleanup.
7. Run focused regressions and actual remote/clipboard smoke fixtures; update usage.

## Validation

- cargo +1.98.0 test -p zterm-cli
- cargo +1.98.0 clippy -p zterm-cli --all-targets --all-features -- -D warnings
- cargo +1.98.0 fmt --all -- --check
- cargo +1.98.0 deny check
- sh tests/source-policy.sh
- sh tests/terminal-dependency-policy.sh

New meaningful tests: file-before-image selection, text/multi/directory refusal,
raw PNG byte size, prefix enhanced/legacy/repeat/release, no upload on Ctrl+V,
paused Enter/text/mouse never replayed, cancel/detach during source/transfer/final,
no stale completion, one upload on repeats, right alignment/Unicode/narrow/single-row/
resize/alternate layout and preserved terminal cursor after status paint.

Use disposable remote Sessions to upload image/PDF/binary and cap-boundary fixtures.
Record real clipboard/outer-terminal environment and actual host file byte checks;
TUI fixtures do not prove physical IME interoperability. Do not submit unintended
AI prompts while exercising path insertion.

## Risk and rollback

Risky owners: terminal_ui/session.rs, prefix.rs, StatusRenderer and composition.rs.
Do not change terminal reporting modes to accommodate the new command. Revert only
this feature's source/command/chrome wiring if necessary; retain underlying terminal
state and uploaded files. Parent review is required for changed UX contracts.
