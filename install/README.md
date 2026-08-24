# Official installer boundary

`install.sh` is the small mutable GitHub HTTPS bootstrap. It resolves either
the latest stable Release or one exact published tag, downloads that Release's
immutable `zterm-install.sh`, and executes it. The bootstrap cannot defend
against simultaneous compromise of the reviewed bootstrap source and GitHub
HTTPS; users who need a fixed trust root should follow the reviewed/manual
verification path in `docs/install.md`.

`versioned.sh.in` is never served directly. `zterm-release-tool prepare`
generates the immutable Release installer from the signed manifest's single
artifact inventory. The generated script authenticates the manifest/archive
against its embedded digest table, then delegates Ed25519 verification,
candidate identity checks, fsync, and atomic no-clobber publication to the
digest-authenticated candidate binary.

Neither stage runs setup, creates `~/.zterm`, starts a daemon, changes shell
files, invokes sudo, or registers a service.
