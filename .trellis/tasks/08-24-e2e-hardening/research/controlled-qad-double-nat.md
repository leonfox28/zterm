# Controlled QAD double-NAT evidence

Date: 2026-08-31

## Question

The retained Foundation network Gate could prove direct transport only after
injecting `Builder::external_addr`. Its official-n0 product case stayed on the
Relay inside the nested Colima/Patchbay/TUN topology, so that result did not
separate these possibilities:

- official QAD/UDP did not produce a reflexive candidate;
- Iroh did not exchange or try automatically discovered candidates;
- the outer Colima/TUN mapping was destination-dependent or could not hairpin.

The M10 extension adds one controlled case without changing the production
profile or adding a product/test socket override.

## Fixture

Case D adapts Iroh 1.0.3's own Patchbay relay pattern. A disposable Patchbay
device on the simulated Internet runs `iroh_relay::server::Server` with:

- a self-signed HTTPS Relay on an ephemeral port;
- QAD on an ephemeral UDP port;
- one lab-only `relay.test` DNS record;
- test-only insecure CA verification at the two endpoint builders.

The two endpoints remain behind independent `Nat::Home` routers. The dial
address contains only endpoint identity plus the controlled Relay route. It
does not contain `Builder::external_addr`, a configured direct candidate, a
raw address, or a Zedra Relay. The connection must start on Relay and then
select Direct automatically.

The fixture records only content-free evidence:

- QAD UDP v4/v6 success flags;
- presence of global v4/v6 without their values;
- whether the mapping varies by QAD destination;
- candidate counts and source buckets without addresses;
- Relay/Direct path-event kinds and three independent stream completions.

Iroh's public `EndpointAddr` erases `DirectAddrType`. The Gate labels a
candidate `NetReportGlobalMatch` only when it exactly matches the public
net-report global address in memory. That proves QAD observed the same address,
but it does not guess the erased internal type because a port-mapped address can
have the same value. All other non-configured candidates remain
`PublicApiUnclassified`. No endpoint ID or IP address is printed in the
successful evidence record.

## Result

`sh tests/foundation/network-gate.sh` completed with
`NETWORK_GATE=GO_WITH_DEFERRED_ADDRESS_DISCOVERY` on the second bounded run:

| Case | Discovery/control | Redacted net-report evidence | Selected path |
| --- | --- | --- | --- |
| A official product | official-n0, no injected candidate | both endpoints: UDP v4 true, global v4 present, one net-report-global match, mapping varies by destination | Relay -> Relay |
| B injected control | raw UDP passed, one Config candidate per endpoint | both endpoints also observed official QAD/global v4 | Relay -> Direct |
| C official fallback | non-DNS UDP blocked | UDP v4 false, no global v4, no QAD candidate | Relay -> Relay over WSS/TCP |
| D controlled QAD | local Relay/QAD, no injected candidate | both endpoints: UDP v4 true, global v4 present, one net-report-global match; variation unknown (`None`) with one controlled QAD destination | Relay -> opened Direct -> selected Direct |

Each completed case exchanged three independent bidirectional streams. Case D
therefore proves that Iroh 1.0.3 automatically discovers, exchanges, probes,
and selects direct candidates across the same inner Patchbay Home x Home NAT
model. Case A proves that official QAD was reachable in this run; its failure
was not caused by missing QAD/global-v4 discovery. Official QAD observations
varied by destination. Case D has only one controlled QAD destination, so it
does not measure whether that mapping varies; it proves the narrower fact that
the inner mappings learned from controlled QAD promoted to Direct. In this
nested lab, outer Colima/TUN endpoint-dependent mapping or missing hairpin
behavior remains the lab-specific explanation to test, not a missing zterm QNT
candidate-exchange path.

This does not prove that the user's company/home topology has the same NAT
behavior, and it does not replace the M10 two-real-network official-n0
acceptance. No production Relay/profile behavior was changed.

## Verification record

- First full run: Cases A, C, and D completed with the results above; retained
  Case B's pre-Iroh raw reflector timed out once. No code or assertion was
  weakened for that fixture-only failure.
- Second full run: all A/B/C/D cases passed; total network case time was about
  34 seconds after compilation.
- Independent checker runs: all A/B/C/D cases passed in 34.18 and 33.40
  seconds; Case A again reported destination-varying official QAD mappings,
  while Case D's single controlled destination reported variation as unknown
  (`None`).
- Linux unit verdicts: 2 passed, covering the accepted A/B/C/D matrix and hard
  failure of B, C, or D.
- Linux Clippy for `zterm-daemon --test iroh_network_gate` with `-D warnings`:
  passed.
- Workspace formatting check: passed after formatting.

The runner removed its named disposable container after every failed,
successful, and independent-review run.
