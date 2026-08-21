#!/bin/sh

set -eu

LC_ALL=C
export LC_ALL

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
test_root=$(mktemp -d)
results_file="$test_root/results.tsv"

cleanup() {
    rm -rf "$test_root"
}

trap cleanup EXIT HUP INT TERM

fail() {
    echo "terminal resource gate failed: $*" >&2
    exit 1
}

field_from_line() {
    line=$1
    wanted=$2
    printf '%s\n' "$line" | awk -v wanted="$wanted" '
        {
            for (field_index = 1; field_index <= NF; field_index++) {
                split($field_index, pair, "=")
                if (pair[1] == wanted) {
                    print substr($field_index, length(wanted) + 2)
                    exit
                }
            }
        }
    '
}

assert_integer() {
    label=$1
    value=$2
    case "$value" in
        ''|*[!0-9]*) fail "$label is not an integer: $value" ;;
    esac
}

run_case() {
    case_name=$1
    sessions=$2
    rows=$3
    columns=$4
    scrollback=$5
    workload_lines=$6
    stdout_file="$test_root/$case_name.stdout"
    time_file="$test_root/$case_name.time"

    case "$(uname -s)" in
        Darwin)
            /usr/bin/time -l "$bench_bin" \
                --sessions "$sessions" \
                --rows "$rows" \
                --columns "$columns" \
                --scrollback "$scrollback" \
                --workload-lines "$workload_lines" \
                >"$stdout_file" 2>"$time_file"
            max_rss_bytes=$(awk '/maximum resident set size/ { print $1; exit }' "$time_file")
            user_seconds=$(awk 'NR == 1 && $4 == "user" { print $3; exit }' "$time_file")
            sys_seconds=$(awk 'NR == 1 && $6 == "sys" { print $5; exit }' "$time_file")
            ;;
        Linux)
            /usr/bin/time -v "$bench_bin" \
                --sessions "$sessions" \
                --rows "$rows" \
                --columns "$columns" \
                --scrollback "$scrollback" \
                --workload-lines "$workload_lines" \
                >"$stdout_file" 2>"$time_file"
            max_rss_kib=$(awk -F: '/Maximum resident set size/ { gsub(/[[:space:]]/, "", $2); print $2; exit }' "$time_file")
            assert_integer "$case_name max RSS KiB" "$max_rss_kib"
            max_rss_bytes=$((max_rss_kib * 1024))
            user_seconds=$(awk -F: '/User time \(seconds\)/ { gsub(/[[:space:]]/, "", $2); print $2; exit }' "$time_file")
            sys_seconds=$(awk -F: '/System time \(seconds\)/ { gsub(/[[:space:]]/, "", $2); print $2; exit }' "$time_file")
            ;;
        *)
            fail "unsupported host for /usr/bin/time parsing: $(uname -s)"
            ;;
    esac

    bench_line=$(awk '/^TERMINAL_BENCH / { print; count++ } END { if (count != 1) exit 1 }' "$stdout_file") ||
        fail "$case_name did not emit exactly one TERMINAL_BENCH record"
    structural_bytes=$(field_from_line "$bench_line" structural_bytes)
    retained_history_rows=$(field_from_line "$bench_line" retained_history_rows_per_session)
    elapsed_ns=$(field_from_line "$bench_line" elapsed_ns)
    assert_integer "$case_name structural bytes" "$structural_bytes"
    assert_integer "$case_name retained history rows" "$retained_history_rows"
    assert_integer "$case_name elapsed ns" "$elapsed_ns"
    assert_integer "$case_name max RSS bytes" "$max_rss_bytes"
    [ -n "$user_seconds" ] || fail "$case_name has no user CPU measurement"
    [ -n "$sys_seconds" ] || fail "$case_name has no system CPU measurement"
    wall_seconds=$(awk -v nanoseconds="$elapsed_ns" 'BEGIN { printf "%.6f", nanoseconds / 1000000000 }')

    printf '%s\n' "$bench_line"
    printf 'TERMINAL_RESOURCE case=%s max_rss_bytes=%s wall_seconds=%s user_seconds=%s sys_seconds=%s\n' \
        "$case_name" "$max_rss_bytes" "$wall_seconds" "$user_seconds" "$sys_seconds"
    printf '%s\t%s\t%s\t%s\n' \
        "$case_name" "$structural_bytes" "$max_rss_bytes" "$retained_history_rows" >>"$results_file"
}

result_field() {
    wanted_case=$1
    field_number=$2
    awk -F '\t' -v wanted_case="$wanted_case" -v field_number="$field_number" '
        $1 == wanted_case { print $field_number; found++ }
        END { if (found != 1) exit 1 }
    ' "$results_file"
}

cd "$repo_root"

command -v cargo >/dev/null 2>&1 || fail "cargo is required"
command -v jq >/dev/null 2>&1 || fail "jq is required"
[ -x /usr/bin/time ] || fail "/usr/bin/time is required"

build_json="$test_root/build.json"
cargo build --profile bench -p zterm-core --bench terminal_state --message-format=json >"$build_json"
bench_bin=$(jq -r '
    select(.reason == "compiler-artifact")
    | select(.target.name == "terminal_state")
    | select(.executable != null)
    | .executable
' "$build_json" | tail -n 1)
[ -n "$bench_bin" ] || fail "Cargo did not report the terminal_state executable"
[ -x "$bench_bin" ] || fail "terminal_state executable is not runnable: $bench_bin"

# The required candidate matrix uses a deliberately shallow workload first.
# Structural reservation, rather than lazy RSS alone, decides whether a
# configuration can ever fit once its scrollback is populated.
run_case candidate_1_120x40_10k 1 40 120 10000 512
run_case candidate_3_120x40_10k 3 40 120 10000 512
run_case candidate_16_120x40_10k 16 40 120 10000 512
run_case candidate_1_512x256_10k 1 256 512 10000 512
run_case candidate_3_512x256_10k 3 256 512 10000 512
run_case candidate_16_512x256_10k 16 256 512 10000 512

# Saturated representatives measure the parser/container overhead which fixed
# cell arithmetic intentionally cannot predict. The accepted recommendation
# must pass both its structural reservation and real 256 MiB process RSS gate.
run_case saturated_3_120x40_10k 3 40 120 10000 10040
run_case saturated_1_512x256_10k 1 256 512 10000 10256
run_case recommended_3_240x80_2k 3 80 240 2000 2080
run_case recommended_8_240x80_2k 8 80 240 2000 2080

terminal_budget_bytes=134217728
host_budget_bytes=268435456

candidate_three_typical=$(result_field saturated_3_120x40_10k 2)
candidate_three_typical_rss=$(result_field saturated_3_120x40_10k 3)
candidate_three_typical_history=$(result_field saturated_3_120x40_10k 4)
candidate_one_large_rss=$(result_field saturated_1_512x256_10k 3)
candidate_one_large_history=$(result_field saturated_1_512x256_10k 4)
recommended_three_structural=$(result_field recommended_3_240x80_2k 2)
recommended_three_rss=$(result_field recommended_3_240x80_2k 3)
recommended_three_history=$(result_field recommended_3_240x80_2k 4)
recommended_eight_structural=$(result_field recommended_8_240x80_2k 2)
recommended_eight_rss=$(result_field recommended_8_240x80_2k 3)
recommended_eight_history=$(result_field recommended_8_240x80_2k 4)
candidate_three_large=$(result_field candidate_3_512x256_10k 2)
candidate_sixteen_typical=$(result_field candidate_16_120x40_10k 2)

[ "$candidate_three_typical_history" -eq 10000 ] ||
    fail "three-session saturated candidate retained $candidate_three_typical_history of 10000 history rows"
[ "$candidate_one_large_history" -eq 10000 ] ||
    fail "large saturated candidate retained $candidate_one_large_history of 10000 history rows"
[ "$recommended_three_history" -eq 2000 ] ||
    fail "three recommended sessions retained $recommended_three_history of 2000 history rows"
[ "$recommended_eight_history" -eq 2000 ] ||
    fail "eight recommended sessions retained $recommended_eight_history of 2000 history rows"
[ "$candidate_three_typical" -le "$terminal_budget_bytes" ] ||
    fail "three saturated 120x40/10k sessions exceed the 128 MiB terminal reservation budget"
[ "$candidate_three_typical_rss" -le "$host_budget_bytes" ] ||
    fail "three saturated 120x40/10k sessions exceed the 256 MiB host RSS budget"
[ "$recommended_three_structural" -le "$terminal_budget_bytes" ] ||
    fail "three recommended sessions exceed the 128 MiB terminal reservation budget"
[ "$recommended_eight_structural" -le "$terminal_budget_bytes" ] ||
    fail "eight recommended sessions exceed the 128 MiB terminal reservation budget"
[ "$recommended_three_rss" -le "$host_budget_bytes" ] ||
    fail "three recommended sessions exceed the 256 MiB host RSS budget"
[ "$recommended_eight_rss" -le "$host_budget_bytes" ] ||
    fail "eight recommended sessions exceed the 256 MiB host RSS budget"
[ "$candidate_one_large_rss" -gt "$host_budget_bytes" ] ||
    fail "one saturated 512x256/10k session should exceed the 256 MiB host RSS budget"
[ "$candidate_three_large" -gt "$terminal_budget_bytes" ] ||
    fail "three 512x256/10k sessions should exceed the 128 MiB structural reservation"
[ "$candidate_sixteen_typical" -gt "$terminal_budget_bytes" ] ||
    fail "sixteen 120x40/10k sessions should exceed the 128 MiB structural reservation"

cleanup
trap - EXIT HUP INT TERM
[ ! -e "$test_root" ] || fail "temporary resource root remains: $test_root"

echo "TERMINAL_RESOURCE_GATE=PASS"
echo "RESOURCE_TEMP_CLEANUP=PASS"
