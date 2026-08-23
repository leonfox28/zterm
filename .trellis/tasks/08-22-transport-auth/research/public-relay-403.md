# Public Relay 403 break-loop analysis

- Date: 2026-08-23
- Public target: `https://relay.zenithconsulting.cn`
- Execution boundary: GitHub-hosted Ubuntu only; no Endpoint or public-network
  probe was run on the local macOS host

## Bug Analysis: handshake-only acceptance hid the public failure layer

### 1. Root Cause Category

- **Category**: D/E — test coverage gap plus implicit assumption.
- **Specific cause**: the first manual workflow ran only
  `Endpoint::online()`. Iroh does not publish a home-relay status until its
  HTTPS `/ping` latency probe selects a relay, so an HTTP/proxy rejection before
  selection appeared only as an empty status list after 45 seconds. The
  workflow implicitly assumed that this timeout distinguished Relay protocol
  failure from public HTTP-surface failure; it did not.
- **External acceptance state**: the improved hosted run received HTTP 403 from
  public `/healthz` before binding an Endpoint. This proves the current blocker
  is on the public HTTPS/proxy/access-policy path, not in the task-owned Iroh
  state machine. The available evidence does not distinguish Cloudflare from
  OpenResty/origin policy, so no provider-specific root cause is claimed.

### 2. Why the first attempt was insufficient

1. [`32612287182`](https://github.com/leonfox28/zterm/actions/runs/32612287182)
   on `5e021cd` reached the real Iroh probe but timed out with
   `no home relay status was reported`. The error was accurate but did not
   identify the failed public path.
2. Ordinary CI was intentionally network-independent. Its green Relay static
   job could prove probe/config/workflow shape, but could not observe a public
   WAF or reverse-proxy response.
3. The fix added bounded exact-status checks for `/healthz` (200),
   `/generate_204` (204), and `/ping` (200) before Endpoint bind. The reviewed
   rerun [`32613231264`](https://github.com/leonfox28/zterm/actions/runs/32613231264)
   on `bf3d313` then failed immediately and specifically:
   `/healthz returned HTTP 403; expected 200`.

### 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Runtime acceptance | Manual hosted workflow checks exact public health, captive-portal, and home-relay-selection HTTP paths before Endpoint bind | DONE |
| P0 | Bounded failure | Each HTTP request has a 15-second deadline, no redirect, and zero retries; the Iroh attempt remains one 45-second bounded attempt | DONE |
| P0 | Test coverage | `tests/relay/static.sh` enforces manual-only trigger, read-only permissions, timeouts, zero retries, exact paths/statuses, and reuse of the existing handshake probe | DONE |
| P0 | Documentation | Backend Relay and transport/auth specs record `/ping` as part of the public Iroh acceptance surface | DONE |
| P1 | External owner evidence | Inspect public proxy/WAF and origin logs for the 403, then change only the identified external owner with explicit authority | BLOCKED ON EXTERNAL ACCESS/AUTHORITY |
| P1 | Final acceptance | Re-dispatch the reviewed workflow after the external 403 is fixed and record one successful run | PENDING |

### 4. Systematic Expansion

- **Similar issues**: any Iroh `Endpoint::online()` acceptance can hide a
  failure in `/ping`, because home-relay selection precedes the authenticated
  WebSocket connection.
- **Design improvement**: keep transport protocol acceptance behind explicit
  public-surface checks rather than adding retries or interpreting an empty
  watcher as a protocol-specific error.
- **Process improvement**: public infrastructure evidence stays in a dedicated
  manual workflow. Push/PR CI verifies its policy and code shape without
  depending on public availability.
- **Knowledge gap closed**: `/healthz` and `/generate_204` alone are not enough
  for Iroh home-relay selection; `/ping` is also required.

### 5. Bayesian update

Before the hosted run, plausible causes were public proxy/service failure
(45%), hosted-runner/WAF incompatibility (35%), and probe/workflow defect (20%).
The first timeout weakly favored the first two. A direct HTTP 403 before
Endpoint bind, together with a green seven-job repository matrix, reduces the
task-owned probe/state-machine hypothesis below 5% and places more than 95% of
the probability on the external public HTTPS/access-policy layer. Provider
ownership within that layer remains unresolved and must be determined from
proxy/origin evidence, not guessed from the status code alone.

### 6. Knowledge Capture

- [x] Updated `.trellis/spec/backend/relay-deployment.md`.
- [x] Updated `.trellis/spec/backend/transport-auth.md`.
- [x] Updated `docs/relay.md` and the Relay static policy gate.
- [x] Recorded exact failed hosted runs without marking acceptance complete.
- [x] Checked for `src/templates/markdown/spec/`; this repository has no
  corresponding template copy to synchronize.
- [ ] Record the external owner/fix and one successful manual hosted run before
  archiving this task or marking parent M5-M6 complete.
