# Implementation validation

## Scope reviewed

- `crates/cli/src/terminal_ui/composition.rs`: retain active/live/in-bounds
  coordinates regardless of child visibility; copy the child's visibility.
- `crates/cli/src/terminal_ui.rs`: real compositor/presenter regression for
  main and alternate layouts, with visible → hidden → hidden movement → visible
  transitions and unchanged cells. Assert the final physical CUP/visibility,
  capture/sync ordering, one write/flush, and equality no-op behavior.
- `.trellis/spec/backend/local-daemon-ipc.md`: document that independent
  position/visibility contract and its assertion points.
- Task documentation/research only otherwise; no protocol, dependency, daemon,
  native-terminal settings, installation, or release change.

## Red/green evidence

Before the product correction:

```text
cargo test -p zterm-cli --lib hidden_cursor
FAILED: screen=Main cursor=(2,3,false)
Actual final cursor output: ESC[1;1H ESC[?25l
```

After the correction, the same test passes for both layouts, including movement
while hidden. The source-block probe now yields:

```text
visible: child=(34,27,true) outer=(34,27,true)
hidden: child=(34,27,false) outer=(34,27,false)
hidden-moved: child=(35,31,false) outer=(35,31,false)
history: child=(34,27,true) outer=(0,0,false)
reconnecting: child=(34,27,true) outer=(0,0,false)
```

## Checks

- PASS: targeted hidden-cursor regression (failed before, passed after).
- PASS: `cargo test -p zterm-cli`: library 82 passed / 3 isolated helpers
  ignored; binary unit test and command/setup integration tests passed;
  daemon autospawn harness exited successfully; doc tests passed.
- PASS: `cargo fmt --all -- --check`.
- PASS: `just ci-policy`: version/dependency policy, format, Actionlint,
  release/installer policy and offline operator/candidate fixtures, ShellCheck,
  and canonical Python syntax checks.
- PASS: `cargo clippy -p zterm-cli --all-targets -- -D warnings`.
- PASS: `cargo build -p zterm-cli --bin zterm`; candidate at
  `target/debug/zterm`, executable version check prints `zterm 0.1.22`.
- PASS: diff whitespace check. No model/wire change, duplicated owner,
  dependency addition, runtime workaround, or scope drift in review.

The initial local repair verification did not claim the full-workspace
pre-push/release gates. The user subsequently authorized formal publication;
the existing release operator will require exact PR CI and main candidate
evidence before tagging. This section records the completed local checks only.

## Native GUI acceptance

**PASS — user-confirmed on 2026-09-06.** After receiving the explicit candidate
build command and instructions to test Chinese input in Herdr/Pi, the user
reported: “测试了没有问题了”. This closes acceptance for the original failure.
Exact remote/outer-terminal versions and separate post-fix shell/Codex/direct
Herdr comparisons were not supplied; do not claim independently observed runs.

Computer-use inventory showed Ghostty running. Access with
`cua.getApp("com.mitchellh.ghostty")` was refused with:

> Computer Use is not allowed to use the app 'com.mitchellh.ghostty' for safety reasons.

No alternate UI automation mechanism was used to bypass that restriction.
The assistant therefore deferred native verification to the user, who confirmed
it above. The assistant did not modify any user terminal session, input method
setting, or installed Zterm executable.

## Manual candidate smoke

1. In the old Zterm frontend, use `Ctrl+]` then `.` to detach while retaining
   the existing Herdr/Pi session.
2. From a local shell, run the candidate:

   ```sh
   /Users/huyuanzhe/.paseo/worktrees/0xwpkru7/wicked-spider/target/debug/zterm connect dev
   ```

3. In the existing Herdr Pi editor, begin Chinese composition and verify the
   preedit/candidate UI follows the editor. Move the insertion point, repeat,
   and compare shell/Codex and Herdr without Zterm.
4. Record the actual outer terminal and `dev` endpoint versions. If positioning
   still fails, preserve the failure evidence and investigate the documented
   DEC-2026 post-sync anchor question before expanding the correction.

This change is in the local CLI presenter/compositor; the remote daemon does
not need this source patch for the coordinate fix. The candidate reports the
same development version, so use its explicit path rather than the installed
`zterm` command. Native acceptance is now recorded; commit and archival
remain as task bookkeeping.
