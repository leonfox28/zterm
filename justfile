set shell := ["sh", "-eu", "-c"]

default:
    @just --list

# Report required local tools and the hosted-only evidence boundary.
doctor:
    sh tools/ci/doctor.sh

# Fast edit-loop checks: portable policy, native Clippy, and secret-scan scope.
check-fast:
    sh tests/source-policy.sh
    just ci-policy
    cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
    sh tests/secret-scan.sh
    sh tests/secret-scan-fixture.sh

# Authoritative pre-push gate on the current host.
check:
    just check-fast
    cargo +1.98.0 test --workspace --all-features
    cargo +1.98.0 doc --workspace --no-deps
    just ci-dependencies
    cargo +1.98.0 fmt --manifest-path tests/relay/handshake-probe/Cargo.toml -- --check
    cargo +1.98.0 clippy --locked --manifest-path tests/relay/handshake-probe/Cargo.toml -- -D warnings
    sh -n deploy/relay/*.sh tests/relay/*.sh
    sh tests/relay/publication-channels.sh
    sh tests/relay/verify-upstream.sh
    if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then sh tests/relay/static.sh; else echo 'HOSTED-ONLY: relay Compose metadata requires Docker Compose'; fi
    @echo 'HOSTED-ONLY: other macOS/Linux architectures, Windows, glibc 2.28, Docker/QEMU image execution, protected signing, final installers, attestation, and immutable publication run in GitHub Actions.'

# Canonical portable CI owner: version, format, workflow/release policy, shell, and Python syntax.
ci-policy:
    sh tests/workspace-version.sh
    sh tests/terminal-dependency-policy.sh
    cargo +1.98.0 fmt --all -- --check
    actionlint
    sh tests/release/static.sh
    sh tests/release/operator-fixture.sh
    shellcheck -s sh install/install.sh tests/release/*.sh tests/secret-scan*.sh tests/terminal-dependency-policy.sh $(find tools/ci tools/release -type f -name '*.sh' -print)
    sh tools/ci/check-python-syntax.sh tests/release/https_fixture.py tests/release/https_fixture_bind_test.py

# Full Unix runtime evidence; CI assigns docs/smoke to their canonical hosts.
ci-unix docs='false' smoke='false':
    case {{ quote(docs) }} in true|false) ;; *) echo 'docs must be true or false' >&2; exit 64;; esac
    case {{ quote(smoke) }} in true|false) ;; *) echo 'smoke must be true or false' >&2; exit 64;; esac
    cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
    cargo +1.98.0 test --workspace --all-features
    if [ {{ quote(docs) }} = true ]; then cargo +1.98.0 doc --workspace --no-deps; fi
    if [ {{ quote(smoke) }} = true ]; then cargo +1.98.0 run --quiet --package zterm-cli; fi

# Hosted Windows shared/unsupported-platform boundary.
ci-windows:
    cargo +1.98.0 clippy --workspace --lib --bins --all-features -- -D warnings
    cargo +1.98.0 test -p zterm-core -p zterm-proto -p zterm-platform -p zterm-terminal -p zterm-daemon --lib --all-features

# Workspace and isolated relay-probe dependency policy.
ci-dependencies:
    cargo +1.98.0 deny check
    cargo +1.98.0 deny --manifest-path tests/relay/handshake-probe/Cargo.toml --config tests/relay/handshake-probe/deny.toml check

# Docker-capable Ubuntu owner for the complete optional relay bundle.
ci-relay:
    cargo +1.98.0 fmt --manifest-path tests/relay/handshake-probe/Cargo.toml -- --check
    cargo +1.98.0 clippy --locked --manifest-path tests/relay/handshake-probe/Cargo.toml -- -D warnings
    sh -n deploy/relay/*.sh tests/relay/*.sh
    sh tests/relay/static.sh
    sh tests/relay/publication-channels.sh
    sh tests/relay/verify-upstream.sh
    sh tests/relay/build-platforms.sh
    sh tests/relay/smoke.sh
    sh tests/secret-scan.sh
    sh tests/secret-scan-fixture.sh

# Create and open a reviewable release PR; never creates a tag.
release-prepare version:
    sh tools/release/operator.sh prepare {{ quote(version) }}

# Tag exact protected green main and watch the formal release workflow.
release-publish version:
    sh tools/release/operator.sh publish {{ quote(version) }}
