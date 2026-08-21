# PTY Lifecycle Contract

## Scenario: Host-Owned Login PTY

### 1. Scope / Trigger

This contract applies whenever `zterm-platform` opens, reads, resizes, waits for,
or closes a pseudo-terminal. It also applies when daemon code attaches terminal
consumers to a running session. The purpose is to keep operating-system PTY
details and termination authority out of transports and UI attachments.

### 2. Signatures

The public platform boundary is zterm-owned:

```rust
PtyHost::spawn(command: ExplicitPtyCommand, size: PtySize)
    -> Result<PtySession, PtyError>
PtyHost::spawn_current_account_login_shell(
    size: PtySize,
    working_directory: Option<&Path>,
) -> Result<PtySession, PtyError>

PtySession::take_reader(&mut self) -> Result<PtyReader, PtyError>
PtySession::write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError>
PtySession::resize(&self, size: PtySize) -> Result<(), PtyError>
PtySession::try_wait(&mut self) -> Result<PtyChildState, PtyError>
PtySession::wait(&mut self) -> Result<PtyExitStatus, PtyError>
PtySession::close_explicitly(&mut self) -> Result<PtyExitStatus, PtyError>
```

`portable_pty` master, slave, reader, writer, child, killer, exit-status, and
error types remain private implementation details. `ExplicitPtyCommand` is a
low-level fixture/integration primitive, not a first-stage user command API.

### 3. Contracts

- On supported Unix systems, the default session shell, home, and default cwd
  come from the effective UID's account record, never the daemon's inherited
  `$SHELL`, `$HOME`, or cwd.
- The builder explicitly sets `HOME`, `SHELL`, and cwd, then uses
  `CommandBuilder::new_default_prog()` so portable-pty supplies login argv0.
- The output reader transfers exactly once. The session retains the PTY master,
  input writer, and root-child handle.
- Only the session owner may call `close_explicitly()`. Attachments and
  transports may observe terminal state but must not own `PtySession`, a PTY
  handle, or a child-kill capability.
- Dropping an attachment, connection, or revision subscriber does not call PTY
  close. Root-child natural exit and explicit session close are the only
  Foundation termination triggers.
- `close_explicitly()` uses portable-pty's child killer once and waits. Zterm
  adds no signal escalation or process-group policy at this layer.
- Windows keeps the same zterm-owned boundary; current-account Unix login-shell
  behavior returns a typed unsupported-platform error until a native Windows
  implementation is added.

No environment key is required from a caller for account-backed shell startup.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| rows or columns is zero | `PtyError::InvalidSize`, before `openpty` |
| program/shell/cwd/home is relative | `InvalidPath(NotAbsolute)` |
| executable does not exist or is not a file | `InvalidPath(NotFound/NotFile)` |
| shell/program lacks effective execute access | `InvalidPath(NotExecutable)` |
| home/cwd is not an accessible directory | `InvalidPath(NotDirectory/Inaccessible)` |
| effective account lookup fails or has no record | `AccountLookup` / `AccountNotFound` |
| second reader transfer | `ReaderAlreadyTaken` |
| native operation fails | `Operation` with a zterm-owned `PtyOperation` |
| account login requested on an unsupported target | `UnsupportedPlatform` |

All path and size validation must finish before any PTY is opened.

### 5. Good / Base / Bad Cases

- Good: use the effective account record, explicitly set account environment and
  cwd, transfer one reader to the daemon drain, and retain `PtySession` in the
  session owner until natural exit or an explicit user close.
- Base: start a deterministic absolute-path fixture through
  `ExplicitPtyCommand`, read its output, resize it, and wait for natural exit.
- Bad: inherit the daemon's `$SHELL` or cwd, expose portable-pty types across a
  crate boundary, give an attachment a `PtySession`, or close the PTY when an
  Iroh stream disappears.

### 6. Tests Required

- Unit: reject zero dimensions and invalid program/shell/home/cwd before PTY
  creation; assert the login builder uses effective-account values.
- Unix integration: real PTY input/output, child-observed resize, natural exit
  status, reader single transfer, and explicit close with a bounded deadline.
- Drain integration: emit more than the kernel PTY buffer and write an
  independent control marker after all writes; the marker must appear even with
  zero attachments.
- Ownership integration: drop attachment and simulated transport guards while
  the root child continues; only explicit close may terminate it.
- Platform CI: run behavior tests on Unix and compile/run the non-Unix boundary
  on Windows without importing Unix account APIs.

### 7. Wrong vs Correct

#### Wrong

```rust
// A connection owns the process and kills it when the socket disappears.
struct Attachment {
    pty: PtySession,
}
```

#### Correct

```rust
// The session runtime owns the PTY. An attachment owns only a terminal-state
// subscription/checkpoint and can be dropped independently.
struct Attachment {
    terminal_view: TerminalView,
}
```

## Design Decision: One Termination Authority

The PTY session owner is the sole component with close authority. This prevents
temporary network loss, slow consumers, and future multi-device attachment
churn from changing process lifetime. A later session supervisor may add an
explicit close policy, but it must preserve the same ownership boundary.
