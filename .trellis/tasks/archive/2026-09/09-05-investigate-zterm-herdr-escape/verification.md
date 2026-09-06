# Verification: fixed Ctrl+] command mode

## Scope and approval

The user approved the final reduced proposal with “好，开始执行吧”, then asked
“继续”. The task was started as in_progress. Implementation, checks and review
were performed directly in the main agent, without sub-agents.

This fixes the CLI input-boundary architecture: decoded legacy/enhanced keys now
reach one command owner before child encoding. It adds only period -> Detach,
local unknown/timeout cancellation, explicit quoting and fixed-prefix CLI/help.
No input-method adaptation, command-triggered keyboard flags, device switching,
Herdr branch or daemon production change is included.

## Regression evidence

- Before implementation, the focused production-codec regression
  `decoded_enhanced_prefix_and_period_dispatch_local_detach` failed: enhanced
  prefix/period produced forwarded input instead of Detach. This agrees with
  the retained generic/Herdr baseline research at 3f77461.
- The same regression passes through the new shared owner, including mixed
  legacy/enhanced input and one-byte fragmentation.
- Generic `daemon_autospawn` enhanced-prefix fixture: a deterministic shell
  enables flags 15, receives no unknown/timeout command input (authoritative
  revision stays unchanged), detaches with encoded prefix/period, restores the
  outer PTY and remains reattachable with the same Session ID and flags.
- Retained physical Ghostty/Doubao evidence is unchanged. U+3002 in modes 0/7
  still cancels locally; a reported period key matches. No new physical-key
  capture or physical-input compatibility claim is made.

## Checks

Host: macOS arm64; Rust 1.98.0. Logs are local ignored build artifacts.

| Command | Observed result |
| --- | --- |
| `cargo +1.98.0 test -p zterm-cli --lib --test daemon_autospawn --all-features --locked` | Exit 0; 67 CLI tests passed, 3 existing helper tests ignored in the parent harness; outer-PTY harness passed. Log: target/prefix-final-checks.log |
| `cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings` | Exit 0 after lifecycle additions. Log: target/prefix-final-clippy.log |
| `cargo +1.98.0 fmt --all -- --check` | Passed |
| Explicit Rustfmt check of included prefix.rs/keyboard.rs/session.rs | Passed; include! source files require explicit coverage |
| `just check` | Exit 0 on the final tree, including the parameterized SS3 regression: policy, workspace Clippy/tests/docs, dependency checks, relay probe/upstream/static checks. CLI: 67 passed, 3 existing helper tests ignored in the parent harness. Log: target/prefix-final-gate.log |
| `git diff --check` | Passed |
| `task.py validate` | Passed; pre-existing large local-daemon-ipc spec exceeds automatic injection limit, so relevant sections were read directly. New command contract is a separate bounded document. |

The local gate retains its hosted-only boundary for other supported hosts,
glibc 2.28, Docker/QEMU runtime checks, signing and publication. No release,
installation or existing user Session was changed.

## Direct full-scope review

Reviewed all changed CLI production code, CLI argument/help tests, generic PTY
fixture and its shared daemon test shell, user documentation, task artifacts
and backend specs. The frontend spec is a placeholder and no frontend UI code
is changed. Daemon/core/protocol production ownership is unchanged.

Findings corrected during review:

- Track held identities only when the last presented flags request releases;
  otherwise distinct unknown commands could accumulate stale records.
- Retire owned releases despite changed Ctrl modifiers, normalize representable
  legacy letter/Tab/Enter/Backspace identity, and bound held-key state at 64.
- Keep an unknown UTF-8 scalar or opaque CSI/Alt/SS3 frame intact across reads;
  consume the attempted command while preserving unrelated following input.
- Keep already-encoded fallthrough tagged so a lossy encoding cannot create a
  second prefix decision. Preserve copy priority and pre-invalidation encoding.
- Document the fixed prefix in all three command help entries and assert it
  alongside normal removed-option rejection.

Tests cover each changed contract, including capacity/reset, timeout-owned
release, forwarding identity, copy/paste/history/pointer and existing fences.
The spec records why committed IME text cannot establish a physical-key lease.

## Completion

Implementation, full-scope review, final checks and specification updates are
complete. All acceptance criteria are met within the recorded evidence limits.
The user authorized “走发布流程吧”: execute the reviewed work commit and
Trellis bookkeeping, then prepare and publish v0.1.20 through the repository
operator. Release authorization supersedes the earlier implementation-only
publication exclusion. Local installation and user Session changes remain
outside this release operation.
