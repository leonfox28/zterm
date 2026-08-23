# Step 9 multi-process production PairingService gate

## Result

The remaining implementation gap now has a Linux-only automated target:

```text
pairing_service::multiprocess_test::
two_process_production_pairing_service_is_directional_and_reuses_one_endpoint
```

The parent lib-test process self-spawns exact ignored host and controller helper
tests as two distinct OS processes. Each helper owns task-private `UserPaths`, a
committed `identity.key`, one `StoreActor`, one `AuthorizationRegistry`, one
`DeviceDirectory`, one loopback-only Iroh `Endpoint`, one `ConnectionBroker`,
and the production `PairingService` with its real `BrokerPairTransport`.

The host creates a real one-time ticket. The controller runs the complete
pair-ALPN Begin/Challenge/Proof/Accepted exchange, then performs the required
normal-ALPN confirmation. The gate checks that:

- the host durably and in memory authorizes only the controller;
- the controller durably records only the host as an outbound known device;
- the controller does not acquire reverse inbound authorization;
- after pair plus normal confirmation, each broker converges to exactly one
  normal candidate/primary with no active business stream or extra pair entry;
- each child retains the same sole Endpoint identity and bound socket through
  pair plus normal confirmation;
- the direct loopback address remains test-only and does not become a verified
  Relay cache entry.

The ticket crosses only a Unix control socket below an explicit `0700`
task-private root; each socket is explicitly `0600`. It is never placed in
argv, an environment variable, a file, panic output, or a test snapshot. Only
the child role and control-socket path are passed through the test process
environment. Child stdout/stderr are connected to the null device rather than
captured, so a failed helper cannot copy bearer-adjacent diagnostics into the
parent result. The parent gives both helpers one common exit deadline; on any
parent unwind or exit timeout, its guards send kill and poll for reap under a
bounded grace period.

## Safety and platform boundary

The parent is ignored on every non-Linux Unix target, and the parent plus both
helper entrypoints call an explicit Linux guard before any Endpoint bind. The
Endpoint profile disables Relay and uses one IPv4 loopback transport; the
ticket retains a syntactically valid Relay hint, but the existing test-only
broker route selects the task-private direct address without DNS or public
network access.

On the macOS development host, only these non-network commands were run:

```sh
cargo test -p zterm-daemon --lib --all-features --no-run
cargo clippy -p zterm-daemon --lib --tests --all-features -- -D warnings
cargo test -p zterm-daemon --lib --all-features -- --list
```

Linux must execute:

```sh
cargo test -p zterm-daemon --lib --all-features \
  pairing_service::multiprocess_test::two_process_production_pairing_service_is_directional_and_reuses_one_endpoint -- --exact
```

## Deliberate irreducible boundary

The helpers do not invoke the production hidden CLI, `run_daemon`, or
`NetworkStartup`. Doing so with two identities on one CI account would require
production state/socket override arguments, which this task explicitly
forbids, and the public M8 pair command does not yet exist. Instead the child
processes compose the production identity/store/auth/directory/broker/pairing
owners through private lib-test access and use a deliberately small test-only
ALPN accept router. This gate is therefore truthful evidence for multi-process
production `PairingService`/`BrokerPairTransport` and real Iroh ALPN behavior,
not evidence for the future public CLI or full daemon/network lifecycle
surface.
