# Focused local-daemon contract context

Extracted for this task on 2026-09-05 from
`.trellis/spec/backend/local-daemon-ipc.md`. That source exceeds the 32 KiB
per-file context injection limit; inject this focused summary and read the
referenced source sections when changing their owners. This is research context,
not a replacement specification. Task PRD/design override explicitly selected
old public UX contracts; implementation must update the authoritative spec.

## Ownership and boundaries

- Source lines 151–187: client/ipc owns unary messages; operations owns command
  use cases and lifecycle. CLI may receive typed results but not UserPaths,
  socket paths, database, identity keys, endpoints, routes, or operation leases.
- Source lines 158–172: one installed executable, one daemon per effective
  user, hidden daemon entry, no supervisor or login/boot registration.
  lifecycle.lock serializes short launcher/setup operations; daemon.lock is
  lifetime ownership and the daemon never waits for lifecycle.lock.
- Same-UID socket authentication and strict bounded unary framing/EOF remain
  unchanged. Potentially blocking Session work belongs on the blocking worker
  owner, not inline on the current-thread async runtime.
- Source lines 180–187: bare invocation prints guidance before setup; after
  setup it creates/attaches local main. Status/doctor/logs/daemon status/stop,
  help/version and parse failure do not spawn. Setup/restart explicitly spawn;
  pair/device/Session operations may ensure after setup validation.

## Lifecycle and test requirements

- Source lines 578–591: stop success requires bounded full Session ownership
  release. Flush the response and close its socket before signaling listener
  exit. Cleanup deadline/failure, response-flush failure or dropped caller
  must not orphan children or report false completion. Failed cleanup retains
  listener/socket ownership for status and retry.
- Source lines 604–612: local readiness/status/stop/update preflight do not wait
  for Iroh, DNS, Relay or Internet. Restart waits for readiness to disappear,
  socket absence and daemon-lock release; socket absence alone is insufficient.
- Source lines 621–641: detached launch redirects stdio and uses stable account
  home cwd. Running observations come from IPC; stopped setup may validate
  disk state only when no StoreActor owns it. Doctor never spawns.
- Source section 6 and docs/development.md: use test-private effective-user
  paths and existing local-only process/PTY fixtures on macOS. Do not operate
  real account identity, run real Iroh acceptance, or infer hosted Linux/signed
  Release evidence from a native development gate.

## Clauses intentionally changed by the approved task

- Lines 177 and 638: remove public human/JSON alternatives; preserve typed
  observations and hidden distribution verification.
- Lines 613–617: replace public force with English conditional y confirmation
  and -y/--yes. The selected C1 also unifies session close/device revoke/reset/
  uninstall; reset/uninstall combine Session and deletion impact once and
  remove their public force flags. Deletion still needs approval with no Sessions.
- Add the atomic idle-stop admission and new-version update startup contracts
  from task design. The existing shutdown server currently ignores the force
  wire field, so a client-only confirmation fix is insufficient.
