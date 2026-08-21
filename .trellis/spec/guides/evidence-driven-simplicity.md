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

Repeat a validation only when the second check catches a different failure.
The review or test description must name that distinct failure; “defense in
depth” by itself is not sufficient.

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
