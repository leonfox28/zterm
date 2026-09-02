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
- CI admission runs on pull requests, `main` pushes, and manual dispatch rather
  than duplicating a PR branch through both push and pull-request events. The
  five-entry Rust matrix still expands the source-policy step on every host;
  pure version/format/docs owners may be centralized because they do not prove
  checkout bytes on another OS.
- A workflow step that requires POSIX shell behavior must name `shell: bash` on
  Windows instead of depending on the runner's default shell.

## Verification Checklist

Before changing source attributes or cross-platform workflows:

- [ ] Inspect the effective attribute with `git check-attr`, not only the text
      written in `.gitattributes`.
- [ ] Check actual checked-out Rust sources for carriage returns.
- [ ] Preserve deliberate path-specific attributes such as journal merge rules.
- [ ] Use `just check-fast` for the local edit loop; it includes the local
      source-policy and portable policy owners.
- [ ] Run `just check` before push or delivery. It is the authoritative local
      gate, while hosted runners retain evidence for their own checkout bytes.
- [ ] Let the Linux, macOS, and Windows matrix execute the same policy probe.
- [ ] When a policy compares command output byte-for-byte, set color and other
      presentation controls at that command's owning boundary; do not inherit
      runner-wide formatting such as `CARGO_TERM_COLOR=always`.
- [ ] Consider adjacent platform assumptions: executable bits, case-sensitive
      paths, path separators, symlinks, and default shells.

For platform-specific Rust modules, gate the private implementation at the
complete boundary: imports, fields, helper types, helper functions, and impl
methods. Keep only the intentionally shared public API unconditional. Do not
hide native-runner failures with `allow` attributes or fake references. A
cross-compile that stops inside a native dependency is useful diagnostics, but
only the hosted target runner is acceptance evidence.

For host-only terminal engines, distinguish three separate claims:

- macOS/Linux behavior belongs to native real-PTY tests on each hosted OS;
- Windows shared-boundary acceptance compiles/tests the engine crate but does
  not imply that the Unix login-shell runtime exists on Windows;
- Android/iOS remote-client acceptance is dependency isolation of core/proto,
  not a claim that the host engine, local PTY, or renderer is supported there.

An outer Ghostty, kitty, Alacritty, or tmux process is another independent ANSI
consumer. It does not nest or share the daemon's terminal-engine object. Test
the fixed hosted TERM/COLORTERM profile at the PTY builder boundary rather than
inferring child capabilities from the developer's outer terminal.

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

## Incident: Unix Private Service State Reached Windows

- **Root cause categories**: change-propagation failure plus test coverage gap.
  Unix listener dispatch was gated, but its private service fields, helpers,
  and imports remained visible to the Windows target and failed `-D warnings`.
- **Evidence**: macOS/Linux were green while the hosted Windows compile reported
  dead private state; a local macOS cross-build stopped earlier in `ring`
  because the Windows SDK was unavailable and could not validate this layer.
- **Prevention**: keep the public unsupported-platform boundary constructible,
  gate the entire private native implementation, and require the hosted Windows
  shared-contract job before completing a cross-platform milestone.

## Incident: Process-Reap Latency Was Used as Concurrency Evidence

- **Root cause categories**: implicit assumption plus test-evidence mismatch. A
  shutdown test treated successful child scheduling, signal handling, exit, and
  reap within 100 milliseconds as proof that two close operations started
  concurrently.
- **Evidence**: the same production path passed Linux x86_64/arm64, macOS
  Intel, and repeated local macOS runs, while a loaded macOS ARM runner missed
  only the fixed reap window. The underlying PTY library itself permits a
  longer Unix signal grace period before escalation.
- **Prevention**: prove concurrency at the ownership boundary with an explicit
  state transition, barrier, or notification showing that every independent
  owner received the operation before waiting. Verify eventual process/thread
  cleanup separately under one realistic absolute deadline. Do not use a
  sub-grace-period wall-clock bound on OS scheduling, signal delivery, or reap
  as a proxy for concurrency. If the expected behavior depends on child startup
  code such as a signal trap, the child must emit readiness after that setup;
  `spawn()` returning or a parent-side sleep is not a readiness barrier.

## Incident: Ubuntu Tool Availability Was Assumed on macOS

- **Root cause categories**: implicit assumption plus test coverage gap. A
  four-platform release matrix required `command -v shellcheck` in every job
  even though only the Ubuntu runner contract provided ShellCheck.
- **Evidence**: the same signed installer passed both Linux jobs, while both
  macOS jobs exited before syntax or fixture execution at the tool-presence
  probe. Signing had already required human approval.
- **Prevention**: assign each host tool to one explicit runner owner and test
  that ownership in workflow policy. Run platform-independent lint once on the
  generated artifact before approval/signing, then keep portable syntax checks
  and real behavior fixtures on every target. Never put an unconditional
  `command -v <tool>` in a multi-OS matrix unless every runner explicitly
  installs that tool or the repository has executable evidence that it is part
  of every selected image.

## Incident: Python HTTPServer Resolved a Hostname Before Listening

- **Root cause categories**: implicit assumption plus test coverage gap. The
  HTTPS fixture used a numeric loopback address but inherited
  `HTTPServer.server_bind`, which still performs a fully qualified hostname
  lookup after binding and before listening.
- **Evidence**: both Linux installer jobs passed while both GitHub-hosted macOS
  jobs timed out waiting for the fixture port file. GitHub runner maintainers
  confirmed that `socket.getfqdn()` can stall in this interval behind macOS
  local-network privacy ([runner-images#14409](https://github.com/actions/runner-images/issues/14409)).
- **Prevention**: a numeric-loopback fixture must bind through
  `socketserver.TCPServer.server_bind`, derive its published address from the
  bound socket, and have a socket-free source-policy regression that rejects
  the inherited FQDN path. Do not convert an unnecessary lookup into a longer
  timeout or retry loop.
