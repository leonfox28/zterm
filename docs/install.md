# Install, update, and uninstall

zterm's official native channel is one signed GitHub Release per lockstep Cargo
version. It supports Apple Silicon and Intel macOS 13 or newer, plus arm64 and
x86_64 GNU/Linux with glibc 2.28 or newer. Windows, Alpine/musl, and NixOS are
not supported by this installer. Package-manager channels and background
updates are intentionally absent.

The repository currently contains an explicit `UNCONFIGURED` release-public-key
placeholder. Installer signature verification and `zterm update` fail closed
until the external release-key checkpoint is completed and the reviewed public
key is committed. Existing Relay-era Releases are not native zterm installers.

## Install

The review-first path is:

```bash
curl --proto '=https' --tlsv1.2 --fail --location \
  --output /tmp/zterm-install.sh \
  https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh
less /tmp/zterm-install.sh
sh /tmp/zterm-install.sh
```

After a formal native Release exists, the shorter disclosed-bootstrap path is:

```bash
curl --proto '=https' --tlsv1.2 --fail --location \
  https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh | sh
```

Select one exact stable or prerelease tag, or another user-owned destination:

```bash
sh /tmp/zterm-install.sh --version vX.Y.Z
sh /tmp/zterm-install.sh --install-dir "$HOME/bin"
```

The default destination is `~/.local/bin/zterm`. If that directory is not on
`PATH`, the installer prints the required guidance; it does not edit shell
startup files. It refuses an existing destination, symlinked/foreign-owned
directory, unsafe permissions, unsupported target, NixOS, musl, an old OS or
glibc, and any incomplete or mismatched Release asset.

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
and four-target artifact table.

Before publishing a binary, the versioned script checks target/support floor,
destination ownership, bounded download sizes, exact manifest/archive hashes,
and the archive's single-file inventory. Only then does it execute the
candidate's side-effect-free self-check. The candidate verifies the detached
Ed25519 signature over the exact manifest bytes and cross-checks version,
target, source commit, wire/schema versions, and release-key ID before an
fsynced, atomic no-clobber install. GitHub immutable Releases and provenance
attestations additionally bind the tag and assets inside GitHub's supply-chain
audit layer.

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

Update and uninstall are available only from an official managed Release
binary. A source-tree, ordinary-CI, or `UNCONFIGURED` build is rejected before
an update request reaches the network and before uninstall observes or deletes
state. Official binaries may still be installed in any safe user-owned
directory; the proof is about build identity, not a hard-coded path.

If live Sessions would be ended, update refuses unless `--force` is explicit.
After approval, zterm stops the daemon, atomically activates the candidate,
and rechecks it. Activation/post-check failure restores the preceding binary.
PTYs already ended for an approved update cannot be restored. A successful
update does not restart the daemon; the next command starts it on demand.

Binary rollback does not imply persistent-state rollback. A future release
that changes the state schema must provide its own migration and recovery
contract before publication.

## Uninstall and recovery

Review impact interactively, or confirm noninteractively:

```bash
zterm uninstall
zterm uninstall --yes
zterm uninstall --yes --force
```

Uninstall reuses the identity-reset boundary: it stops the daemon, refuses live
Sessions without `--force`, removes only the validated managed state inventory,
and unlinks the exact running executable last. This destroys the local device
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

Before any formal tag is pushed, a repository administrator must have:

1. enabled immutable Releases and confirmed it with
   `gh api repos/leonfox28/zterm/immutable-releases`;
2. created a protected `release` Environment with the required reviewer;
3. generated the long-lived Ed25519 seed outside logs/artifacts, committed only
   its reviewed public key, and stored the lowercase seed as the Environment
   secret `ZTERM_RELEASE_SIGNING_KEY`;
4. reviewed a successful `ci.yml` push run on `main` for the exact commit,
   including all four native release-mode builds.

The default workflow token cannot read the repository Administration setting,
so the workflow does not pretend to verify immutable Releases through an
underprivileged API call and does not request a PAT. The environment reviewer
must confirm that immutable Releases remain enabled before approving the sole
seed-bearing signing job.

CI never creates a tag. After the exact `main` push run succeeds, a human
creates and pushes the canonical `v` + Cargo-version tag. That push starts the
release workflow automatically. Its validate job uses the Actions API to
require a successful `ci.yml` `push` run on `main` for the same commit before
the signing Environment or any Release state is reached. Every downstream job
uses the frozen commit, and draft creation rechecks the tag. The signing tool is
built before the only seed-bearing step so Cargo/build scripts never inherit
the secret. The GitHub-hosted workflow rebuilds four native assets, tests all
four installers, creates and round-trips one draft, emits provenance
attestations, publishes it automatically, and requires the published Release
API response to report `immutable: true`.

A normal key rotation first ships a binary that trusts the next reviewed key
through a manifest signed by the current key. If the current private key may be
compromised, freeze publication and use an independently reviewed recovery
release process; do not silently add another signature format or replace
immutable assets.
