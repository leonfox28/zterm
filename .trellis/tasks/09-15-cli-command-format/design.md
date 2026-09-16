# CLI command and presentation design

Status: implemented and validated; user-approved rustls security update removes the audit blocker, full `just check` passed (see implement.md). Implements PRD R1-R5.

## Scope and ownership

Keep one integrated task: command parsing, error context, help and output share
the same CLI owners and acceptance boundary. No independently releasable child
deliverable warrants splitting the task or dispatching agents.

| Owner | Planned change |
| --- | --- |
| `crates/cli/src/lib.rs` | Public grammar, request construction, human output and confirmation |
| `crates/cli/src/main.rs` | Public result/error stream formatting and checked writes |
| `crates/cli/src/pair_qr.rs` | Terminal-only QR rendering and deterministic presentation seams |
| `crates/cli/src/terminal_ui.rs` | Minimal outer error-context propagation after terminal restoration, if needed |
| `crates/cli/tests/*` and existing unit tests | Migrate invocations; cover changed behavior at current owners |
| `crates/daemon/src/update.rs` and `operations.rs` | Replace public recovery hints recommending daemon status; no lifecycle redesign |
| README, current docs/specs | Synchronize syntax, semantics, output and examples |

The CLI continues calling `LocalRuntime`. Setup validation, frozen mutation
targets, lifecycle locks, network ownership, replay and persistence stay in
their existing layers. No wire/schema/storage change or new runtime dependency
is needed. Read the owning specs through trellis-before-dev before coding.

## Parser and request contracts

PRD Target Command Tree is the single target grammar. Remove old enum variants
and flags rather than accepting them as aliases. Keep exact parsing, default
help/version and exclusive hidden entry behavior.

`ConnectArgs.target` gains clap's `local` default. `session` becomes
`Option<String>` with no clap string default: omission is meaningful.

| Parsed session selection | Terminal request |
| --- | --- |
| None | selector None, create_main true |
| Some name or ID, including main | selector Some(value), create_main false |

Bare configured invocation constructs the omitted-session path. Preserve its
unconfigured guidance/exit behavior. This mapping applies unchanged to local
and remote targets; the runtime already accepts selector and create_main
separately. Terminal preparation still validates TTY before attachment/mutation.

Rename Session New to Create and remove Attach. Each Session management Args
owns a named `--target` with default local; remaining positionals are exactly
the Session/name/new-name in the PRD. Avoid a new global target flag or compatibility
parser. Complete old invocations fail because their command or argument arity is
invalid; malformed extra arguments must fail before `LocalRuntime` executes.

Remove ResetArgs.identity and its post-parse guard, preserving the existing
reset preflight and expected DeviceId commit. Remove PairCreateArgs and its
TTL parser; call the existing core default without deleting protocol TTL checks.

## Output and error presentation

Reuse current string renderers, `padded`, Unicode display widths and shell
quoting. Modest pure rendering helpers are appropriate; no generic table engine,
new theme/config system or output abstraction is needed.

- Status uses Setup/Daemon to represent the current observation without the
  redundant raw State line. Retain every useful field from the approved sample;
  unavailable identity fields are omitted rather than invented. Convert only
  known state labels to human text, preserving distinct network observations.
- Session list includes the requested target, Name/State/Size and full IDs.
  The caller's target spelling is display context, never a new routing authority.
- Device list uses Name/Connection/Known host/Allowed here; known is local
  outbound state, allowed is inbound authorization (Yes/No/Revoked). Keep the
  existing alias/name fallback and full IDs.
- Empty results use valid new commands and correctly shell-quoted arguments.
- Success messages retain exact operation IDs where already provided. Mutation
  confirmation retains frozen IDs even if a human alias/name is displayed too.
- Reset's existing preflight has identity and Session names but no display name.
  Render `Reset this device: local` plus the identity; do not perform a second
  state observation or extend the daemon DTO for a decorative heading.
- Setup/pair/confirmation prompts go to stderr and are flushed before reads;
  confirmations keep current y/yes, EOF and noninteractive behavior. Doctor
  reports and log tails stay stdout query results; daemon log serialization is
  unchanged. No new ANSI styling in ordinary text output.

Use one public error-prefix owner in main (`Error:`); do not double-prefix
clap parse errors or touch hidden entry output. Preserve CliError's underlying
typed error and Debug redaction. A small typed operation context may carry
the command, requested target and Session selector where needed for display;
it must not contain tickets, cwd, routes, terminal text or arbitrary snapshots.

For deferred connections, retain safe request context across run_terminal and
apply it only after terminal restoration. Do not modify the terminal event loop
or infer missing Sessions from error text. The confirmed SessionNotFound kind
can produce the approved `session list --target ...` hint; DeviceNotFound,
unauthorized, transport failures, cancellation and unknown outcomes keep their
distinct source meaning. CreatedSessionAttach preserves the created ID and
partial success. If an error lacks reliable context, print the existing useful
typed diagnostic rather than fabricating a target or recovery action.

Checked stdout writes must propagate real failure, including flush failure.
Retain main's zeroizing PairTicket owner. Inject readers/writers only at the
small I/O seam necessary to verify prompts and result separation.

## Pair create data flow

1. Parse the parameter-free command; help and rejected legacy flags exit early.
2. Create exactly one ticket through LocalRuntime using DEFAULT_PAIR_TTL_SECONDS.
3. Detect stdout TTY/width; use the terminal QR encoder only if renderable.
4. Compose one zeroizing stdout buffer: QR plus raw ticket in interactive mode,
   exactly raw ticket plus newline for pipes. Emit expiry/receiving instructions
   and any presentation fallback explanation on stderr, flushing at output
   boundaries so the combined terminal view is readable.
5. Flush the ticket result and release zeroizing owners. A QR presentation
   failure falls back to the same ticket; stdout write failure still fails.

Pass observed dimensions to a pure terminal renderer so width and capacity
failures can be exercised with synthetic tickets without real pairing/network.
Keep fixed monochrome modules and quiet zone. No PNG writes or extra ticket
minting occur in this path.

Delete pair_qr's check_destination/write_png and their export-only tests.
Do not remove `png` or `tempfile`: `crates/cli/src/terminal_ui/upload.rs:177`
and `:185` use them for clipboard image upload. The scoped change should not
require a Cargo dependency/lockfile edit.

## Compatibility and migration

- Breaking public spelling changes are intentional; no aliases or warning period.
- Explicit --session main becomes existing-only. Generated default-session
  hints must use connect without --session; old explicit-main creation examples
  must be migrated. Existing-session callers can keep explicit main.
- No state or protocol migration. Old clients retain their old CLI semantics;
  the new CLI uses capabilities already supported by the current service.
- Update README, docs/remote-cli.md, docs/core-local-daemon.md, docs/android.md,
  docs/install.md and any other current invocation consumers found by search.
- Update owning local-daemon-ipc and terminal-input-commands specs and adjacent
  clauses actually affected, using trellis-update-spec during implementation.
  Preserve archival evidence, private internal daemon-status RPC terminology,
  and installer's hidden JSON self-check contract.

## Verification and rollback

On 2026-09-16 the user approved the audit follow-up: raise the exact workspace
rustls pin and both workspace/isolated-probe lock entries from 0.23.43 to 0.23.45
for RUSTSEC-2026-0285. Preserve features and every unrelated dependency; this
explicit security patch is the sole exception to the original no-dependency-edit
scope. It introduces no project API or protocol change.

Behavioral owners are listed in implement.md. The material risks are accidental
creation from explicit selectors, old argv reaching a different target, noisy
pipe output, lost secret redaction, and changing a destructive confirmation.
Use existing fixtures for these, not a new daemon/network harness.

Run focused changed-behavior checks and the existing authoritative `just check`
gate once the final code is ready. Developer macOS never executes real Iroh
acceptance; preserve hosted/ignored boundaries. The user resolved the planning-time
Xcode license blocker; Git and Rust 1.98.0 are available during implementation.

If implementation exposes a missing data contract affecting the chosen UX,
record it and revise planning rather than growing backend scope silently.
With no persisted/wire change, rollback is the prior code/version; undo only
task-owned edits and never discard unrelated work. Any previously created
Session or pairing ticket retains its normal lifetime and is not auto-deleted
as part of CLI rollback.
