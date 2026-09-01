#!/bin/sh
set -eu

fail() {
    echo "glibc builder setup failed: $*" >&2
    exit 1
}

[ "$(id -u)" -eq 0 ] || fail "the pinned Debian container must run setup as root"
[ -n "${GITHUB_WORKSPACE:-}" ] || fail "GITHUB_WORKSPACE is required"
[ -f /etc/apt/sources.list ] || fail "Debian package sources are unavailable"

sed -i \
    -e 's|^# deb http://snapshot|deb [check-valid-until=no] http://snapshot|' \
    -e 's|^deb http://deb|# deb http://deb|' \
    /etc/apt/sources.list
apt-get -o Acquire::Check-Valid-Until=false update
DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
    binutils build-essential ca-certificates curl file git pkg-config

git config --global --get-all safe.directory | grep -Fx "$GITHUB_WORKSPACE" >/dev/null \
    || fail "the workflow must trust only the exact checkout before builder setup"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
    https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.98.0
echo "$HOME/.cargo/bin" >>"$GITHUB_PATH"

echo 'pinned Debian 10 / glibc 2.28 builder is ready'
