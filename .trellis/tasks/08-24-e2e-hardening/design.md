# M10 end-to-end and release acceptance design

## 1. Outcome

M10 proves the first-stage product using the same immutable, signed GitHub Release and official installer that the user will run. It does not create a parallel test product, add public state/socket overrides, or substitute compile-only targets for runtime evidence.

The release is not accepted because one broad test command is green. Each externally visible claim has one named evidence owner, and the final checklist links directly to that result.

## 2. Evidence layers

```text
pure/domain tests
  -> local IPC + PTY + process tests
  -> retained real-Iroh transport gate on hosted Linux
  -> installed-binary isolated network lab
  -> installed-binary real-device/two-network run
  -> user final installer acceptance
```

Each layer proves only what it can observe:

- pure/domain tests own replay, revision, bounds, parser, and authorization ordering;
- local tests own same-UID, daemon, PTY, raw terminal, signals, and local continuation;
- `two_daemon_transport` owns retained Endpoint pair/normal ALPN and primary reuse over Linux loopback;
- the installed-binary lab owns OS processes, installer state, network interruption, relay/direct routing, and packet/log observations;
- the real-device run owns physical-network discovery/path behavior and human terminal interoperability;
- the user's final pass owns the documented installation and primary workflow as experienced by an end user.

No new daemon-like in-process harness is added merely to duplicate the installed-binary layers.

## 3. Current M7–M8 handoff

The `remote-cli` child records GitHub run `32725142928`, including hosted Windows shared compilation and the Linux x86_64 execution of `two_daemon_transport`. Its product code, local/PTY evidence, docs, and CI repair are then complete and can be archived.

Parent M7–M8 remain release-acceptance pending until M10 runs two installed `zterm` OS processes through the public pair/device/session/connect surface. This is an explicit ownership transfer, not a claim that remote Session runtime was already executed by the retained transport test.

## 4. Installed artifact matrix

M10 consumes one M9 signed candidate tag. Every hosted job installs through `install/install.sh --version <tag>` into an ephemeral ordinary account/destination, then confirms:

- no state or daemon before setup;
- exact binary target/build/version and managed permissions;
- setup/ensure/single-instance and local Session behavior;
- supported macOS arm64/x64 and glibc Linux arm64/x64 execution;
- unsupported musl/NixOS/low-glibc rejection before artifact download;
- update/activation rollback/uninstall fixtures where destructive behavior remains task-private.

The matrix downloads Release assets, not Actions artifacts. It retains hashes, manifest signature status, and run URLs as evidence.

## 5. Deterministic Linux network lab

A root-owned test orchestrator may create task-private Linux users, homes, namespaces, veth/NAT rules, packet capture, and an upstream Iroh relay fixture. The installed `zterm` binaries themselves receive no state/socket/identity override and run as the created ordinary users.

Scenarios are intentionally minimal and distinct:

1. direct-capable path and observed promotion;
2. relay-only path with direct blocked;
3. active connection interruption and restoration with the same Session/PTY;
4. DNS/Pkarr unavailable with valid cache, then cache unavailable with explicit failure;
5. two simultaneous Sessions over one primary with independent streams;
6. directional pairing, authorization, revoke, and reconnect rejection;
7. remote-created Session continued through host same-UID local attach and returned to remote control.

The fixture explicitly labels QAD state, route source, relay identity, and whether a path is loopback/lab/official. A local upstream relay proves relay protocol behavior but not official-n0 service availability. Packet capture searches unique synthetic terminal sentinels in plaintext and correlates encrypted traffic/path events without logging ticket or terminal payloads.

## 6. Real-device and two-network acceptance

After automation and the signed candidate are green, the main session reaches one user checkpoint. The user provides or selects two supported macOS/Linux devices/accounts and two independent networks; no SSH credential or PairTicket enters Git, task artifacts, argv, environment variables, or assistant-visible logs.

Both devices install the exact candidate via the official HTTPS installer. The run covers:

- first setup and distinct identities;
- one-way pairing and direction denial, followed by an independent reverse pairing;
- `main` plus a named second Session, remote create/list/rename/attach/close;
- long-running task, CLI detach/network switch, reconnect to the same SessionId/process/cwd/screen;
- two Sessions sharing one primary without head-of-line interference;
- remote controller versus host-local ordinary attach/takeover in both directions;
- revoke with PTY preservation and subsequent authorization rejection;
- direct path when available and official-n0 relay fallback when direct is deliberately unavailable;
- tmux and the fixed Herdr revision through the same generic terminal path.

If a physical network cannot produce one desired path, record the observed topology and move only that exact scenario to another real network; do not add product/test hooks to force a label.

## 7. Terminal and security matrix

Existing authoritative corpora remain owners for terminal model/revision semantics. M10 adds only release-blocking gaps:

- bounded fuzz targets for frame decoder, pairing ticket, prefix parser, and terminal byte ingestion using fixed toolchain/time budgets;
- malformed/oversize/stalled stream isolation and resource caps;
- OSC 52, DCS/APC, unknown graphics, alternate screen, Unicode width, resize, bracketed paste, tmux, and Herdr black-box behavior;
- local peer UID, symlink/permission, unauthorized Endpoint, revoke race, and secret/log/error redaction;
- packet/relay log review showing no synthetic terminal plaintext or bearer secret.

Fuzzing is a bounded release gate, not an open-ended attempt to prove absence of bugs. A failure becomes a concrete fix; a green run does not trigger additional parser variants.

## 8. Acceptance record

One task-local evidence document maps every parent PRD A–E item to:

- exact release tag and commit;
- artifact target/digest and installer result;
- test or manual scenario name;
- GitHub run URL or redacted manual record;
- observed path/profile and explicit evidence limit.

Unchecked items stay unchecked. Loopback, namespace, official-n0, optional self-hosted, public Internet, and physical-network results are never conflated.

## 9. Failure and cleanup

- Each hosted/lab run owns a task-private prefix and deletes only that prefix after bounded daemon/session shutdown.
- Failure retains content-free diagnostics and the minimum non-secret artifact needed to reproduce it.
- A failed network scenario does not trigger extra relays, public workflows, or product state overrides.
- A failed signed candidate is superseded by a new version/tag; immutable Release assets are never replaced.
- User devices are uninstalled/reset only with the existing explicit confirmation contract.

## 10. Completion boundary

First stage completes only after M1–M10 and PRD A–E have direct evidence, the signed installer workflow is the path actually used, independent review is green, and the user completes the final workflow. Android, Windows runtime, GUI, iOS, autostart, crash-persistent PTY, automatic updates, transcript, file transfer, and Agent-specific behavior remain later phases.
