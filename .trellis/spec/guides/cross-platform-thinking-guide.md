# Cross-Platform Thinking Guide

> **Purpose**: Treat repository bytes, checkout behavior, and tool invocation as
> part of the build contract on every supported operating system.

---

## The Boundary

A source file can be identical in Git but different in a working tree. Git
attributes, client defaults such as `core.autocrlf`, filesystem behavior, and
the selected shell all sit between the repository and the compiler or formatter.
Local macOS/Linux success therefore does not prove that a Windows checkout has
the same bytes or command semantics.

## Required Source Checkout Contract

- The repository root `.gitattributes` owns text normalization. Text files use
  LF working-tree endings unless a future format has a documented exception.
- Rust sources must resolve to `eol=lf`; do not rely on each contributor's Git
  configuration to match `rustfmt.toml`.
- More-specific attributes remain independently testable. In particular,
  Trellis developer journals must continue to resolve `merge=union`.
- Cross-platform CI must run `sh tests/source-policy.sh` immediately after
  checkout and before `rustfmt` or compilation in every OS matrix entry.
- A workflow step that requires POSIX shell behavior must name `shell: bash` on
  Windows instead of depending on the runner's default shell.

## Verification Checklist

Before changing source attributes or cross-platform workflows:

- [ ] Inspect the effective attribute with `git check-attr`, not only the text
      written in `.gitattributes`.
- [ ] Check actual checked-out Rust sources for carriage returns.
- [ ] Preserve deliberate path-specific attributes such as journal merge rules.
- [ ] Run the source-policy regression and `cargo fmt --all -- --check` locally.
- [ ] Let the Linux, macOS, and Windows matrix execute the same policy probe.
- [ ] Consider adjacent platform assumptions: executable bits, case-sensitive
      paths, path separators, symlinks, and default shells.

## Incident: Windows Rust Formatting Failure

- **Root cause categories**: implicit assumption plus test coverage gap. The
  initial workflow assumed checkout bytes were platform-invariant and invoked
  `rustfmt` before asserting that contract.
- **Evidence**: the first public CI run passed Unix jobs but Windows reported an
  incorrect newline style for all Rust sources; the repository requires Unix
  newlines in `rustfmt.toml`.
- **Prevention**: root-level LF normalization makes the intended bytes
  structural, while the pre-format matrix probe detects attribute drift and
  actual carriage returns close to checkout.
- **Expansion**: future platform-specific behavior must be validated at the
  earliest shared boundary rather than inferred from one developer machine.
