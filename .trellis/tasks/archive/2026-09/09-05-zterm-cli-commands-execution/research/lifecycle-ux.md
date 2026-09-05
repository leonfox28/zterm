# Public output and daemon lifecycle UX evidence

Research on 2026-09-05 after the user selected the first redesign behaviors.
No product code or runtime state was changed.

## Public JSON removal

- `crates/cli/src/lib.rs:217` defines shared JsonArgs for status, doctor,
  daemon status, and device list; SessionListArgs at line 305 owns its own flag.
- The status/doctor handlers at lines 1438/1447 choose text or JSON. Device and
  Session renderers also branch on JSON. Review dedicated serialization view
  types for deletion when their public output consumer is removed.
- Existing CLI assertions consume JSON in
  `crates/cli/tests/command_side_effects.rs` and
  `crates/cli/tests/daemon_autospawn.rs`; revise those checks around observable
  text or existing typed runtime results without retaining a hidden substitute
  for the deleted public output mode.
- README, `docs/core-local-daemon.md`, and `docs/remote-cli.md` list the public
  options. Audit backend specs too when implementing.
- Do not confuse Zterm public JSON flags with unrelated `gh --json` uses in
  release tooling.

## Internal JSON has an active verification consumer

- `crates/daemon/src/distribution.rs:252` emits ReleaseSelfCheck JSON from the
  hidden flag; `run_candidate_self_check` at line 499 parses those exact bytes
  into ReleaseSelfCheck for candidate validation.
- `install/versioned.sh.in:191` and release fixtures also invoke this hidden
  candidate entry. Removing public text/JSON selection does not require
  redesigning the trusted candidate verification contract.

## Confirmation can use existing daemon-owned data

- `crates/daemon/src/service.rs:88` exposes active Session count/names in daemon
  status; SessionImpact at line 108 contains count/names/interruption state.
- `crates/daemon/src/client/ipc.rs:274` provides update_preflight without
  stopping the daemon.
- `LocalRuntime::stop` (`operations.rs:1010`) currently rejects positive Session
  count without force, reporting only count and retry guidance.
- `LocalRuntime::restart` (`operations.rs:1031`) calls stop, waits for shutdown,
  then ensures the daemon. A stopped configured daemon is explicitly started.
- `crates/cli/src/lib.rs:1462` and line 1472 directly forward force to stop and
  restart; update at line 916 also has no interaction callback today.
- Existing confirmation for other destructive commands requires literal yes.
  The user subsequently selected C1: all seven affected commands share y/yes
  and -y/--yes; reset/uninstall combine deletion and Session impact once.

## Update ordering and startup

- `LocalRuntime::update` (`operations.rs:390`) currently owns the complete
  transaction: managed-build validation, bounded download/authentication,
  daemon compatibility and Session-impact checks, stop/wait, lifecycle lock,
  candidate activation/post-check, install metadata, commit, lock release.
- It returns previous/installed version and ended Session names at line 485;
  the CLI prints that the daemon remains stopped at `lib.rs:935`.
- CLI interaction must occur after authenticated preparation and before stop.
  The design needs a narrow prepared-update/confirmation boundary or existing
  equivalent; avoid stopping first or downloading the same candidate twice.
- Zero-Session inspection is not blanket approval to kill a Session that starts
  before the stop request. Follow-up source inspection found that
  `service.rs:892` currently decodes and ignores LocalStopRequest.force,
  immediately invoking SessionService::shutdown_until. There is no authoritative
  daemon-side interruption check to preserve; implementing one is necessary
  for the requested confirmation contract.
- `session.rs:874` starts shutdown through the existing registry admission
  gate. RegistryState owns live, provisional, cleanup-only and Starting name
  reservations. Decide idle admission under that same lock before changing
  `accepting`, then use the existing bounded shutdown implementation. Do not
  create a second session count or lock owner in the CLI.
- Startup must use the activated executable after lifecycle-lock release.
  `lifecycle.rs:124` already launches that exact stored executable path with
  the hidden daemon argument and waits for readiness. `client/ipc.rs:164` and
  `:1256` return daemon version/protocol observations without requiring them to
  equal the caller's package version. For the current unchanged wire format,
  reuse that launcher and validate readiness against the authenticated installed
  manifest, rather than against the old updater's version. Future wire-format
  migration remains an explicit release compatibility task.
- Pre-setup updates are currently possible. They have no configured identity
  with which to start a daemon; the final design must make this boundary
  visible without implicitly running setup.
- Candidate activation rollback remains separate from post-update daemon-start
  failure. The user requires honest final status; the exact implementation
  should not conflate a successful binary replacement with a failed startup.

## Planning status

The user has approved public JSON removal, English conditional y confirmation
for the three named lifecycle commands, daemon startup after successful update,
`-y` for direct execution, and removal of those commands' `--force` option.
The plan gives `-y` the conventional `--yes` long spelling and keeps internal
wire boolean fields intact. The subsequently selected C1–C5 and logs -n are
recorded in additional-cli-ux.md; all public force flags are removed. The latest
user refinement excludes logs -f without changing the other selected work.
