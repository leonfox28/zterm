# Thinking Guides

> **Purpose**: Expand your thinking to catch things you might not have considered.

---

## Why Thinking Guides?

**Most bugs and tech debt come from "didn't think of that"**, not from lack of skill:

- Didn't think about what happens at layer boundaries → cross-layer bugs
- Didn't think about code patterns repeating → duplicated code everywhere
- Didn't think about edge cases → runtime errors
- Didn't think about future maintainers → unreadable code

These guides help you **ask the right questions before coding**.

---

## Available Guides

| Guide | Purpose | When to Use |
|-------|---------|-------------|
| [Code Reuse Thinking Guide](./code-reuse-thinking-guide.md) | Identify patterns and reduce duplication | When you notice repeated patterns |
| [Cross-Layer Thinking Guide](./cross-layer-thinking-guide.md) | Think through data flow across layers | Features spanning multiple layers |
| [Cross-Platform Thinking Guide](./cross-platform-thinking-guide.md) | Verify checkout bytes and command behavior across operating systems | Source attributes, CI matrices, or platform tooling changes |
| [Evidence-Driven Simplicity Guide](./evidence-driven-simplicity.md) | Require a real failure model and one validation owner before adding machinery | Validation, fallback, recovery, monitoring, deployment, or test expansion |
| [Root-Cause and Architecture Thinking Guide](./root-cause-and-architecture-thinking-guide.md) | Classify a bug as a local contract violation or an architecture/boundary defect before choosing fix scope | Every bug diagnosis; especially recurring, cross-layer, state, ownership, or new-platform failures |

---

## Quick Reference: Thinking Triggers

### When to Think About Cross-Layer Issues

- [ ] Feature touches 3+ layers (API, Service, Component, Database)
- [ ] Data format changes between layers
- [ ] Multiple consumers need the same data
- [ ] You're not sure where to put some logic
- [ ] You are adding an event kind, JSONL record, RPC payload, or config field
- [ ] UI / command code starts casting raw payload fields directly

→ Read [Cross-Layer Thinking Guide](./cross-layer-thinking-guide.md)

### When to Think About Code Reuse

- [ ] You're writing similar code to something that exists
- [ ] You see the same pattern repeated 3+ times
- [ ] You're adding a new field to multiple places
- [ ] **You're modifying any constant or config**
- [ ] **You're creating a new utility/helper function** ← Search first!
- [ ] Two files read the same untyped payload field with local casts
- [ ] Multiple branches update the same derived state from `kind` / `action`

→ Read [Code Reuse Thinking Guide](./code-reuse-thinking-guide.md)

### When Verifying AI Cross-Review Results

- [ ] Reviewer claims "user input can be malicious" → Check the actual data source (internal manifest? user config? external API?)
- [ ] Reviewer flags "missing validation" → Is the data from a trusted internal source?
- [ ] Reviewer says "behavior change" → Read the code comments — is it intentional design?
- [ ] Reviewer identifies a "bug" in test → Mentally delete the feature being tested — does the test still pass? If yes → tautological test

**Common AI reviewer false-positive patterns**:
1. **Trust boundary confusion**: Treating internal data (bundled JSON manifests) as untrusted external input
2. **Ignoring design comments**: Flagging intentional behavior documented in code comments as bugs
3. **Variable misreading**: Not tracing a variable to its actual definition (e.g., Map keyed by path vs name)

**Verification rule**: Every CRITICAL/WARNING finding must be verified against the actual code before prioritizing. Budget ~35% false-positive rate for AI reviews.

### When Adding Safety or Deployment Machinery

- [ ] A new validator, fallback, rollback, monitor, metric, or deployment layer is proposed
- [ ] The same invariant is already checked elsewhere
- [ ] A test matrix is growing faster than observable behavior
- [ ] A stateless service is gaining recovery state or rollback automation
- [ ] An artifact or metric has no current consumer

→ Read [Evidence-Driven Simplicity Guide](./evidence-driven-simplicity.md)

### When Diagnosing or Fixing a Bug

- [ ] The reported application or input is starting to define the solution
- [ ] A small patch is available, but the owning invariant is still unclear
- [ ] Multiple writers, baselines, state copies, or timing assumptions interact
- [ ] The same class of bug has appeared in another state, feature, or platform
- [ ] A fix may require a new architecture, but evidence may still show a local
      implementation error

→ Read [Root-Cause and Architecture Thinking Guide](./root-cause-and-architecture-thinking-guide.md)

### When to Think About Cross-Platform Issues

- [ ] A workflow runs on more than one operating system
- [ ] Source attributes, line endings, executable bits, or shell selection change
- [ ] A local formatter or compiler result is being used as evidence for Windows
- [ ] Paths, case sensitivity, symlinks, or platform defaults affect the command

→ Read [Cross-Platform Thinking Guide](./cross-platform-thinking-guide.md)

---

## Pre-Modification Rule (CRITICAL)

> **Before changing ANY value, ALWAYS search first!**

```bash
# Search for the value you're about to change
grep -r "value_to_change" .
```

This single habit prevents most "forgot to update X" bugs.

---

## How to Use This Directory

1. **Before coding**: Skim the relevant thinking guide
2. **During coding**: If something feels repetitive or complex, check the guides
3. **After bugs**: Add new insights to the relevant guide (learn from mistakes)

---

## Contributing

Found a new "didn't think of that" moment? Add it to the relevant guide.

---

**Core Principle**: 30 minutes of thinking saves 3 hours of debugging.
