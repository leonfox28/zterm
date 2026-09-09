# Parent execution and integration plan

## Phase transition

- [x] Product decisions converged; input pause and all listed refinements accepted.
- [x] Parent and three child PRD/design/implementation artifacts prepared.
- [x] Present final artifact summary; review approved by the user (开始吧).
- [x] Activate 09-09-file-upload-service first. Parent remains integration owner.

No implementation/check agent dispatch. The main session loads trellis-before-dev
before edits and trellis-check after implementation. JSONL dispatch manifests are
not an inline gate. Do not activate all children merely because they exist.

## Ordered delivery

1. Shared service child: core/proto contract, transport capability propagation,
   host authority/storage, client transfer and focused deterministic validation.
2. Desktop child: source backend, local command actions, input pause/cancel and
   right-aligned status rendering; integrate with the validated shared service.
3. Android child: retained bridge operation, source staging/system pickers/dialog,
   input pause and lifecycle fencing; generate bindings through existing tooling.
4. Parent integration: cross-client/real-host acceptance, docs/spec consistency and
   the project's applicable final quality gate. Do not infer runtime from a build.

Child 2/3 depend on shared service readiness; child ordering is recorded in each
plan. If parent wire/UX behavior changes materially, update all affected artifacts
and repeat final review before implementing the changed behavior.

## Cross-child acceptance checklist

- [ ] Desktop copied image/PDF/arbitrary file and raw screenshot produce readable
  byte-correct host files. Android Image/File pickers do the same, single selection.
- [ ] Exactly 50,000,000 bytes succeeds; cap+1 fails before final publication.
- [ ] Ctrl+V/local behavior retained; prefix+v starts and prefix+c cancels. Input
  attempted during pause is not replayed; output continues through progress updates.
- [ ] Path is inserted once without Enter after completion and under original
  authority. No duplicate transfer from repeated gestures or Activity recreation.
- [ ] Real cancellation/disconnect/takeover/nav/commit races expose correct outcomes.
- [ ] Completed files survive detach/end; same names cannot overwrite; no retained
  quota/age cleanup. Old host/broker leaves ordinary terminal input usable.
- [ ] Desktop narrow/Unicode/alternate/resize and Android IME/inset/dialog layout
  runtime evidence recorded. Cloud/unknown size gets explicit preparation.
- [ ] Remote Codex/Pi can access image files; record attachment UI separately.
  PDF/binary transfer evidence is independent of AI format interpretation.

Use disposable host Sessions and task-owned files/images only. Do not modify user
clipboard for tests without isolating/restoring it, or submit unintended AI prompts.
Retain explicit pending rows for unavailable OS/device runtime environments.

## Verification commands

Run affected child checks first. At final integration, use repository quality
requirements (just check-fast / relevant workspace test suites and just android-check)
with current exact toolchain 1.98.0. See child plans for scoped commands. New
clipboard dependencies also require cargo +1.98.0 deny check and relevant supported
OS CI builds. Existing tests/terminal-dependency-policy.sh must keep Android free
of daemon/PTY/desktop clipboard dependencies.

Update backend core-wire-domain/shared-client/local-daemon-ipc/input-command/
effective-user-state and frontend android-app specs with the actual implementation.
Update user-facing remote/Android usage docs. Do not auto-publish/deploy this feature.

## Rollback points

Feature capability/dispatch is the host availability boundary. Reverting clients
must not delete retained files; reverting host capability disables new uploads
without changing old terminal kinds. Do not weaken transport/auth/input gates to
make integration pass. No successful upload cleanup is part of rollback.

## Current evidence

Implementation is complete across the three children. Workspace tests, Android
build/lint and real Android-to-host upload evidence are recorded in
`research/validation.md`. Unchecked platform/AI runtime rows above remain explicit
follow-up acceptance, not claims inferred from compilation. Final commit review is
pending workflow Phase 3.4 confirmation; no publication has occurred.
