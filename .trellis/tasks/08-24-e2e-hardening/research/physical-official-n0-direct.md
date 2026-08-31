# Physical official-n0 Direct acceptance

Date: 2026-08-31

## Acceptance topology

- Client: installed zterm 0.1.7 on macOS, connected through a cellular hotspot
  with no VPN or user-configured proxy in the acceptance run.
- Host: installed zterm 0.1.7 on Debian, private IPv4 behind a MikroTik
  RouterOS 7.21.5 PPPoE router with a public IPv4 and endpoint-independent UDP
  source/destination NAT.
- Infrastructure: the pinned official-n0 profile. No injected direct address,
  custom Relay, or test binary was used.

Public addresses and endpoint IDs are intentionally omitted.

## Initial symptom and false leads

The connection authenticated and carried a terminal over Relay but never
selected Direct. VPN access made Direct work, while the cellular-to-home path
remained Relay-only. The controlled Patchbay Gate had already proved that Iroh
1.0.3 could exchange QAD-discovered candidates and promote Relay to Direct, so
the remaining question was in the physical network path.

The home router had working endpoint-independent UDP NAT. A temporary
host-scoped `ein-dnat` forward accept rule received zero packets during the
cellular test and still had zero packets after the successful Direct stream.
It did not participate in the accepted path, so the evidence rejects the
suspected inbound firewall drop. The temporary rule was removed after the
successful acceptance run.

## Root cause

The Debian endpoint's active IPv4 UDP source port was correlated with RouterOS
connection tracking. All four current QAD flows targeted addresses inside
`198.18.0.0/15` on UDP 7842. They accumulated outbound bytes, received zero
reply bytes, and never acquired the router's WAN source-NAT address.

Those addresses were OpenClash/Mihomo Fake-IP results for the official n0
Relay names. The QAD packets were sent toward the transparent-proxy path on the
LAN, not toward official QAD through PPPoE. Older connection records from
other daemon lifetimes showed real public UDP 7842 destinations with replies,
which independently demonstrated that the ISP and RouterOS could carry QAD.

## Repair

The deployment was changed so `iroh.link` and its subdomains return real DNS
answers, and UDP from the Debian endpoint bypasses OpenClash. The UDP bypass is
required for both QAD and the dynamic peer port; a destination-port-7842 rule
alone would not cover the final peer path.

After applying and restarting the proxy configuration,
`getent ahostsv4 euc1-1.relay.n0.iroh.link` returned a real public A record
rather than `198.18.0.0/15`. Both zterm daemons were restarted for a clean
candidate report and connection.

## Result

During one cellular-to-home connection, both installed peers reported:

```text
authenticated connections: 1
primary connections:       1
active streams:            1
direct paths:              1
relay paths:               0
```

An interactive terminal subsequently attached and worked. This completes the
physical official-n0 Direct claim for the tested IPv4 topology. It also
explains why VPN worked: the VPN supplied a mutually reachable private path
that did not depend on the intercepted QAD flow.

## Remaining acceptance items

- Three initial `zterm connect simplus` attempts returned
  `operation_outcome_unknown`; the host nevertheless had one live Session and
  both peers already had a Direct transport. A later attachment succeeded.
  Treat this as a separate initial-attachment/correlation reliability defect,
  not a NAT or Direct failure, and reproduce it before closing M10.
- Add a bounded, redacted user-facing diagnostic for fake-IP/QAD interception
  only if it can use supported production APIs without leaking addresses or
  enabling Iroh's unstable net-report feature in the normal dependency graph.
