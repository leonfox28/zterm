#!/bin/sh
set -eu

minimum_macos=13.0
minimum_glibc=2.28

fail() {
    echo "native release build failed: $*" >&2
    exit 1
}

if [ "${1:-}" = contract ]; then
    printf '%s\n' "minimum_macos=$minimum_macos" "minimum_glibc=$minimum_glibc"
    exit 0
fi

output_dir=${1:-}
if [ "$#" -ne 1 ] || [ -z "$output_dir" ]; then
    fail "usage: build-native.sh <output-directory>"
fi

target=${RELEASE_TARGET:-}
[ -n "$target" ] || fail "RELEASE_TARGET is required"
case "$target" in
    aarch64-apple-darwin)
        platform=macos
        export MACOSX_DEPLOYMENT_TARGET=$minimum_macos
        ;;
    aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        platform=linux
        ;;
    *) fail "unsupported native release target: $target" ;;
esac

host=$(rustc +1.98.0 -vV | sed -n 's/^host: //p')
[ "$host" = "$target" ] || fail "Rust host $host does not equal release target $target"

source_commit=${ZTERM_SOURCE_COMMIT:-}
source_epoch=${SOURCE_DATE_EPOCH:-}
[ -n "$source_commit" ] || fail "candidate build requires ZTERM_SOURCE_COMMIT"
[ -n "$source_epoch" ] || fail "candidate build requires SOURCE_DATE_EPOCH"
[ "$(git rev-parse HEAD)" = "$source_commit" ] \
    || fail "checkout does not match ZTERM_SOURCE_COMMIT"

cargo +1.98.0 build --locked --release --package zterm-cli
binary=target/release/zterm
[ -x "$binary" ] || fail "release binary is missing or not executable"

case "$platform" in
    macos)
        file "$binary" | grep -F arm64 >/dev/null \
            || fail "Mach-O architecture does not match $target"
        observed_macos=$(otool -l "$binary" \
            | awk '$1 == "minos" {print $2}' | sort -u)
        [ "$observed_macos" = "$minimum_macos" ] \
            || fail "Mach-O minimum is $observed_macos, expected $minimum_macos"
        ;;
    linux)
        [ "$(getconf GNU_LIBC_VERSION)" = "glibc $minimum_glibc" ] \
            || fail "builder does not provide glibc $minimum_glibc"
        case "$target" in
            aarch64-unknown-linux-gnu) machine='AArch64' ;;
            x86_64-unknown-linux-gnu) machine='Advanced Micro Devices X86-64' ;;
        esac
        readelf --file-header "$binary" \
            | grep -E "Machine:[[:space:]]+$machine$" >/dev/null \
            || fail "ELF architecture does not match $target"
        maximum_glibc=$(readelf --version-info "$binary" \
            | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)
        [ -n "$maximum_glibc" ] || fail "ELF binary has no GLIBC version requirements"
        [ "$(printf '%s\n%s\n' "$maximum_glibc" "GLIBC_$minimum_glibc" \
            | sort -V | tail -1)" = "GLIBC_$minimum_glibc" ] \
            || fail "ELF requires $maximum_glibc, newer than GLIBC_$minimum_glibc"
        ;;
esac

mkdir -p "$output_dir"
candidate="$output_dir/zterm-$target"
identity="$output_dir/zterm-$target.identity.json"
if [ -e "$candidate" ] || [ -e "$identity" ]; then
    fail "candidate output already exists for $target"
fi
"$binary" --internal-release-self-check >"$identity"
cp "$binary" "$candidate"
chmod 700 "$candidate"

echo "native release candidate verified for $target"
