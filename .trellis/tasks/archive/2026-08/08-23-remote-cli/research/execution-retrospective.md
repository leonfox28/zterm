# Execution retrospective: remote CLI task

Date: 2026-08-24

## What happened

- The `remote-cli` implementation used 42 Trellis worker spawns across 20
  channels, in addition to native Codex sub-agents. Several slices reloaded the
  same task context and repeated broad checks.
- Ownership overlapped in adjacent daemon modules. In-progress edits caused
  temporary large compile failures and made otherwise independent checks wait
  for unrelated workers.
- The task expanded from M7-M8 product behavior into broad capacity, panic,
  poison, overflow, redaction, platform, multiprocess and stress matrices.
- A public/self-hosted Relay workflow was created even though production scope
  already selected official Iroh/n0, then had to be removed.
- A Linux real-Iroh harness was developed on macOS beyond the minimum needed
  for compile-only evidence, even though runtime acceptance belonged to hosted
  Linux.
- Final PTY verification spent several hours redesigning barriers around a
  harness-only ordering flake before returning to the simpler production
  sequence.
- In the preceding linked transport/auth task, three Trellis `implement`
  workers were launched concurrently through an agent card with
  `provider: claude`. The user's Claude Code configuration routed those workers
  to a separately billed DeepSeek backend. The external provider and billing
  boundary were not checked or approved first.

## Root causes

1. MVP acceptance and optional hardening were not separated after planning.
2. Parallel worker count was treated as throughput rather than coordination
   cost, despite a shared dirty worktree.
3. Green focused evidence did not trigger a stop; each small finding generated
   another implementation or review round.
4. Full checks and context injection were repeated per micro-slice.
5. Provider role names were mistaken for a safe execution boundary; the actual
   model, route and separate billing account were not resolved before spawn.

## Binding decisions for continuation

- The user authorized one bounded cleanup pass after this retrospective. That
  pass is limited to deleting redundant test infrastructure and narrowing
  test-only seams; it may not change product behavior. After it completes, do
  not add another implementation, audit, test matrix, network harness or
  documentation expansion to this task. Remaining local work is commit and
  finish-work only.
- Keep hosted Linux real-Iroh execution, hosted Windows compilation, public CLI
  OS-process evidence, M9 distribution and M10 network-lab evidence explicitly
  pending with their existing owners.
- Use native Codex sub-agents by default. Any separately billed channel/provider
  requires explicit user approval after reporting provider, model, routing,
  worker count and timeout.
- Future tasks default to one implement worker and one checker; no more than two
  concurrent workers, with disjoint file ownership.
- Use focused gates during implementation and broad gates only at phase end and
  before commit.
- Report at four hours; at eight hours stop for explicit approval before any
  further hardening or scope expansion.
- Time-box harness-only flakes to 60–90 minutes or two distinct fixes, then
  simplify or defer.

The reusable rules are also recorded in
`.trellis/spec/guides/evidence-driven-simplicity.md`, which is already included
by this task's implement and check manifests.

## User-authorized cleanup result

The bounded cleanup pass removed more than 3,200 lines of duplicated or
unexecuted acceptance infrastructure without changing product behavior:

- deleted the standalone pairing multi-process and remote Session real-Iroh
  daemon-like harnesses, neither of which had a hosted Linux run or run URL;
- retained `two_daemon_transport` as the single small real-Iroh loopback owner
  and made future workflow expansion conditional on adding its hosted job at
  the same time;
- deleted the response-discard injection that existed only for the removed
  remote Session harness; pure/Unix tests still own retry and response-loss
  behavior;
- removed the CLI's exported Active-marker test entry and its branches from the
  production renderer; the PTY process gate now invokes the real
  `run_terminal`, while pure tests own exact input-fence ordering;
- deleted the duplicate `revoke_races` target and a redundant queued-writer
  unit case. `authorization`, `local_device_ipc`, and the retained
  `session_wire` matrix preserve fairness, failure, durable ordering, detach,
  unaffected-principal, and restart evidence;
- gated the service-level revoke scheduling observer to test builds, removing
  its Option branch from production compilation.

The explicitly deferred product decisions remain unchanged: do not remove the
remote resume checkpoint or defer `reset --identity` under the label of code
cleanup.
