#!/bin/sh
set -eu

repository="leonfox28/zterm"
version=""
install_dir=""
maximum_script_bytes=262144

usage() {
    cat <<'EOF'
Usage: install.sh [--version <vSEMVER>] [--install-dir <directory>]

Without --version, installs the latest stable GitHub Release. An exact stable
or prerelease tag may be selected explicitly. This bootstrap downloads and
runs the immutable installer from that Release; it never runs zterm setup.
EOF
}

fail() {
    echo "zterm bootstrap failed: $*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a tag"
            version=$2
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || fail "--install-dir requires a directory"
            install_dir=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

if [ -n "$version" ]; then
    case "$version" in
        *[!A-Za-z0-9.+-]*) fail "--version contains unsupported characters" ;;
    esac
    command -v awk >/dev/null 2>&1 || fail "awk is required to validate --version"
    version_value=${version#v}
    [ "$version_value" != "$version" ] || fail "--version must be a canonical v-prefixed SemVer tag"
    printf '%s\n' "$version_value" | awk '
        /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-((0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*))?(\+([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$/ { valid = 1 }
        END { exit valid ? 0 : 1 }
    ' || fail "--version must be a canonical v-prefixed SemVer tag"
    installer_url="https://github.com/$repository/releases/download/$version/zterm-install.sh"
else
    installer_url="https://github.com/$repository/releases/latest/download/zterm-install.sh"
fi

if command -v curl >/dev/null 2>&1; then
    download() {
        file_blocks=$((($3 + 511) / 512))
        (
            ulimit -f "$file_blocks"
            curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
                --connect-timeout 10 --max-time 60 --max-filesize "$3" \
                --output "$2" "$1"
        )
    }
elif command -v wget >/dev/null 2>&1; then
    download() {
        file_blocks=$((($3 + 511) / 512))
        (
            ulimit -f "$file_blocks"
            wget --https-only --timeout=60 --tries=1 --quiet \
                --output-document="$2" "$1"
        )
    }
else
    fail "curl or wget is required"
fi

temporary=$(mktemp -d "${TMPDIR:-/tmp}/zterm-bootstrap.XXXXXX") \
    || fail "unable to create a private temporary directory"
chmod 700 "$temporary" || fail "unable to secure the temporary directory"
cleanup() {
    rm -f "$temporary/zterm-install.sh"
    rmdir "$temporary" 2>/dev/null || true
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

installer="$temporary/zterm-install.sh"
download "$installer_url" "$installer" "$maximum_script_bytes" \
    || fail "unable to download immutable installer"
script_bytes=$(wc -c < "$installer" | tr -d '[:space:]')
[ "$script_bytes" -gt 0 ] 2>/dev/null || fail "downloaded installer is empty"
[ "$script_bytes" -le "$maximum_script_bytes" ] 2>/dev/null \
    || fail "downloaded installer exceeds its size bound"

if [ -n "$install_dir" ]; then
    sh "$installer" --install-dir "$install_dir"
else
    sh "$installer"
fi
