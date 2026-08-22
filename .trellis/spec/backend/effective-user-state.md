# Effective-User State Contract

## Scope

Apply this contract to effective-account lookup, `UserPaths`, configuration,
identity, SQLite state, setup, and managed files.

## Contracts

- The effective UID account record owns home, shell, UID, and GID. `$HOME` and
  `$SHELL` are never persistent-state authority.
- Non-spawning diagnostics validate that the account home is an owned
  directory and the account-record login shell is an absolute executable file.
- Persistent state is rooted at `<account-home>/.zterm`. State and runtime
  directories are owned by the effective UID and mode `0700`; managed regular
  files and sockets are no wider than `0600`. Symlinks are rejected.
- Atomic replacement uses a same-directory create-new sibling, file sync,
  rename, and directory sync. Identity creation never replaces an existing
  file.
- `identity.key` is exactly the 32 bytes returned by Iroh `SecretKey`. Setup
  creates it only when no committed config or database exists. Repeated,
  concurrent, failed, or resumed setup never rotates it.
- Config schema v1 represents either official N0 or one explicit
  self-hosted-only HTTPS Relay. Mixed, staging, empty, or invalid profiles are
  rejected without modifying identity or state.
- SQLite is bundled, single-owner, `foreign_keys=ON`, rollback journal,
  `synchronous=FULL`, and transactionally migrated. `PRAGMA user_version` is
  the only schema version.
- Before SQLite `NOFOLLOW` open, the already validated database path is
  canonicalized so macOS' system `/var` symlink is not mistaken for a managed
  database symlink; the final open still uses `SQLITE_OPEN_NOFOLLOW`.
- State contains identity metadata, authorization tombstones/generations, and
  versioned route cache. It does not persist PTYs, terminal bytes, sessions,
  replay windows, pairing offers, or audit streams.
- Tests inject `UserPaths` below a temporary directory. Product code exposes no
  `ZTERM_HOME` or normal CLI state-path override.

## Required evidence

- Path tests cross real type/mode/symlink boundaries and prove atomic writer
  failure leaves the prior file unchanged.
- Setup tests cover fresh, repeated, concurrent, resumable, missing-key, bad
  key, metadata mismatch, and invalid config states.
- Persistence tests cover schema inventory, too-new schema, transaction
  rollback, and identity consistency.
- CLI setup tests prove first noninteractive validation failure leaves the
  entire task-private state root absent and successful setup creates 0700/0600
  nodes.
- Doctor tests cover missing/unsafe managed paths without creating them.
