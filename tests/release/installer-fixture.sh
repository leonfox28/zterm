#!/bin/sh
set -eu

[ "$#" -eq 2 ] || {
    echo "usage: installer-fixture.sh RELEASE_DIRECTORY TEST_INSTALLER" >&2
    exit 1
}
release_directory=$1
test_installer=$2

fail() {
    echo "installer fixture failed: $*" >&2
    exit 1
}

for tool in curl openssl python3 sed wc; do
    command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
done

temporary=$(mktemp -d "${TMPDIR:-/tmp}/zterm-installer-fixture.XXXXXX") \
    || fail "unable to create fixture directory"
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$temporary"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

cat > "$temporary/certificate.conf" <<'EOF'
[req]
prompt = no
distinguished_name = subject
x509_extensions = extensions
[subject]
CN = 127.0.0.1
[extensions]
subjectAltName = IP:127.0.0.1
basicConstraints = critical,CA:TRUE
keyUsage = critical,digitalSignature,keyEncipherment,keyCertSign
EOF
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
    -config "$temporary/certificate.conf" \
    -keyout "$temporary/server.key" -out "$temporary/server.crt" \
    >/dev/null 2>&1 || fail "unable to create fixture certificate"
: > "$temporary/requests.log"
python3 "$(dirname -- "$0")/https_fixture.py" \
    "$release_directory" "$temporary/server.crt" "$temporary/server.key" \
    "$temporary/port" "$temporary/requests.log" &
server_pid=$!

attempt=0
while [ ! -s "$temporary/port" ]; do
    attempt=$((attempt + 1))
    [ "$attempt" -le 10 ] || fail "fixture server did not start"
    sleep 1
done
fixture_base="https://127.0.0.1:$(cat "$temporary/port")"
fixture_home="$temporary/home"
install_directory="$fixture_home/.local/bin"
(
    umask 0077
    mkdir -p "$fixture_home/.local"
)
(
    umask 0002
    mkdir "$install_directory"
)
case "$(LC_ALL=C ls -ld "$install_directory")" in
    drwxrwxr-x*) ;;
    *) fail "fixture did not create an existing 0775 default install directory" ;;
esac

HOME=$fixture_home CURL_CA_BUNDLE=$temporary/server.crt \
    ZTERM_INSTALL_TEST_BASE_URL=$fixture_base \
    sh "$test_installer" \
    > "$temporary/happy.output" 2>&1 \
    || fail "authenticated installer happy path failed"
[ -x "$install_directory/zterm" ] || fail "installed executable is missing"
[ ! -e "$fixture_home/.zterm" ] || fail "installer created managed user state"
"$install_directory/zterm" --internal-release-self-check >/dev/null \
    || fail "installed executable self-check failed"
grep -Fq "Add $install_directory to PATH" "$temporary/happy.output" \
    || fail "installer omitted PATH guidance"

requests_before=$(wc -l < "$temporary/requests.log" | tr -d '[:space:]')
if HOME=$fixture_home CURL_CA_BUNDLE=$temporary/server.crt \
    ZTERM_INSTALL_TEST_BASE_URL=$fixture_base \
    sh "$test_installer" \
    > "$temporary/existing.output" 2>&1; then
    fail "installer overwrote an existing destination"
fi
requests_after=$(wc -l < "$temporary/requests.log" | tr -d '[:space:]')
[ "$requests_after" -eq "$requests_before" ] \
    || fail "existing destination was rejected after a network request"

zero_digest=0000000000000000000000000000000000000000000000000000000000000000
sed "s/^manifest_sha256='[0-9a-f]*'/manifest_sha256='$zero_digest'/" \
    "$test_installer" > "$temporary/bad-digest-installer.sh"
if HOME=$fixture_home CURL_CA_BUNDLE=$temporary/server.crt \
    ZTERM_INSTALL_TEST_BASE_URL=$fixture_base \
    sh "$temporary/bad-digest-installer.sh" \
    --install-dir "$fixture_home/bad-digest-bin" \
    > "$temporary/bad-digest.output" 2>&1; then
    fail "installer accepted an invalid embedded manifest digest"
fi
grep -Fq "manifest digest" "$temporary/bad-digest.output" \
    || fail "invalid manifest digest diagnostic is missing"

echo "authenticated installer fixture verified"
