# Approved work commit

Message: `fix(update): complete upgrades after the originating session ends`

One coherent commit includes the independent updater, real PTY regression and
failure-boundary tests, CLI integration, installation guidance, and executable
spec/task records. All dirty paths are changes from this session; there are no
unrecognized files to include or exclude.

## Product and specification files

- `.trellis/spec/backend/distribution-lifecycle.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/logging-guidelines.md`
- `.trellis/spec/guides/cross-layer-thinking-guide.md`
- `crates/cli/src/lib.rs`
- `crates/cli/src/main.rs`
- `crates/daemon/src/distribution.rs`
- `crates/daemon/src/lib.rs`
- `crates/daemon/src/operations.rs`
- `crates/daemon/src/service.rs`
- `crates/platform/src/local_unix.rs`
- `docs/install.md`
- `crates/daemon/src/update.rs`
- `crates/daemon/src/update/tests.rs`

## Task artifacts

Include `.trellis/tasks/09-06-remote-self-update/` (task metadata, PRD, design,
implementation checklist, inline-context seed files, root-cause/verification
research, and this review). Keep real signed remote acceptance marked pending.

## Verification and limitations

See `research/verification.md`. The updater regression has failed before and
passed after the fix. Existing cancellation, signatures, rollback, daemon
readiness, no-setup and identity boundaries are retained. No real server or
production installation was changed. Linux runtime verification subsequently
passed in hosted CI; an official signed remote update remains a separate
acceptance item. See `research/release.md` for publication evidence.

## Approval boundary

The user approved this concrete work commit plan by requesting the release
workflow on 2026-09-06. This authorizes committing the fix, preparing v0.1.24,
pushing its PR, merging after required checks, and publishing through the
repository release operator. It does not claim a production server upgrade or
completion of the remaining real signed remote acceptance.
