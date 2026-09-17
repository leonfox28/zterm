# CLI plan reassessment

Date: 2026-09-16. Requested by the user before approving the consolidated command tree.
Status: planning review only; no product implementation or task activation.

## Overall assessment

Retain the approved mixed command model. The useful criterion is whether users
can predict the affected device, Session, and lifetime from an invocation, not
whether every verb has the same spelling or every command has equal depth.
The previous decisions remain approved unless the user explicitly changes them.

Retain:

- Top-level `status`; remove the identical `daemon status` handler entry.
- One explicit connection command, `connect`, with an optional device target
  defaulting to `local`.
- `session list/create/rename/close` with `--target` defaulting to `local`.
- The requested `new` to `create` rename and immediate attachment after creation.
- Automatic QR plus manual ticket presentation and the approved fallback rules.
- Simple `reset` with the existing deletion confirmation and managed-state scope.
- Direct removal of replaced spellings without compatibility aliases.

## Behavior revision accepted on 2026-09-16

The user replied “按你说的来” to this reassessment. The revised connection
contract below is now incorporated into PRD R2.3/R2.4.

The current CLI computes `create_main` from whether the Session string equals
`main` (`crates/cli/src/lib.rs:775`). The separate `session attach` path always
passes `create_main: false` (`crates/cli/src/lib.rs:818`). Thus removing that
command under the approved plan removes the convenient existing-only attach
path by the name `main` (the full Session ID remains existing-only).

The accepted revision derives creation permission from whether the user supplied
`--session`:

| Invocation | Accepted behavior |
| --- | --- |
| `zterm` after setup / `zterm connect` | Connect to local `main`, creating it if absent |
| `zterm connect laptop` | Connect to laptop's `main`, creating it if absent |
| `zterm connect laptop --session main` | Connect to existing `main`; report missing rather than create |
| `zterm connect laptop --session work` | Connect to existing `work`; report missing rather than create |
| `zterm session create work --target laptop` | Create `work`, then attach the exact created Session |

This keeps the convenient default while giving every explicitly selected
Session the same existing-only contract. It needs no new public option or wire
capability: the existing terminal request and runtime already carry selector
and `create_main` separately (`crates/cli/src/terminal_ui.rs:789`,
`crates/daemon/src/operations.rs:1163`).

Trade-off: explicit `--session main` no longer means the same thing as omitting
`--session`. The user accepted this difference; it supersedes the earlier
name-based auto-create decision.

## Clarifications that do not require redesign

- `connect laptop` and `session close work --target laptop` need not share the
  same positional layout. The primary object of connect is the device; the
  primary object of a Session management operation is the Session. Keep their
  already approved forms instead of adding another syntax migration.
- `session create` still means create-and-attach. It requires an interactive
  terminal; do not quietly convert it into detached/scriptable creation just
  because `create` sounds like a resource operation. Its help should state the
  immediate attachment behavior.
- Reset clears managed local state (identity, configuration, pairing data and
  running Sessions), not merely a cache. It preserves the executable. No new
  deletion scope or confirmation mechanism follows from removing `--identity`.
- `pair create` removes its three product options, not standard `-h/--help`.
  Its QR and copyable text refer to one ticket. Non-TTY stdout contains exactly
  the ticket plus newline; instructions/expiry and fallback diagnostics belong
  on stderr. Genuine output-write failure still fails the command.
- The 600-second pairing default already has an owner at
  `crates/core/src/pairing.rs:31`; deleting the public TTL override does not imply
  deleting protocol validation or introducing another independent constant.
- Bare `zterm` and `connect` equivalence is qualified by completed setup and
  ordinary interactive use. Before setup, bare invocation currently prints
  guidance (`crates/cli/src/lib.rs:648`); do not accidentally change its exit or
  initialization behavior while making the target optional.

## Planning artifact cleanup completed after acceptance

- PRD R2.3 now uses the accepted explicit-selector contract.
- PRD R2.5 references R2.9 for the authoritative local-first creation syntax.
- R2 reflects that target parameters and pair presentation were already decided.
- Repeated decision bullets were consolidated into requirements and observable
  acceptance criteria; the target grammar is recorded in one dedicated section.

The earlier contradictions were documentation drift, not reasons to re-ask
settled product questions. General output layout is the remaining planning topic.
