# Mainline refresh and host prerequisite — 2026-09-07

## Repository baseline

The user requested development on the changed remote mainline. `git fetch
origin` followed by `git merge --ff-only origin/main` advanced the current
`holy-zebra` branch from `68eab39087ec1e564dec536cf01c7664da5e211c` to
`de25a387fb512dd06ae0bb7f8e3afcd4bbf5f6db` (workspace version `0.1.24`).
There were no divergent local commits and no merge conflicts. Existing
project-scoped Penpot configuration and untracked Android planning/prototype
files remain in place. This is the implementation baseline; earlier research
continues to describe its explicitly dated inspection revision.

Relevant upstream changes:

- `8416f4d` preserves a hidden live cursor's coordinates for IME positioning
  in the desktop compositor. Android must likewise treat position and glyph
  visibility independently when providing native IME cursor anchors. Retain
  the upstream main/alternate-screen regression during shared extraction.
- `ca7eb7e` moves native host upgrades into an independent updater so they can
  finish after their originating Session ends. Keep this desktop/local process
  lifecycle owner outside `zterm-client` and the Android dependency graph.
  Android's existing exact-Session recovery/ended behavior applies after a host
  update; never recreate or select a replacement Session by name.
- The range has no changes under `crates/core`, `crates/proto`, `proto`, or
  `crates/daemon/src/client`, nor in the inspected pairing/broker entry files.
  The selected v2 protocol, client extraction boundary, Rust/Iroh pins and
  Kotlin/Compose/UniFFI architecture therefore remain applicable. Read the
  changed IPC/distribution/lifecycle specs before their affected code is edited.
- Derive Android `versionName` from the current workspace version as planned;
  do not hardcode the original `0.1.22` research baseline.

These are compatibility refinements of the approved scope, not a new product
decision. The user's 2026-09-07 implementation approval remains valid.

## Host validation supplied by the user

The user explicitly reported:

> zterm 在 mac 和 linux 上的基础功能已验证完毕，你可以用 zterm 连接到 mac 本地，用 zterm connect dev 连接到远程 linux 主机

Accept this as user-supplied completion of the existing host baseline for
Android development. Do not block the App on repeating an entire desktop
acceptance campaign. The report does not supply separate direct/relay route
observations, exact tested daemon revisions, or a six-row fixture log; do not
invent these details or mark independently observed runtime rows passed.
The existing migration task records this distinction next to Phase 15.2.
Android direct/relay, shared-client regression, and phone acceptance remain
their own future checks.

Read-only checks performed in this worktree on 2026-09-07:

| Command | Observed result | Evidence boundary |
| --- | --- | --- |
| `zterm --version` | Installed local CLI reports `0.1.24` | Does not establish remote daemon version |
| `zterm session list local` | Exit 0; no running Sessions | Local control access succeeds; no terminal created |
| `zterm session list dev` | Exit 0; `main` is attached | Authorized remote list succeeds; route not measured |

Development connection entries are `zterm` / `zterm connect local` on macOS and
`zterm connect dev` for the Linux host. The already-attached remote `main`
Session was not taken over, sent input, detached, closed, or updated in this
refresh. Use task-owned Sessions for subsequent interactive regression work.

## Current execution checkpoint

PRD/design/implementation plan approval and the user-confirmed host prerequisite
are recorded. No further product approval is needed for this baseline refresh.
The current turn's workflow instruction still requires planning, so this
checkpoint does not run `task.py start` or introduce Android product code.
The next implementation step remains the narrow native bridge/build gate in
Phase 1, after activation and `trellis-before-dev` context loading.
