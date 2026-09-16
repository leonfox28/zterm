# Implementation plan

Status: CLI and user-approved security update implemented; full `just check` passed on 2026-09-16. User approved the two-commit plan; committing locally without push.
Mode: Codex inline. One integrated CLI change, no implement/check subagents.

## 0. Review and environment gate

- [x] Present the complete PRD/design/plan for review; obtain approval for this
  final summary, then `python3 .trellis/scripts/task.py start .trellis/tasks/09-15-cli-command-format`.
- [x] Load trellis-before-dev and the relevant spec indexes/guidelines before
  editing product files. Follow trellis-check after implementation and
  trellis-update-spec for the owning command/output contracts.
- [x] Recheck git status and preserve unrelated work. User resolved the Xcode
  license blocker; initial Git status contained only this task's untracked files.
- [x] Verify `just`, Rust 1.98.0 and available validation tools. Do not execute
  setup/reset/update/uninstall/pair against the user's actual device state.

## 1. Public grammar and Session intent (R2.1-R2.5, R2.9-R2.11)

- [x] Update enums/Args in crates/cli/src/lib.rs, removing the two public
  operations and legacy flags without aliases; preserve hidden-entry parsing.
- [x] Use optional ConnectArgs.session so omission alone enables create_main.
  Bare configured invocation uses None, and explicit main is existing-only.
- [x] Apply default local targets and named --target Session arguments.
- [x] Retain create-and-attach identity/error behavior and reset preflight.
- [x] Migrate current test/example invocations in the same slice so no test
  accidentally continues covering a deleted spelling instead of behavior.

Gate: current parser/command-side-effect tests cover the new grammar, rejected
old arity/flags, help/version and unconfigured no-side-effect behavior. Extend
the existing local-only daemon/PTY fixture to demonstrate missing explicit
main does not create a Session, then default connect creates/reuses it. Pure
request assertions cover a remote alias carrying the same selection intent;
do not create a real remote connection on macOS for a CLI parser change.

## 2. Automatic pairing presentation (R2.6-R2.8)

- [x] Make Pair Create parameter-free and keep the core's default TTL owner.
- [x] Refactor the small QR renderer to accept presentation constraints;
  compose QR and manual ticket from one zeroizing ticket result.
- [x] Preserve exact pipe stdout, stderr guidance, same-ticket fallback and
  checked writes. Remove PNG export code/tests and stale fallback suggestions.
- [x] Keep png/tempfile dependencies required by clipboard image upload.

Gate: synthetic-ticket tests exercise wide-terminal, narrow-terminal, non-TTY
and capacity/render-failure output; byte-level assertions show exact pipe output
and matching original ticket. Retain redacted Debug/errors and verify one real
writer failure is reported. Use the existing QR module test owner rather than
adding an acceptance path that mints real user tickets.

## 3. Human output and streams (R5)

- [x] Update status/setup summaries, device/Session lists and empty hints with
  the accepted plain text layout and complete identifiers.
- [x] Update success and confirmation text while preserving exact preflight
  identities and operation-specific confirmation triggers.
- [x] Route prompts/progress/errors to stderr and results to stdout; doctor and
  logs remain query results. Flush before waiting for input.
- [x] Add minimal typed error context where required by the accepted examples,
  retaining terminal restoration and original error categories/partial success.
- [x] Update typed-error match sites if the CLI error wrapper gains a variant;
  keep secret/cwd/route data out of context and Debug.
- [x] Update recovery hints in daemon/update.rs and any current code text that
  recommends removed commands; no daemon behavior change is intended.

Gate: existing renderer tests cover useful fields/Unicode/full IDs/direction
labels; existing confirmation fixtures cover cancellation, -y and redirected
stdout. Add only focused assertions for the changed error/stream contracts,
including created-session partial failure and unknown-outcome preservation.
Do not add per-string tests for every minor cosmetic wording change.

## 4. Docs, specs and consumer consistency (R3/R4)

- [x] Update README and current CLI/local-daemon/install/Android docs, affected
  examples and owned spec clauses, preserving historical task evidence.
- [x] Search for removed spellings/options and explicit-main create hints.
  Classify hits: current instructions must change; negative tests and archive
  history may intentionally retain old syntax; internal RPC names are not CLI.
- [x] Compare all 19 target operations with actual help and docs. Verify pair
  help still exists, hidden flags stay hidden and no public JSON/force returns.

## 5. Validation and final quality gate

Run relevant focused checks after each meaningful slice; at final code state
run the authoritative workspace gate rather than inventing a parallel full suite.

```sh
cargo +1.98.0 test -p zterm-cli --lib --all-features
cargo +1.98.0 test -p zterm-cli --bin zterm --all-features
cargo +1.98.0 test -p zterm-cli --test command_side_effects --all-features
cargo +1.98.0 test -p zterm-cli --test setup_permissions --all-features
cargo +1.98.0 test -p zterm-cli --test daemon_autospawn --all-features
just check
git diff --check
```

Use filtered tests where sufficient during the edit loop. daemon_autospawn is
harness=false and runs its existing isolated Unix/PTY acceptance program; do
not assume libtest filtering applies to it. Bounded fixtures own cleanup.

- [x] trellis-check: confirm scope, spec compliance, final diff and data flow
  from argv to create_main/selector to existing LocalRuntime, including errors.
- [x] Record exact commands/results and any external blockers or hosted-only
  evidence separately; do not call blocked/unrun checks passed.
- [x] Inspect help/output only with side-effect-free flags or task-private
  fixtures. No production-state demos or real Iroh execution on developer macOS.
- [x] Report completed behavior, compatibility changes and actual verification;
  session wrap-up follows trellis-finish-work when appropriate. No release,
  publication or unrelated cleanup is implied by this plan.

## Rollback points

After grammar/intent, pairing presentation and output slices, ensure affected
owners are internally coherent before moving on. A failing contract is fixed
at that owner; do not restore a removed alias to make an old test pass. There
is no storage migration. Restore only task-owned changes if rollback is needed,
preserving all unrelated work and ordinary live state.

## Execution evidence (2026-09-16)

- Activated with `task.py start --allow-empty-context`: Codex inline loads specs in
  the main session; empty JSONL files intentionally do not dispatch subagents.
- Focused combined cargo test invocation: CLI lib (101 passed, 3 existing helper
  ignores), bin (2 passed), command_side_effects (1 passed), setup_permissions
  (1 passed), and harness-false daemon_autospawn (exit 0). Additional parser/table
  assertions are included in the final workspace gate.
- Initial compilation caught a test-only Zeroizing<String> Display mismatch;
  changed the synthetic QR assertion to use as_str(), then focused tests passed.
- Actual side-effect-free help inspected for connect/session/create/pair/reset.
  Current invocation search retains old syntax only in rejection tests/specs.
- `git diff --check` passed. `just check` passed source/format/release policy,
  workspace Clippy, secret scans, all workspace tests and doc build. CLI's final
  workspace run contains 105 lib tests (102 passed, 3 existing helper ignores),
  plus passing bin and integration targets.
- Dependency audit initially hit the sandbox's read-only Cargo advisory lock.
  Elevated `just ci-dependencies` then failed on existing rustls 0.23.43:
  RUSTSEC-2026-0285, fixed in >=0.23.45. Bans/licenses/sources passed.
  HEAD already contains the same rustls version; this task has no Cargo.toml or
  Cargo.lock diff. Do not silently expand CLI scope to upgrade network dependencies.
- Continued the remaining original gate steps independently: isolated Relay probe
  fmt/Clippy, relay shell syntax, both upstream archive verifications and Docker
  Compose static checks passed. Separate probe dependency audit failed on the
  same pre-existing rustls 0.23.43 advisory; its bans/licenses/sources passed.
- That first authoritative gate was NOT green; the approved follow-up below
  resolves it. Real Iroh and cross-UID hosted evidence remain outside
  this macOS run; existing explicit-only/helper ignores were preserved.
- No production device state, network acceptance, release, commit or push executed.

## Approved dependency follow-up (2026-09-16)

User authorized fixing the audit blocker. Update the workspace's exact rustls pin
and both workspace/probe lockfiles from 0.23.43 to patched 0.23.45 using targeted
Cargo updates. Preserve features, TLS configuration and unrelated dependencies.
Re-run dependency audits and the authoritative `just check` gate. No new API,
protocol or platform runtime contract is introduced; no advisory suppression.

Results:

- Cargo targeted updates changed only rustls version/checksum in each lockfile;
  the root manifest's exact pin changed to =0.23.45, with features unchanged.
- Both dependency audits pass advisories/bans/licenses/sources.
- A fresh full `just check` exited 0 with rustls 0.23.45: policy, Clippy,
  secret scans, workspace tests, docs, both dependency audits, probe fmt/Clippy,
  relay shell checks, official archive verification and Compose static checks.
- `git diff --check` passed. No additional spec convention is needed for this
  dependency-only patch; the CLI contracts were already updated in owning specs.
- No production state mutation, commit, push or release was performed.

## Approved commit batch (2026-09-16)

1. `fix(deps): upgrade rustls to 0.23.45`
   - `Cargo.toml`
   - `Cargo.lock`
   - `tests/relay/handshake-probe/Cargo.lock`
2. `feat(cli): simplify commands and unify output`
   - `crates/cli/src/lib.rs`
   - `crates/cli/src/main.rs`
   - `crates/cli/src/pair_qr.rs`
   - `crates/cli/src/terminal_ui.rs`
   - `crates/cli/tests/command_side_effects.rs`
   - `crates/cli/tests/daemon_autospawn.rs`
   - `crates/daemon/src/update.rs`
   - `README.md`
   - `docs/android.md`
   - `docs/core-local-daemon.md`
   - `docs/install.md`
   - `docs/remote-cli.md`
   - `.trellis/spec/backend/local-daemon-ipc.md`
   - `.trellis/spec/backend/terminal-input-commands.md`
   - `.trellis/tasks/09-15-cli-command-format/` (all task artifacts and research)

Unrecognized dirty files: none. Do not push. Archive/journal follow only after
the work commits, using the repository's finish-work prerequisites.
