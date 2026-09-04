# Root-Cause and Architecture Thinking Guide

Use this guide whenever diagnosing or fixing a bug. The goal is not to turn
every defect into a refactor. The goal is to decide, with evidence, whether the
reported example is a local implementation mistake or a symptom of a missing
system invariant, wrong ownership boundary, duplicated state, or incomplete
architecture.

## Non-negotiable rule

A failing application, input, platform, or screen is evidence and a regression
fixture; it does not define the solution scope by itself. Before editing product
code, record a root-cause classification in the active task:

- **Local implementation defect**: an existing, adequate architecture or
  contract is violated by one implementation path.
- **Architecture / boundary defect**: the current design has no single owner or
  invariant capable of making all equivalent paths correct.
- **Undetermined**: current evidence cannot distinguish the two; investigate
  before choosing patch or refactor scope.

The classification is a diagnostic result, not a preference for larger changes.
Do not claim “local” merely because a small patch is available, and do not claim
“architectural” merely because a redesign is attractive.

## Diagnose before changing code

- [ ] State the observable failure without naming the proposed fix.
- [ ] Build or identify the smallest application-neutral reproduction when
      practical; keep the real application as an external smoke fixture.
- [ ] Trace the complete causal path from authoritative input/state to the
      externally wrong result, including every boundary and final writer.
- [ ] Name the intended invariant and the layer that should own it.
- [ ] Search current code, tests, specs, and prior task research for sibling
      exceptions, duplicated state, or earlier failures at the same boundary.
- [ ] Identify one observation that distinguishes a local violation from a
      missing or incorrectly placed invariant.
- [ ] Record the classification and evidence before implementation scope is
      approved. If evidence changes, stop and reconverge the plan.

## Signals that require an architecture audit

- The same class of failure appears in multiple features, platforms, states, or
  transitions.
- More than one layer can write the same externally visible state or advance
  competing baselines.
- Correctness depends on ordering between components that do not share one
  committed model.
- A proposed fix needs process names, titles, glyphs, themes, timing delays,
  repeated repaint, or other recognition of the reporting example.
- Several branches independently calculate the same ownership, lifecycle, or
  derived state.
- A new client or platform exposes that the existing boundary transports a
  backend-specific representation instead of domain semantics.
- Fixing one symptom would leave equivalent inputs to fail through another
  path.

When these signals are present, define the missing invariant at the highest
stable owning boundary. Map all producers and consumers, then prefer one state
owner and one transition contract over accumulating repair branches.

## Evidence that a local fix is appropriate

- The owning contract already covers the failing and sibling cases.
- A single implementation path demonstrably violates that contract while other
  paths follow it.
- The correction does not require new cross-layer state, application identity,
  hidden ordering, or a second source of truth.
- An application-neutral regression fails before the correction and passes
  afterward, while existing contract tests cover the surrounding architecture.

In this case, fix the local implementation directly. Do not enlarge the task
into an unrelated redesign merely to satisfy this guide.

## Scope and completion rules

- A local fix is complete when the violated invariant and regression are both
  restored, and the task records why no architecture change is needed.
- An architecture fix is not complete when only the reported example works.
  Acceptance must verify the new invariant across equivalent paths and remove
  or bypass the obsolete competing ownership model.
- Compatibility fallbacks may remain, but they must have an explicit owner,
  sunset or compatibility rationale, and their own correctness contract. A
  fallback repair must not be presented as completion of the target
  architecture.
- Prefer internally staged, reversible commits for a large migration. Internal
  checkpoints do not narrow the approved final outcome or justify releasing a
  temporary architecture unless the user explicitly approves that boundary.

## Wrong vs correct

Wrong:

```text
Herdr loses its last-column bar after scrolling
-> detect Herdr or repaint the last column after each wheel event
-> add another exception when the next TUI behaves differently
```

Correct architecture classification:

```text
Last-column cells survive in the authoritative model but not on the host
-> trace every presentation writer and committed baseline
-> discover that semantic content is encoded before later UI composition
-> compose one desired frame and let one presenter own the physical transition
-> retain the Herdr case only as one regression/smoke fixture
```

Correct local classification:

```text
One encoder emits an inclusive erase for an empty suffix
-> the existing row-replacement contract already forbids touching that cell
-> correct the encoder and add an application-neutral boundary regression
-> record evidence that no competing owner or missing state model remains
```

## Durable handoff

For non-trivial bugs, the active task's `prd.md` or research note must preserve:

1. symptom and application-neutral reproduction;
2. root-cause classification;
3. causal evidence and rejected alternatives;
4. owning invariant and affected boundaries;
5. why the chosen fix scope is sufficient;
6. acceptance criteria for the failure class, not only the original example.

Future implementers and reviewers must reload that record. Chat history alone is
not an acceptable source of the task's architectural goal.
