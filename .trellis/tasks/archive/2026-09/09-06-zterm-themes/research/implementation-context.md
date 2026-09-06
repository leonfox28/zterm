# Focused implementation context

The converged PRD, design and implementation plan are authoritative. Read protocol
contracts for exact subset. Earlier audit/reporting-only proposals are historical;
state/frontend research provides detailed anchors, not competing product choices.
This focused context avoids truncating the large IPC spec/research files during
injection. Read the cited source sections when touching each boundary.

## Decisions that supersede provisional research

- State owner: only engine-owned ZtermColorState; Alacritty grid/SGR remains sole.
  No upstream palette writes alongside this owner. Immediate bounded query replies.
- Reverse mode5: mapped default fg/bg and indices0↔7,8↔15, not blanket cell XOR.
  Unknown must retain source-aware fallback. Effective snapshots already map it.
- Effective cursor snapshot retains custom/native rendering provenance. Fixed or
  explicit Dynamic app cursor roles need software steady block even if their RGB
  happens to equal base. Keep CUP for IME; never emit physical color setters.
- Notifications follow changed external base/appearance, not own app palette
  setters/reset/pop. Unknown appearance cannot generate a fake997 preference.
- Stack explicit slots follow documented no-push/pop semantics; omitted/0 uses
  LIFO. Public-reference versus xterm source bookkeeping differs; contract/test
  choice is explicit in color-protocol-contracts.md.
- Compatibility: fresh canonical create209, attach323, update324; retire202/300,
  no aliases/fallback. No extra negotiation/semantic marker is needed. A field
  marker on old request kinds alone would allow old servers to create/start PTYs
  before client response rejection. Both fresh request kinds prevent that effect.
- One physical decoder lifetime across keyboard epochs; joined-reader bounded
  drain preserves reply framing while rejecting stale keys. A blind tcflush loses
  split OSC prefixes/terminators and cannot satisfy the new contract.
- Probe250ms/64KiB, one round, trailing DSR, one coalesced refresh. Missing boundary
  leaves consume-only late replies and prevents ambiguous new rounds. No local
  generation authenticates arbitrary delayed response bytes. No periodic polling.

## IPC/frontend source contract to read before edits

`.trellis/spec/backend/local-daemon-ipc.md` is too large for full injection.
Relevant existing contracts (baseline2026-09-06):

- Lines309–336: stateful create cancellation polls its original bounded result;
  preserve exact SessionId/CreatedSessionAttach/outcome-unknown behavior. Fresh
  attach/takeover stays input-fenced; replaceable current-controller resize is
  permitted during replacement sync. Color observations use the same authority.
- Lines316–320 and535–548 currently prescribe reader join/kernel flush/new epoch.
  This task intentionally changes the unparsed-flush mechanism; preserve its
  stale-key guarantees and exact-once complete-paste-on-resume behavior in tests.
- Lines412 onward: committed selection/source identity and range semantics. A
  color-only update preserves valid text selection; actual text changes still
  follow existing invalidation. Terminal replies do not alter selection/prefix.
- Lines441–469: one projected input mode owner, staged semantic candidate and
  one successful buffered write/flush before commit. Pinned-history text-only
  deltas currently do no row I/O; color-only changes now redraw retained rows.
  Never let a hidden live delta replace pinned history or bake RGB into cache.
- Lines529–558: immutable history anchors, Live/History/ResumePending, bounded
  exact-once replay and distinction between replacement sync and reconnect.
- Lines617 onward: strict attachment-frame allowlist needs structured update324;
  exact target-issued IDs and remote opaque tunnel stay unchanged.

## Session, PTY and user-state contracts to read before edits

- `.trellis/spec/backend/session-service.md`: single controller/pending takeover,
  operation-fingerprint replay, replacement snapshot readiness and current lease.
  Startup base must flow through default_main/create_reserved before model/spawn,
  and named create via SessionCreateRequest/fingerprint. Pending takeover stages
  only; install base and lease at commit, replacing stale prepared snapshots.
- `.trellis/spec/backend/pty-lifecycle.md`: ordered output drain/finalization,
  independent interruption/reap, safe child environment. No startup-query gap.
- `.trellis/spec/backend/terminal-input-commands.md`: sole Ctrl+] owner, exact
  prefix/key/paste behavior, no application-name or terminal-brand routing.
- `.trellis/spec/backend/effective-user-state.md`: effective-account ownership;
  tests use isolated temp state. Never edit or restart the user's ~/.zterm daemon.

## Detailed research navigation

- `color-state-architecture.md`: file/signature table; pinned Alacritty0.26/vte0.15
  limits; revision/update/reply ordering; initialization/takeover; colors in
  snapshots/history; corrected fresh-kind compatibility section and test anchors.
- `color-frontend-architecture.md`: exact stdin decoder and reader fence anchors;
  no-ID protocol limitation;2031 ownership; semantic/physical baseline distinction;
  history/selection and actual-glyph software cursor; focused fixture matrix.
- `color-theme-capability-audit.md`: original C01–C19 evidence and C13 example.
- `nested-application-theme-diagnosis.md`: screenshot attribution remains uncertain.

These sources are local research, not test pass evidence. Final native check is
just check; hosted Linux/release evidence stays with existing CI owners. Avoid
mirrored tests or broad repeated stress beyond distinct acceptance failures.
