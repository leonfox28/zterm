# Research: Official n0 versus optional self-hosted Relay acceptance

- Query: Audit the user's correction that M5-M6 production transport uses Iroh's official n0 Relay and that `relay.zenithconsulting.cn` must not be a required transport-auth gate; classify commits `5e021cd`, `bf3d313`, and `1d90b55`.
- Scope: mixed (repository contracts/tests plus read-only GitHub commit metadata)
- Date: 2026-08-23

## Findings

### Decision

The user's correction is right. The task's production profile is the pinned Iroh
1.0.3 official n0 production map; the self-hosted Relay is optional and isolated
(`prd.md:17-19`, `prd.md:43-45`, `prd.md:171-172`). The parent plan also says the
Phase Zero self-hosted deployment had already supplied its one-time public
acceptance and must not be dragged back to the end of Phase One
(`../08-20-cross-platform-relay-terminal-mvp/implement.md:44-59`).

The new manual self-hosted workflow is therefore not an M5-M6 completion gate.
The smallest correction is to remove the workflow and its workflow-specific
static/spec/docs additions. The already-existing
`tests/relay/public-handshake.sh` remains the optional deployment's direct
post-update tool; a hosted wrapper can be reconsidered in a separate Relay
operations task if it acquires a current consumer.

### Existing official n0 evidence

The ordinary Linux CI runs must **not** be described as public-n0 runtime tests:

- `connection_broker` binds loopback endpoints with `RelayMode::Disabled`
  (`crates/daemon/tests/support/network_fixture.rs:46-55`).
- the two-process production pairing gate is also loopback-only with
  `RelayMode::Disabled`
  (`crates/daemon/src/pairing_service/multiprocess_test.rs:331-340`).

Those Linux runs do prove the new M5-M6 broker, ALPN, pairing, stream, and
authorization code on real Iroh endpoints. Current `iroh_profile_gate` separately
proves that the exact builder used by production still contains the four official
n0 entries, QAD, production lookups, relay-only publication, and both product
ALPNs (`crates/daemon/tests/iroh_profile_gate.rs:27-59`, `:116-139`, `:170-219`).

Actual official Relay/path evidence was already accepted by the completed
Foundation Gate, which is a dependency of this child. Its disposable Linux lab:

- bound `InfrastructureProfile::zterm()` and waited for an official production
  home Relay (`crates/daemon/tests/iroh_network_gate.rs:520-550`);
- required each endpoint's home Relay to belong to the official map and each
  case to begin on Relay (`:917-946`);
- blocked all endpoint non-DNS UDP in Case C and completed three encrypted
  bidirectional streams over official WSS/TCP Relay
  (`docs/foundation-gate.md:82-105`, `:119-132`).

Therefore the correct M5-M6 evidence is compositional: retained official-n0
Foundation runtime/path evidence + current exact production-profile regression +
current Linux real-Iroh M5-M6 tests. A new handshake against the optional
`relay.zenithconsulting.cn` adds no required product evidence. Its HTTP 403 is an
optional deployment incident, not a transport-auth blocker.

### Commit/file disposition

Commit file lists and patches were read through the GitHub commit API; no local
git operation or network/Endpoint test was run.

| Commit / file | Action | Reason |
| --- | --- | --- |
| `5e021cd` `.github/workflows/public-relay-acceptance.yml` | Revert/delete | Added solely to manufacture a new hosted self-hosted gate; the direct probe already owns optional deployment acceptance. |
| `5e021cd` `.trellis/spec/backend/relay-deployment.md` | Revert the manual-workflow additions | Keep the pre-existing official-default/optional-self-hosted and direct post-update handshake contracts. |
| `5e021cd` `.trellis/spec/backend/transport-auth.md` | Rewrite | Retain the useful cross-UID Linux contract; remove the workflow signature, manual public Relay gate/matrix row, and dispatch command. |
| `5e021cd` `docs/relay.md` | Revert the workflow paragraph | Keep the pre-existing direct `public-handshake.sh` optional runbook. |
| `bf3d313` `.github/workflows/public-relay-acceptance.yml` | Revert with workflow deletion | `/healthz`, `/generate_204`, and `/ping` diagnostics served only the unnecessary wrapper. |
| `bf3d313` `tests/relay/static.sh` | Revert workflow-specific assertions | They statically enforce a workflow with no M5-M6 consumer. |
| `bf3d313` `.trellis/spec/backend/relay-deployment.md` | Revert workflow HTTP-preflight additions | Do not promote optional deployment diagnostics into the product acceptance contract. |
| `bf3d313` `.trellis/spec/backend/transport-auth.md` | Revert public HTTP/manual-gate additions | They are out of transport-auth scope. |
| `bf3d313` `docs/relay.md` | Revert workflow-specific HTTP paragraph | The optional direct health/handshake runbook already exists later in the document. |
| `1d90b55` `implement.md` | Rewrite | Retain cross-UID run evidence; remove the GitHub workflow command and 403 blocker; close Step 9 with the compositional official-n0 evidence. |
| `1d90b55` `research/public-relay-403.md` | Revert/delete | Truthful optional-infrastructure incident, but misplaced as this child's blocker; this audit preserves why it is removed. |
| `1d90b55` `research/step10-quality-review.md` | Rewrite | Retain green CI and cross-UID evidence; remove F7/blocker language and map Relay/path acceptance to the inherited official Foundation evidence. |

One earlier planning line also needs correction even though it predates those
three commits: `design.md:584-588` and `implement.md:391-416` turned a fresh
self-hosted handshake into task-local evidence. Replace that with the
compositional official-n0 evidence above. Do not change the PRD's official-default
contract.

### Genuinely remaining gates

No public/self-hosted network gate remains for M5-M6. After the scope correction,
the remaining work is bookkeeping and regression verification:

1. correct the task/spec/docs artifacts listed above;
2. run the normal non-public quality gate and obtain a green ordinary hosted CI
   head;
3. update the parent M5-M6 progress, validate, finish, and archive this child.

The real two-physical-network automatic address-discovery matrix remains deferred
to parent M10 (`docs/foundation-gate.md:119-132`); it is not a transport-auth
completion gate.

## Files Found

- `.trellis/tasks/08-22-transport-auth/prd.md` — official n0 is the product default; self-hosted is explicit and isolated.
- `.trellis/tasks/08-22-transport-auth/design.md` — task design plus the later-overstrict disposable-Relay evidence line.
- `.trellis/tasks/08-22-transport-auth/implement.md` — current misplaced manual workflow/403 blocker.
- `.trellis/tasks/08-22-transport-auth/research/step10-quality-review.md` — clean matrix and currently incorrect remaining-gate conclusion.
- `.trellis/tasks/08-22-transport-auth/research/public-relay-403.md` — optional self-hosted 403 incident mislabeled as task blocker.
- `.trellis/tasks/archive/2026-08/08-21-foundation-gate/{prd.md,implement.md}` — approved official-n0 prerequisite and retained A/B/C gate.
- `docs/foundation-gate.md` — authoritative accepted official Relay/path report.
- `crates/daemon/tests/iroh_network_gate.rs` — official n0 external network gate implementation.
- `crates/daemon/tests/iroh_profile_gate.rs` — current exact production profile and two-ALPN regression.
- `crates/daemon/tests/support/network_fixture.rs` — proof that ordinary Linux broker evidence is loopback/Relay-disabled.
- `crates/daemon/src/pairing_service/multiprocess_test.rs` — proof that Linux production pairing evidence is loopback/Relay-disabled.
- `docs/phase-zero-verification.md` — earlier optional self-hosted authenticated handshake evidence.

## Code Patterns

- Production owner obtains its Relay map from `RelayMode::Default`, never copied
  URLs (`crates/daemon/src/transport.rs:87-97`, `:143-160`).
- External route hints become one relay-only `EndpointAddr` and do not mutate the
  configured map (`crates/daemon/src/route.rs:24-48`, `:92-140`).
- Product authorization is receiver-owned and checked independently of the path
  (`crates/daemon/src/connection_broker.rs:2106-2125`).

## External References

- GitHub commits: `5e021cd` (`ci: add manual public relay acceptance`),
  `bf3d313` (`ci: diagnose public relay surface`), and `1d90b55`
  (`docs: record public relay acceptance blocker`).
- Pinned transport/runtime version: Iroh `1.0.3`.

## Related Specs

- `.trellis/spec/backend/relay-deployment.md`
- `.trellis/spec/backend/transport-auth.md`
- `.trellis/spec/guides/evidence-driven-simplicity.md`

## Caveats / Not Found

- A green ordinary Linux CI run alone is not official-n0 runtime evidence; the
  accepted Foundation Gate is required for that claim.
- This audit did not run any network test, bind any Endpoint, or inspect/change
  Cloudflare, OpenResty, 1Panel, or server state.
- Per the research-agent isolation contract, `implement.jsonl` and `check.jsonl`
  were not loaded even though the dispatch requested them; the PRD, design,
  implementation plan, applicable specs, code, prior research, and exact commit
  patches were inspected directly.
