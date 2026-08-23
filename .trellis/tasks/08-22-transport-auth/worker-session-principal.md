# Implement worker brief: Session principal ownership

Active task: `.trellis/tasks/08-22-transport-auth`

Implement only Step 4 of `implement.md`. You own:

- `crates/daemon/src/session.rs`;
- the minimal local adapter/service call sites needed to pass an explicit same-UID principal (`crates/daemon/src/local_ipc.rs` and/or `crates/daemon/src/service.rs`);
- focused daemon session/controller test files and shared session test fixture needed to prove the behavior.

Do not modify root dependencies, core/proto sources, store/auth/transport/lifecycle code, CLI commands, or Trellis specs/task artifacts.

You are not alone in the codebase. Preserve all existing/user edits, do not revert unrelated changes, and adapt to concurrent core/proto work. Do not commit, push, or merge.

Required behavior:

1. `prepare_attach` and `prepare_attach_until` require an explicit `AttachmentPrincipal`; no adapter infers it from a target/selector.
2. The principal must be carried by the actor command and stored in every `ActorAttachment` / prepared attachment ownership path.
3. Takeover must verify that the caller principal matches the prepared attachment owner, while retaining the existing same-operation response-loss continuation semantics.
4. Add a bounded `detach_remote_principal_until(device_id, deadline)` service/actor path that removes only matching `RemoteEndpoint` attachments and matching controller leases across live/provisional sessions and returns a precise impact/result.
5. Principal detach must never close a Session, interrupt/signal a PTY, remove the terminal model, or affect local/other-remote attachments.
6. Preserve existing mailboxes, deadlines, lock order, replay owner and Session/PTY lifetime contracts. Use deterministic synchronization in tests, not sleeps as concurrency evidence.

Update every existing prepare-attach call site explicitly and run the full Step 4 focused gate. Add focused integration evidence for remote principal detach across multiple sessions, stale input/resize/takeover rejection, local/other-principal survival, and idempotent natural-exit/detach races. Report any design conflict rather than widening into M7 transport code.
