# Implement worker brief: core + proto foundation

Active task: `.trellis/tasks/08-22-transport-auth`

Implement only Steps 0–2 of `implement.md`, with these ownership boundaries:

- root `Cargo.toml` / `Cargo.lock` only for the approved exact direct dependencies;
- `crates/core/**` and focused core tests/fixtures;
- `proto/zterm/v1/**`, `crates/proto/**`, and focused proto tests/fixtures;
- do not modify `crates/daemon/**`, `crates/cli/**`, public CLI commands, or Trellis specs/task artifacts.

You are not alone in the codebase. Preserve all existing/user edits, do not revert or rewrite unrelated files, and adapt to concurrent changes. Do not commit, push, or merge.

Required behavior is the approved PRD/design, especially:

1. Add transport-neutral alias, offer/nonce/secret, relay-hint, ticket, candidate-key, authorization-generation/snapshot, transport-limit, canonical-ticket/transcript and HMAC primitives to `zterm-core` without prost/Iroh/SQLite/CLI/OS dependencies.
2. Keep secret-bearing values redacted and zeroized. Use exact `ring = 0.17.14`, `base64 = 0.22.1`, `zeroize = 1.9.0` only at their real owners; do not add alternate crypto/random/base64 crates.
3. Extend stable `DomainErrorKind` only with the approved categories and retain round-trip codes.
4. Replace pairing placeholders and centrally register kinds 12–21 and 100–105. Add the ticket text and route-cache adapters in the proto crate, preserving the one existing frame decoder and global frame/control limits.
5. Enforce the tighter bounds before allocation/conversion. Canonical authentication bytes must not use prost serialization order.
6. Add deterministic golden/compatibility tests. Runtime-generated secrets must never appear in output or snapshots.

Baseline before your edits is green: source policy, workspace version, fmt, workspace Clippy/tests/docs, cargo-deny, and `iroh_profile_gate` all passed. Run all Step 1 and Step 2 focused gates before reporting. If an approved API detail is impossible against current code, report the exact conflict instead of widening scope.
