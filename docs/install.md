# Install, update, and uninstall

zterm's official native channel is one signed GitHub Release per lockstep Cargo
version. It supports Apple Silicon macOS 13 or newer, plus arm64 and
x86_64 GNU/Linux with glibc 2.28 or newer. Windows, Alpine/musl, and NixOS are
not supported by this installer. Intel macOS is no longer published; its
historical Releases remain available. Package-manager channels and background
updates are intentionally absent.

## Install

The normal path installs the latest stable native Release through one fixed
bootstrap URL:

```bash
curl -fsSL https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh | sh
```

To inspect the bootstrap before running it, use the optional review-first path:

```bash
curl -fsSL \
  --output /tmp/zterm-install.sh \
  https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh
less /tmp/zterm-install.sh
sh /tmp/zterm-install.sh
```

Select one exact stable or prerelease tag, or another user-owned destination:

```bash
curl -fsSL https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh \
  | sh -s -- --version vX.Y.Z
curl -fsSL https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh \
  | sh -s -- --install-dir "$HOME/bin"
```

The downloaded review-first script accepts the same options:

```bash
sh /tmp/zterm-install.sh --version vX.Y.Z
sh /tmp/zterm-install.sh --install-dir "$HOME/bin"
```

The default destination is `~/.local/bin/zterm`. If that directory is not on
`PATH`, the installer prints the required guidance; it does not edit shell
startup files. An existing writable `~/.local/bin`, including the ordinary
`0775` user-private-group directory produced by `umask 0002`, is accepted
without requiring `chmod`. The installer refuses an existing destination, a
relative, symlinked, non-directory, or unwritable install path, an unsupported
target, NixOS, musl, an old OS or glibc, and any incomplete or mismatched
Release asset.

Installation does not run `setup`, create `~/.zterm`, generate an identity,
start a daemon, register a service, invoke `sudo`, or change shell files. Run
this separately when ready:

```bash
zterm setup
```

## What is authenticated

The small mutable repository bootstrap selects latest stable or an exact tag,
then downloads that Release's immutable `zterm-install.sh`. This first HTTPS
hop cannot defend against simultaneous compromise of the reviewed bootstrap
source and GitHub. The versioned script embeds its exact tag, manifest digest,
and that release's authenticated artifact table.

Before publishing a binary, the versioned script checks target/support floor,
absolute directory shape and basic writability, bounded download sizes, exact
manifest/archive hashes, and the archive's single-file inventory. Only then
does it execute the candidate's side-effect-free self-check. The candidate
verifies the detached Ed25519 signature over the exact manifest bytes and
cross-checks version, target, source commit, wire/schema versions, and
release-key ID before an fsynced, atomic no-clobber install. GitHub immutable
Releases and provenance attestations additionally bind the tag and assets
inside GitHub's supply-chain audit layer.

For a manual audit, download `SHA256SUMS`, `zterm-release.json`, its `.sig`, the
target archive, and `zterm-install.sh` from one exact Release. Verify the listed
SHA-256 values (`shasum -a 256` on macOS or `sha256sum` on Linux) and the GitHub
attestation, inspect the versioned script, then run it. The authenticated
candidate's internal verifier can be exercised explicitly after the archive
digest is checked:

```bash
./zterm --internal-release-verify zterm-release.json zterm-release.json.sig
```

That command succeeds only when the candidate itself is the target/build
authenticated by the signed manifest. It is an audit entry point and remains
hidden from ordinary public help.

## Explicit updates

Updates occur only when requested:

```bash
zterm update
zterm update --version vX.Y.Z
```

The first command selects the latest stable Release; the second may select an
exact stable or prerelease tag. Same-version installs and managed downgrades
are rejected. Manifest signature, target, version monotonicity, archive
length/hash, and candidate self-check all finish before zterm contacts or
stops the daemon.

The updater requires only its own target's package. Other platforms may be
absent or newly introduced without blocking this machine's update. The signed
manifest's common metadata is authenticated first; the selected entry must be
unique and pass URL, support-floor, length, digest, and build-identity checks.
No package for this platform produces an explicit unsupported-platform error.
The full publication matrix is checked by maintainer tooling, not the updater.

Update and uninstall are available only from an official managed Release
binary. A source-tree, ordinary-CI, or `UNCONFIGURED` build is rejected before
an update request reaches the network and before uninstall observes or deletes
state. Official binaries may still be installed in any user-selected absolute
writable directory; the proof is about build identity, not a hard-coded path.

With no live Sessions, update proceeds directly. Otherwise it lists their
names, including detached Sessions, and asks for English `[y/N]` confirmation.
Enter `y` or `yes` to continue in the same invocation; `-y`/`--yes` confirms
directly. Empty input, EOF, or another answer cancels. Noninteractive use with
live Sessions requires `-y`/`--yes`; public `--force` has been removed.

After approval, zterm stops the daemon, atomically activates the candidate,
and rechecks it. Activation/post-check failure restores the preceding binary.
A successful update starts the new daemon on configured installations, even
if it was previously stopped. The CLI shows actual update phases and reports
success after local readiness, without waiting for Internet connectivity.
PTYs ended for the update are not restored.

Before setup, update installs the binary and prints `Run zterm setup to
configure and start the daemon.` It never creates an identity implicitly. If
activation committed but startup fails, the new binary stays installed and
the command returns an explicit partial-completion error with restart guidance.
An already installed older updater still runs its old behavior for the first
upgrade; invoking the new binary enables these command changes.

Binary rollback does not imply persistent-state rollback. A future release
that changes the state schema must provide its own migration and recovery
contract before publication.

## One-time migration from the four-target updater

Versions through **0.1.17** require exactly four manifest targets. They reject a
new three-target manifest before downloading an update, even on supported
Apple Silicon/Linux hosts. A future binary cannot change that already installed
verifier. The new verifier accepts historical signed inventories and the new
three-target inventory without a platform allowlist or count requirement;
a missing current target returns `unsupported_platform` to the CLI.

For the first migration, use the existing authenticated installer/recovery path:

1. Download and review the bootstrap above, select the exact new release, and
   install it into an empty temporary directory with `--install-dir`. This
   authenticates the candidate before touching the current executable or daemon.
2. Stop the existing daemon with the old binary's `daemon stop`. If Sessions are
   active, finish them first; that old binary also has its historical `--force`
   option. The current CLI uses `-y`/`--yes` instead.
3. Retain the old executable at a fresh backup path, leaving its original
   destination empty. Do not run `uninstall` or remove `~/.zterm`: pairing and
   identity must remain intact.
4. Run the already authenticated candidate's
   `--internal-release-install <original-absolute-executable-path>` entry. It
   uses the existing fsynced, atomic no-clobber installer. If activation fails,
   restore the retained old executable; do not overwrite an unexpected file.
5. Check the installed `--version`, then resume normal use. Subsequent updates
   use `zterm update` again. Coordinate the CLI/daemon versions across connected
   hosts according to the existing release compatibility policy.

This is a one-time maintenance procedure using the existing verified activation
boundary, not a new background updater or identity reset. Historical Intel
installations stay on their last published compatible release.

## Uninstall and recovery

Review impact interactively, or confirm noninteractively:

```bash
zterm uninstall
zterm uninstall -y
```

Uninstall asks once to confirm the executable/identity deletion and any running
Sessions it will end, or accepts `-y`/`--yes` directly. Deletion still needs
confirmation when there are no Sessions. It reuses the identity-reset boundary:
stop the daemon, remove only the validated managed state inventory, and unlink
the exact running executable last. This destroys the local device
identity and authorization state; reinstalling and running setup creates a new
identity that must be paired again. It does not send `RevokeSelf` to other
devices, so copied old private keys remain a per-host revoke concern.

Ordinary update failures leave either the current binary untouched or the
automatic rollback binary restored. If a published binary cannot run its own
updater, quarantine the exact executable, review/download a known-good
versioned installer, and install into the now-empty destination. Do this only
when that release supports the current state schema; otherwise preserve
`~/.zterm` and request a release-specific recovery procedure. Never replace an
unknown file or bypass signature/hash verification merely to force a rollback.

## Release-operator checkpoint

The executable release runbook is [Release operations](releasing.md). This
section records the installer trust prerequisites rather than duplicating the
operator commands.

Before any formal tag is pushed, a repository administrator must have:

1. enabled immutable Releases and confirmed it with
   `gh api repos/leonfox28/zterm/immutable-releases`;
2. created a protected `release` Environment with the required reviewer;
3. generated the long-lived Ed25519 seed outside logs/artifacts, committed only
   its reviewed public key, and stored the lowercase seed as the Environment
   secret `ZTERM_RELEASE_SIGNING_KEY`;
4. reviewed a successful `ci.yml` push run on `main` for the exact commit,
   including the three native candidate builds and verified unsigned assembly.

The default workflow token cannot read the repository Administration setting,
so the workflow does not pretend to verify immutable Releases through an
underprivileged API call and does not request a PAT. The environment reviewer
must confirm that immutable Releases remain enabled before approving the sole
seed-bearing signing job.

CI never creates a tag. `release-prepare` prepares the version in the feature
PR, or creates a standalone version PR from main. `release-publish` tags only
after exact main CI and its retained candidate pass; `just release VERSION PR`
coordinates the required waits and merge. The tag workflow reuses that candidate,
tests three signed installers, round-trips one late draft, emits provenance
attestations, and requires immutable publication. Relay images are not published.

A normal key rotation first ships a binary that trusts the next reviewed key
through a manifest signed by the current key. If the current private key may be
compromised, freeze publication and use an independently reviewed recovery
release process; do not silently add another signature format or replace
immutable assets.
