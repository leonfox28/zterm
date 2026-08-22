#!/bin/sh

set -eu

mode=all
case $# in
    0)
        ;;
    2)
        if [ "$1" != "--mode" ]; then
            echo "usage: $0 [--mode tmux|herdr|m4|all]" >&2
            exit 2
        fi
        mode=$2
        ;;
    *)
        echo "usage: $0 [--mode tmux|herdr|m4|all]" >&2
        exit 2
        ;;
esac

run_tmux=false
run_herdr=false
run_agents=false
case "$mode" in
    tmux)
        run_tmux=true
        ;;
    herdr)
        run_herdr=true
        ;;
    m4)
        run_tmux=true
        run_herdr=true
        ;;
    all)
        run_tmux=true
        run_herdr=true
        run_agents=true
        ;;
    *)
        echo "unsupported terminal black-box mode: $mode" >&2
        echo "usage: $0 [--mode tmux|herdr|m4|all]" >&2
        exit 2
        ;;
esac

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
test_root=$(mktemp -d)
tmux_socket="$test_root/tmux.sock"
tmux_started=false

cleanup() {
    if [ "$tmux_started" = true ]; then
        "$tmux_bin" -S "$tmux_socket" kill-server >/dev/null 2>&1 || true
    fi
    rm -rf "$test_root"
}

trap cleanup EXIT HUP INT TERM

write_interaction() {
    interaction_file=$1
    resize_marker=$2
    completion_marker=$3
    latest_marker=$4
    cat >"$interaction_file" <<EOF
stty size > '$resize_marker'; i=0; while [ "\$i" -lt 400 ]; do printf '\033[3%dmzterm-blackbox-%04d\033[0m\n' "\$((i % 8))" "\$i"; i=\$((i + 1)); done; printf '$latest_marker\n'; printf complete > '$completion_marker'
EOF
}

run_adapter() {
    TERM=xterm-256color cargo test -p zterm-daemon --test terminal_blackbox -- "$@"
}

cd "$repo_root"

if [ "$run_tmux" = true ]; then
    tmux_bin=$(command -v tmux)
    tmux_version=$("$tmux_bin" -V)
    if [ "$tmux_version" != "tmux 3.7c" ]; then
        echo "expected tmux 3.7c, got: $tmux_version" >&2
        exit 1
    fi

    tmux_case_dir="$test_root/tmux"
    mkdir -p "$tmux_case_dir"
    tmux_interaction="$tmux_case_dir/interaction"
    tmux_resize="$tmux_case_dir/resize"
    tmux_complete="$tmux_case_dir/complete"
    write_interaction "$tmux_interaction" "$tmux_resize" "$tmux_complete" ZTERM-TMUX-LATEST
    tmux_started=true
    run_adapter \
        --case tmux-3.7c \
        --mode interaction \
        --expect-screen alternate \
        --program "$tmux_bin" \
        --arg -S \
        --arg "$tmux_socket" \
        --arg -f \
        --arg /dev/null \
        --arg new-session \
        --arg /bin/sh \
        --cwd "$tmux_case_dir" \
        --interaction-file "$tmux_interaction" \
        --resize-marker "$tmux_resize" \
        --completion-marker "$tmux_complete" \
        --expect-latest ZTERM-TMUX-LATEST \
        --quit-hex 657869740a
    "$tmux_bin" -S "$tmux_socket" kill-server >/dev/null 2>&1 || true
    tmux_started=false
    if "$tmux_bin" -S "$tmux_socket" has-session >/dev/null 2>&1; then
        echo "test tmux server remains: $tmux_socket" >&2
        exit 1
    fi
fi

if [ "$run_herdr" = true ]; then
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64)
            herdr_asset=herdr-macos-aarch64
            herdr_sha=a5d4f4d504d8b309c91f811050559300faba31258425f53c50852fc96f6ae574
            ;;
        *)
            echo "Herdr v0.8.2 black-box is not configured for $(uname -s)/$(uname -m)" >&2
            exit 1
            ;;
    esac

    herdr_case_dir="$test_root/herdr"
    mkdir -p "$herdr_case_dir/home"
    herdr_bin="$herdr_case_dir/herdr"
    curl --fail --location --retry 3 --retry-all-errors --retry-delay 2 --silent --show-error \
        --connect-timeout 15 --max-time 180 \
        --output "$herdr_bin" \
        "https://github.com/herdrdev/herdr/releases/download/v0.8.2/$herdr_asset"
    printf '%s  %s\n' "$herdr_sha" "$herdr_bin" | shasum -a 256 -c -
    chmod 700 "$herdr_bin"
    herdr_config="$herdr_case_dir/config.toml"
    cat >"$herdr_config" <<'EOF'
onboarding = false

[terminal]
default_shell = "/bin/sh"
shell_mode = "non_login"
new_cwd = "current"
EOF
    herdr_socket="$herdr_case_dir/herdr.sock"
    herdr_interaction="$herdr_case_dir/interaction"
    herdr_resize="$herdr_case_dir/resize"
    herdr_complete="$herdr_case_dir/complete"
    write_interaction "$herdr_interaction" "$herdr_resize" "$herdr_complete" ZTERM-HERDR-LATEST
    run_adapter \
        --case herdr-0.8.2 \
        --mode interaction \
        --expect-screen alternate \
        --program /usr/bin/env \
        --arg "HOME=$herdr_case_dir/home" \
        --arg "SHELL=/bin/sh" \
        --arg "HERDR_CONFIG_PATH=$herdr_config" \
        --arg "HERDR_SOCKET_PATH=$herdr_socket" \
        --arg HERDR_DISABLE_SOUND=1 \
        --arg "$herdr_bin" \
        --arg --no-session \
        --cwd "$herdr_case_dir" \
        --interaction-file "$herdr_interaction" \
        --resize-marker "$herdr_resize" \
        --completion-marker "$herdr_complete" \
        --expect-latest ZTERM-HERDR-LATEST \
        --quit-hex 0271

    if [ -S "$herdr_socket" ]; then
        echo "isolated Herdr socket remains after --no-session exit: $herdr_socket" >&2
        exit 1
    fi
fi

if [ "$run_agents" = true ]; then
    codex_bin=$(command -v codex)
    codex_version=$("$codex_bin" --version)
    if [ "$codex_version" != "codex-cli 0.148.0" ]; then
        echo "expected codex-cli 0.148.0, got: $codex_version" >&2
        exit 1
    fi
    codex_case_dir="$test_root/codex"
    mkdir -p "$codex_case_dir/home/.codex" "$codex_case_dir/workspace"
    run_adapter \
        --case codex-cli-0.148.0 \
        --mode startup \
        --expect-screen main \
        --program /usr/bin/env \
        --arg "HOME=$codex_case_dir/home" \
        --arg "CODEX_HOME=$codex_case_dir/home/.codex" \
        --arg "$codex_bin" \
        --cwd "$codex_case_dir/workspace" \
        --quit-hex 0303

    case "$(uname -s):$(uname -m)" in
        Darwin:arm64)
            opencode_asset=opencode-darwin-arm64.zip
            opencode_sha=b483e547c029b4f0ba381f0d0c5b420bec48c24c2bbec1fb7f22252bae83da46
            ;;
        *)
            echo "OpenCode v1.18.20 smoke is not configured for $(uname -s)/$(uname -m)" >&2
            exit 1
            ;;
    esac

    opencode_case_dir="$test_root/opencode"
    mkdir -p "$opencode_case_dir/home" "$opencode_case_dir/workspace"
    opencode_archive="$opencode_case_dir/$opencode_asset"
    curl --fail --location --retry 3 --retry-all-errors --retry-delay 2 --silent --show-error \
        --connect-timeout 15 --max-time 180 \
        --output "$opencode_archive" \
        "https://github.com/anomalyco/opencode/releases/download/v1.18.20/$opencode_asset"
    printf '%s  %s\n' "$opencode_sha" "$opencode_archive" | shasum -a 256 -c -
    unzip -q "$opencode_archive" -d "$opencode_case_dir"
    opencode_bin="$opencode_case_dir/opencode"
    chmod 700 "$opencode_bin"
    opencode_version=$("$opencode_bin" --version)
    if [ "$opencode_version" != "1.18.20" ]; then
        echo "expected OpenCode 1.18.20, got: $opencode_version" >&2
        exit 1
    fi
    run_adapter \
        --case opencode-1.18.20 \
        --mode startup \
        --expect-screen alternate \
        --program /usr/bin/env \
        --arg "HOME=$opencode_case_dir/home" \
        --arg "XDG_CONFIG_HOME=$opencode_case_dir/home/.config" \
        --arg "XDG_DATA_HOME=$opencode_case_dir/home/.local/share" \
        --arg "XDG_CACHE_HOME=$opencode_case_dir/home/.cache" \
        --arg "$opencode_bin" \
        --cwd "$opencode_case_dir/workspace" \
        --quit-hex 03 \
        --quit-hex 03
fi

cleanup
trap - EXIT HUP INT TERM
if [ -e "$test_root" ]; then
    echo "temporary black-box root remains: $test_root" >&2
    exit 1
fi
if [ "$run_tmux" = true ] && "$tmux_bin" -S "$tmux_socket" has-session >/dev/null 2>&1; then
    echo "test tmux server remains after cleanup: $tmux_socket" >&2
    exit 1
fi

echo "TERMINAL_BLACKBOX_GATE=PASS"
echo "BLACKBOX_CLEANUP=PASS"
