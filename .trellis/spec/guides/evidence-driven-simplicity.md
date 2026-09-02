# Evidence-Driven Simplicity Guide

Use this guide before adding validation, fallback, recovery, monitoring, or
deployment machinery. The goal is not minimum line count; it is the shortest
mechanism that satisfies an observed contract.

## Before adding complexity

- [ ] Name the concrete failure mode in one sentence.
- [ ] Identify who or what can actually produce it; do not treat trusted
      repository metadata as arbitrary Internet input.
- [ ] Identify the one trust boundary that owns the invariant.
- [ ] Check whether an existing platform already enforces it.
- [ ] Remove the proposed mechanism mentally. If no current behavior, data, or
      security boundary changes, do not add it.
- [ ] Name the current consumer of every metric, artifact, manifest, or output.
      No consumer means the feature is deferred, not prebuilt.
- [ ] Prefer a standard primitive over a custom parser, validator, monitor, or
      state machine.

## Validation ownership

Validate an invariant once at its owning boundary:

| Invariant | Typical owner |
| --- | --- |
| Downloaded upstream bytes | Artifact build/download boundary |
| Product version | Product manifest/release boundary |
| Registry image integrity | Registry and container engine |
| Runtime availability | One post-start service check |
| User input | The first externally controlled input boundary |
| PTY output controls and terminal extras | One bounded policy before the host terminal engine |

Repeat a validation only when the second check catches a different failure.
The review or test description must name that distinct failure; “defense in
depth” by itself is not sufficient.

A third-party terminal grid may manage its own allocation while the product
still bounds input-controlled strings, replies, events, and combining extras.
Do not reconstruct allocator/RSS admission from `size_of` or cache capacity
unless an approved product rule consumes that estimate. Count/dimension limits
and hostile-input caps remain useful because they protect different boundaries.

## Recovery and state

- Stateless components default to replace/recreate.
- A short manual escape hatch is enough when failure is reversible and no data
  migration exists.
- Add automated rollback or routine rollback drills only for persisted state,
  irreversible migrations, material availability requirements, or a real
  incident that recreation cannot address.
- Do not execute a recovery path after the new path has already passed its
  acceptance check merely to prove that the recovery path exists.

## Testing

- Give each contract one authoritative test.
- Cover the happy path and the smallest negative case that crosses a real
  boundary.
- Do not enumerate syntax combinations already rejected by a standard parser.
- Do not reproduce a runtime test in static text assertions unless each catches
  a different defect.
- Count maintenance cost, execution time, and service interruption as test
  costs, not as free thoroughness.

## Execution budget and delegation

- Start from the approved acceptance criteria. Classify every proposed extra
  check as either a current blocker or deferred hardening; do not silently turn
  an MVP into a security or platform-completeness audit.
- Default to one implement worker followed by one independent check worker for
  a coherent slice. At most two workers may run concurrently, and only with
  disjoint file ownership. More workers or overlapping ownership require an
  explicit user-approved reason.
- Do not spawn another reviewer merely to repeat a green review. A follow-up
  worker must name the new defect, missing acceptance criterion, or failed
  gate it owns.
- Run focused checks while iterating. Run the broad workspace gate once at the
  end of a phase and once before commit, unless a concrete cross-workspace
  regression requires another run.
- At four hours of active work, report completed scope, remaining scope, and
  the largest risk. At eight hours, stop and obtain explicit approval before
  continuing hardening or widening the task.

## Provider and billing boundary

- An agent role such as `implement` or `check` does not identify its model or
  billing account. Before launching a channel or local provider CLI, resolve
  the actual provider, model, routing/base URL, worker count, and timeout.
- A provider that can bill outside the current Codex session must never be
  launched without explicit user approval for that task. Local configuration
  or an agent-card default is not approval.
- Prefer native Codex sub-agents when they are available. Do not silently fall
  back from a native worker to `claude`, another CLI, or an externally routed
  model.
- Stop on the first authentication, quota, or unexpected-cost signal. Do not
  add concurrent workers or allow automatic retry loops to amplify it.

## Flaky tests and harnesses

- First decide whether the failure is in production behavior or only in the
  harness. Capture one content-free observation that distinguishes the two.
- Time-box a harness-only flake to 60–90 minutes or two materially different
  fixes. If neither works, simplify the scenario or record it as deferred;
  do not keep adding barriers, production seams, or stress matrices.
- A compile-only platform target proves compilation only. Keep its harness
  minimal until the owning hosted platform can execute it.
- Stress repetition is confirmation after a fix, not a substitute for a
  causal model. Do not repeatedly run broad loops while the failure mechanism
  is still unknown.

## Persistence across compaction

- Record decisions that change scope, provider/billing, delegation, test
  strategy, or stop conditions in the active task artifact or this guide before
  continuing. Commentary and chat history alone are not durable project state.
- A session continuation must read the active task artifacts and applicable
  guides before spawning workers or widening verification.

## Review examples

Bad:

```text
Cargo validates SemVer -> custom shell parses SemVer again -> OCI tag strips
the prefix -> deployment validates a registry digest again -> release switches
to an old image after the new image already passed.
```

Good:

```text
Cargo owns the product version -> the Release tag must equal "v" + that
version -> Docker publishes and pulls the same tag -> one live service check
accepts the deployment.
```

## Stop condition

Once all observable acceptance criteria pass, stop. Additional checks require
a newly identified risk or requirement, not a general desire to be exhaustive.
If the remaining item cannot be executed on the current platform, record the
exact external owner and command, then stop building local substitutes for that
evidence.
