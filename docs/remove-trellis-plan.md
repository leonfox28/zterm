# Remove Trellis from zterm

Status: project and global uninstall completed on 2026-09-30.
Branch: `chore/remove-trellis`.
Inspected baseline: `b8364539d27b8a5c9fa9c496eab507dfd8e7be73`.

The user subsequently requested official project-level removal followed by
global CLI removal. No Trellis task was created. The execution record below
describes the completed scope; the broader documentation migration proposed
later in this document remains a historical proposal.

## Execution record

- Executed `trellis uninstall --yes`, then
  `npm uninstall -g @mindfoldhq/trellis`, in that order.
- Verified that `.trellis/`, the managed platform files, and the `trellis` / `tl`
  executables are absent. Neither the CLI nor its core package remains in the
  active global npm environment; no other NVM Node installation contains the CLI.
- Verified that the independent Penpot MCP configuration is byte-for-byte
  unchanged using a fingerprint comparison without printing its connection value.
- Removed the obsolete journal Git attribute, its source-policy assertion, and
  the special secret-scan exclusion. Updated the existing scan fixture to cover
  generated build output while retaining credential-rejection cases.
- Retained the double-NAT evidence byte-for-byte at
  `docs/verification/controlled-qad-double-nat.md` and repaired its reference in
  `docs/foundation-gate.md`.
- Before uninstall, saved and verified a private archive of the complete local
  `.trellis/` tree, including ignored data, outside the repository in temporary
  storage. Tracked specs, task research, and journals also remain recoverable
  from the baseline commit. The proposed engineering-docs/roadmap migration and
  replacement `AGENTS.md` were not performed in this uninstall step.
- Passed `sh tests/source-policy.sh`, `sh tests/secret-scan.sh`,
  `sh tests/secret-scan-fixture.sh`, `just ci-policy`, and `git diff --check`.
  The remaining checkout contains no Trellis or Mindfold references outside this
  historical plan. Fresh-session behavior in each AI platform was not exercised.
- Completed the local pre-push checks before opening the removal PR: workspace
  Clippy, 727 passing Rust tests (11 existing ignored cases), documentation,
  dependency policy, relay-probe format/Clippy, shell syntax, both upstream relay
  artifact checksums, and relay static checks. The `just check` invocation stopped
  at the sandbox's read-only Cargo advisory-cache lock; `just ci-dependencies`
  passed outside the sandbox, followed by the remaining relay checks separately.

The sections below retain the original inventory and migration proposal for
reference; their paths describe the pre-uninstall state.

## Official removal command

Use the official `trellis uninstall` command for managed-file removal. The
CLI and project were both version `0.6.17` at inspection; local command help confirmed:

```sh
trellis uninstall --dry-run  # Preview only
trellis uninstall            # Remove after interactive confirmation
```

`--yes` / `-y` skips the command's confirmation prompt. The
[official implementation](https://github.com/mindfold-ai/Trellis/blob/main/packages/cli/src/commands/uninstall.ts)
uses `.trellis/.template-hashes.json` to identify owned platform files, strips
managed configuration entries, and deletes the entire `.trellis/` directory.
Preserve wanted specs, task research, journals, and unique ignored data first;
uninstall does not create a recovery backup.

A read-only preview on 2026-09-30 reported **251 deletion entries** (including
the entire `.trellis/` tree, so this is not a recursive file count) and edits to
`.cursor/hooks.json` and `.codex/config.toml`. It covers all five installed
platform integrations and shared skills. The current `AGENTS.md` contains only
the managed block and is scheduled for deletion; create its replacement after
uninstall. The current generated `.pi/settings.json` is also scheduled for
deletion. Local scrubber inspection confirms that independent Codex MCP fields
are retained while the generated fallback setting and `agents.max_depth = 1`
are removed.

The command does not handle this repository's custom source-policy assertion,
Git attribute rule, secret-scan fixture, documentation migration, or evidence
links. These remain the project-specific work in this plan. Prefer the official
command for removal and inspect the resulting diff for residual coupling.

For a temporary comparison, the official alternative is `trellis ablate`
followed by `trellis restore`; both support previews and use an external recovery
transaction. See the [official README](https://github.com/mindfold-ai/Trellis#faq).
Removing the globally installed CLI is a separate package-manager operation,
such as `npm uninstall -g @mindfoldhq/trellis` for an npm global installation.

## Repository findings

| Surface | Observed integration | Planned treatment |
| --- | --- | --- |
| `AGENTS.md` | Entire file is a Trellis-managed instruction block | Replace with native project instructions and links to engineering documentation |
| `.trellis/` | Workflow, Python scripts, runtime agent definitions, configuration, specs, tasks, journals, template metadata | Extract useful project knowledge, then remove the tree |
| `.agents/skills/trellis-*/` | Shared workflow skills and their reference files | Remove all Trellis skill directories |
| `.codex/` | Three Trellis agents, three hook scripts, `hooks.json`, and mixed project configuration | Remove Trellis agents/hooks and their registrations; edit `config.toml` selectively |
| `.cursor/` | Trellis agents, commands, skills, three hooks, and `hooks.json` | Remove the generated integration and its registrations |
| `.grok/` | Trellis agents, commands, and skills | Remove the generated integration |
| `.opencode/` | Trellis agents, commands, skills, three plugins, three helper modules, and a plugin dependency manifest | Remove the integration, private helpers, and its otherwise unused `package.json` |
| `.pi/` | Trellis agents, prompts, extension, and settings registration | Let the official command remove the integration and current generated settings; preserve independent additions if present at execution time |
| `.gitattributes` | Journal-only `merge=union` rule and Trellis comments | Remove the journal rule; retain repository text/LF policy |
| `tests/source-policy.sh` | Explicit assertion that the Trellis journal merge rule exists | Remove that assertion with the attribute rule |
| `tests/secret-scan.sh` and `tests/secret-scan-fixture.sh` | Trellis directory exclusion and a matching fixture | Remove the obsolete exclusion and update the fixture |
| `docs/foundation-gate.md` | Link to task-owned double-NAT evidence | Move the evidence into ordinary docs and update the link |

The targeted search found no Trellis references in product Rust/Kotlin sources,
protocol files, Cargo manifests/lockfile, release tooling, or `justfile`.
CI reaches the affected source-policy and secret-scan scripts through existing
checks. There is no product dependency replacement to design.

`.codex/config.toml` also contains an independent Penpot MCP configuration.
Preserve that configuration without copying its connection value into the plan,
migration notes, or review output. Let the official scrubber remove its generated
fallback setting, Trellis-specific commentary, and recursion pin.

The shared `.agents/` directory and five platform directories should be cleaned
by owned file/directory, so any independent configuration survives. The official
Pi scrubber also removes the generated `enableSkillCommands` setting; the current
file becomes empty and is deleted.

## Preserve project knowledge

### Engineering contracts

Use `docs/engineering/` as the ordinary documentation home. Preserve the existing
`backend/`, `frontend/`, and `guides/` layout so most relative links remain valid.

The current 36 spec files comprise 18 populated backend/frontend contracts,
10 placeholder templates, five thinking guides, and three indexes.

1. Copy the 16 populated backend contracts and the two Android frontend
   contracts to their equivalent paths under `docs/engineering/`; official
   uninstall will remove their original copies later.
2. Retain the five guides after removing Trellis implementation examples,
   workflow commands, task-state requirements, and compulsory agent dispatch.
   Preserve project decisions about bounded scope, evidence, architecture,
   cross-platform behavior, and avoiding duplicated checks.
3. Verify that the 10 placeholder templates contain no project-specific rules,
   then omit them. Rebuild the three indexes without placeholder links or
   template-filling instructions. Add a short engineering landing page.
4. Update the relay-deployment secret-scan contract, the transport-auth review
   wording, and the cross-platform journal rule to match the removal.
5. Check relative links, inline path references, and code examples. Preserve
   current contracts during the move; broad editorial rewrites can follow later.

The new `AGENTS.md` should identify the Rust workspace and Android app, point to
`docs/development.md` and the applicable engineering indexes, and list the
existing validation entry points. Plans can use ordinary Markdown when needed.
Project instructions should require neither task registration nor journal
updates, automatic commits, phase activation, or Trellis commands.

### Evidence and outstanding work

Copy the record currently at
`.trellis/tasks/08-24-e2e-hardening/research/controlled-qad-double-nat.md` to
`docs/verification/controlled-qad-double-nat.md`, and replace the reference in
`docs/foundation-gate.md` with a relative Markdown link. Preserve the record's
historical date and simulated-network evidence limits.

Nine task records currently say `in_progress`. Review their latest acceptance
notes against the current implementation before removing their records:

- `00-bootstrap-guidelines`
- `08-20-cross-platform-relay-terminal-mvp`
- `08-24-distribution-release`
- `08-24-e2e-hardening`
- `09-02-migrate-alacritty-terminal`
- `09-04-terminal-selection-copy`
- `09-06-android-app`
- `09-06-remote-self-update`
- `09-07-terminal-presentation-continuity`

Capture confirmed unfinished product work in a concise `docs/roadmap.md`, with
its remaining acceptance criteria and any necessary research links. Treat stale
task status as historical metadata, not proof that a feature is unfinished.
Copy any uniquely needed supporting notes alongside the roadmap or verification
docs and repair their links. Keep archived task bundles, journals, temporary
experiments, and task JSON/JSONL records in Git history rather than duplicating
the entire task system under a new directory.

The inspected baseline above retains the tracked history. For example:

```sh
git show b8364539d27b8a5c9fa9c496eab507dfd8e7be73:.trellis/workspace/leon/journal-1.md
```

## Implementation order

1. **Preserve project knowledge and local data.** Copy the engineering contracts,
   selected evidence, and confirmed outstanding work into ordinary docs. Update
   links and review for lost project rules. Keep the original `.trellis/` files
   and manifest available until uninstall. Inventory `.trellis/.developer`,
   `.trellis/.runtime/`, Python caches, and the two ignored `.backup-*`
   directories; preserve any unique wanted material outside the checkout before
   uninstall recursively deletes them as well.
2. **Use official removal.** Repeat `trellis uninstall --dry-run` against the
   prepared state, then run `trellis uninstall` when executing this plan. Let it
   remove owned skills, agents, commands, hooks, plugins, the Pi extension, and
   the shared runtime, while scrubbing mixed configuration. Verify that the
   independent Penpot MCP block is retained. Keep the source files unchanged
   during extraction so there is no need to bypass the CLI's uncommitted-data
   protection.
3. **Finish project-specific cleanup in the same change.** Write the new native
   `AGENTS.md`; remove the journal Git attribute and source-policy assertion;
   remove the secret-scan exclusion and update its fixture. Confirm the migrated
   evidence link resolves. These changes must accompany the uninstall before
   the removal commit is considered complete.
4. **Audit residual local files.** The official command deletes the entire
   `.trellis/` tree, including ignored contents. Inspect any platform files
   outside its manifest and remaining empty directories individually. The
   current `.codex/skills/` directory is empty; preserve independent local files
   introduced since this inventory.
5. **Validate and review.** Run the focused checks below, inspect the final diff,
   and verify a fresh AI session. Keep documentation migration and integration
   removal as separately reviewable commits if that improves rename visibility.

Repository removal is separate from the user's global CLI installations,
user-level AI configuration, conversation history, and external channel data.
Those remain user-managed. Git history provides recovery for tracked files;
unique ignored files need the separate preservation described above.

## Verification

### Repository and integration checks

- Confirm that no tracked files remain under `.trellis/` or the removed Trellis
  skill, agent, hook, plugin, command, prompt, and extension paths.
- Search tracked working-tree content for `trellis`, `mindfold`, old script
  paths, and removed hook filenames. Only this migration plan and deliberately
  retained historical provenance may contain explanatory references; active
  instructions, configuration, tests, and engineering docs must be independent.
  Review each match rather than ignoring all of `docs/`.
- Parse surviving TOML/JSON configuration without displaying connection values.
  Confirm the independent MCP configuration is unchanged and every remaining
  registered file exists.
- Check migrated Markdown links and the foundation evidence reference.
- Start a fresh session in the tools used for this checkout. Verify that startup
  and a normal prompt produce no Trellis bootstrap, task-consent prompt,
  workflow-state injection, or missing-hook error. Existing sessions may retain
  previously injected instructions until restarted. Record which platforms were
  exercised; file inspection alone is not a five-platform runtime test.
- Check both the tracked result and existing-checkout ignored leftovers. A clean
  checkout should work without installing Trellis.

### Focused executable checks

Update the existing secret-scan fixture to check a still-supported generated
output exclusion such as `target/`, and a harmless scanned source. Retain the
positive credential-rejection cases. Keep source and documentation directories
inside the scan; do not replace the removed exclusion with a broader one.

Run after implementation:

```sh
sh tests/source-policy.sh
sh tests/secret-scan.sh
sh tests/secret-scan-fixture.sh
just ci-policy
git diff --check
```

Use the existing `just check` pre-push gate at the normal integration point.
Product source changes are not expected for this removal; if they become
necessary, reassess scope and select the relevant runtime tests. This planning
change itself needs only document review and diff checks.

## Completion criteria

- Project knowledge is reachable from ordinary docs and `AGENTS.md`.
- Active project instructions and tool integrations no longer depend on Trellis.
- The removed journal rule and secret-scan exception have no stale assertions.
- Current document links resolve; outstanding product acceptance work is retained.
- The repository and the cleaned local checkout contain no active Trellis runtime.
- Independent tool configuration is preserved, and applicable checks pass.
