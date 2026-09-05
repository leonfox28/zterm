# Additional CLI UX candidates

- Query: inspect other commands for useful human-facing improvements.
- Date: 2026-09-05.
- Scope: current repository parser, renderers, setup, pairing and device aliases.
- Status: user selected C1–C6, then explicitly removed logs -f/--follow.
  C1–C5 and C6's -n shorthand remain selected. The current PRD/design own the
  final behavior; deferred larger changes below remain outside the task.

## C1: consistent confirmation across destructive commands

Evidence: `crates/cli/src/lib.rs:259`, `:365`, `:400`, `:423`, `:870`, `:939`,
and `:1180`. Device revoke and Session close accept only --yes as the bypass;
shared interactive confirmation requires exactly lowercase yes. Identity reset
and uninstall additionally reject live Sessions without --force before their
confirmation. The approved update/stop/restart slice alone would leave these
commands with a different interaction contract.

Recommendation: extend English [y/N], y/yes and -y/--yes to these four commands;
combine reset/uninstall's identity/deletion and Session-ending impact in one
prompt and eliminate their extra public force requirement. Unlike ordinary
daemon stop/update, identity reset and uninstall still delete identity/data
without live Sessions, so they must still ask once unless -y is explicit.

Trade-off: expands approved command scope and breaks old force invocations, but
gives one understandable confirmation rule across the CLI. This is the most
direct follow-on to the user's selected lifecycle UX.

## C2: human-readable lists/status and explicit empty results

Evidence: `crates/cli/src/lib.rs:1240` renders full ID followed by alias,
outbound_known, inbound_status, generation, online, streams and attachments.
The Session renderer at line 1290 prints full ID plus name/revision/controller/
viewport fields. Both return an empty string for an empty list. Status human
rendering at line 1602 emphasizes bind attempts, publish/lookup and transport
counters, while omitting version and Session names already in StatusView.

Recommendation: use aligned names-first device/Session tables and short English
state labels; explain permissions with columns such as Can connect / Can access
this device, and use attached/detached for controller state. Show useful device
name/version/daemon/Session information in status before network details. Move
diagnostic counters into the appropriate diagnostic presentation rather than
dropping user-useful identity or authorization facts merely for aesthetics.
Do not label an unconnected peer unreachable without evidence or initiate new
network probes to render the table.

Empty-list examples:

```text
No paired devices. Run zterm pair accept to add one.
No sessions on local. Run zterm connect local to start one.
```

Retain access to full device and Session IDs, particularly inbound-only devices
without an alias. Layout design must not imply abbreviated IDs are accepted as
selectors. This is closely related to public JSON removal, but the exact new
layout and field organization need user scope approval.

## C3: default setup infrastructure without an extra question

Evidence: `crates/cli/src/lib.rs:1312` prompts for name and infrastructure
profile; an empty interactive profile selects official-n0. First noninteractive
setup still requires explicit --profile. Existing configuration is immutable
through setup (`crates/daemon/src/bootstrap.rs:70`); setup is not a config-edit
command despite its flags.

Recommendation: ordinary setup asks for the device name and directly uses the
already recommended official-n0. An explicit --profile self-hosted selects the
advanced relay path. Keep existing identities/configuration on repeated setup.
Do not imply that `setup --name` renames an already initialized device; its
conflict error should explain that initialization already exists.

Trade-off: less infrastructure vocabulary in first use, while self-hosting users
must discover the explicit advanced flag in help.

## C4: pairing terminology, success guidance and accurate help

Evidence: `crates/cli/src/lib.rs:700` emits only the ticket from pair create and
reports `Paired outbound device <full-id> as <alias>.` from accept. `--name` on
pair accept actually assigns the local alias (`lib.rs:239`), whereas setup's
--name is the device's display name. The default alias already comes from the
remote display name and collision handling (`core/src/device.rs:134`,
`daemon/src/pairing_service.rs:610`); do not propose that existing behavior as
new work. Directional permission is already implemented correctly.

Recommendation: explain the ticket's actual expiry and which device accepts
it; on acceptance show the selected alias, permitted direction and a concrete
`zterm connect <alias>` next step. Prefer --alias for pair accept if changing
this option is in scope. Preserve the ticket's single secret-bearing stdout
emission and bounded no-echo/explicit-stdin input; human guidance can go to
stderr so it does not corrupt the copyable ticket. Do not introduce a ticket
positional argument, environment variable, or duplicate ticket print.

Small confirmed help defect: PairCreateArgs at `lib.rs:233` advertises 30s as an
example, but `core/src/pairing.rs:33` requires at least 60 seconds. Correct the
example to 60s/10m/1h without changing the actual TTL policy.

Trade-off: clearer human guidance and naming, with a CLI compatibility decision
if --name is actually renamed rather than just better described.

## C5: shorten common local Session inspection

Evidence: `crates/cli/src/lib.rs:305` requires target for Session list, whereas
bare zterm already targets local main (`lib.rs:681`).

Recommendation: allow `zterm session list` to mean `zterm session list local`.
Keep explicit remote targets available. If all Session commands are later
redesigned to omit target, decide one unambiguous target syntax first: simply
making the first of several positional arguments optional is not sufficient.

Trade-off: a small additive convenience for local use; does not replace the
larger question of how users prefer to select remote devices/Sessions.

## C6: bounded log viewing (-n retained; follow excluded)

Evidence: `crates/cli/src/lib.rs:393` and `operations.rs:1046` implement only a
bounded tail with --lines, default 100 and max 1000 lines/1 MiB. There is no
follow mode or short -n option.

Final scope: add -n as the short spelling of --lines, keep the current one-shot
bounded tail and useful empty-log output. The user excluded -f/--follow to avoid
maintaining a continuous reader with rotation/restart/cancellation handling.
Do not implement those excluded behaviors as hidden scaffolding.

## Larger changes to defer until a concrete preference exists

- An interactive device/Session chooser for omitted connect arguments requires
  a defined default entry and selection/cancellation UX; it is a separate
  product interaction, not a small parser cleanup.
- Connect and Session attach overlap but have distinct create-main/existing-
  only semantics (`lib.rs:770`, `:824`). Do not merge them merely to reduce the
  command count. Session new likewise intentionally creates and attaches.
- `status` and `daemon status` share a handler, but both are understandable
  discovery paths. Removing an alias alone saves little user effort.
- Shell completion can help exact alias/Session selection, but dynamic device
  completion must not unexpectedly start the daemon or dial remote peers.
- Adding config-edit/self-rename/device-forget/remote-exec commands requires
  new behavior; none is implied by this inventory review.

## Suggested priority

C1–C5 and the narrowed C6 are selected for this task. Existing log event coverage
is separately researched in logging-gap-analysis.md. No product code or runtime
state was modified during planning.
