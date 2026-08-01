#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
preview_bin=${TERM_INTEROP_BIN:-"$project_dir/target/release/term-interop"}
runner="$script_dir/fixtures/run-preview-and-restore.sh"

for required_command in cargo jq kitty; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done
if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build --manifest-path "$project_dir/Cargo.toml" --release --locked -p terminal-interop-cli
fi

artifact_parent=${TERM_INTEROP_E2E_DIR:-/var/tmp}
mkdir -p "$artifact_parent"
run_dir=$(mktemp -d "$artifact_parent/terminal-interop-text-e2e.XXXXXX")
fixture="$run_dir/fixture.txt"
started="$run_dir/started"
start_fifo="$run_dir/start.fifo"
restored_marker="$run_dir/restored.marker"
socket="$run_dir/kitty.sock"
live_text="$run_dir/live.txt"
after_enter_text="$run_dir/after-enter.txt"
restored_text="$run_dir/restored.txt"

printf 'TEXT PREVIEW SAFE\nescape follows: \033[31mNOT_RED\nwide text: Привет 世界\n' >"$fixture"
mkfifo "$start_fifo"

kitty_pid=
cleanup() {
    if [[ -n "$kitty_pid" ]] && kill -0 "$kitty_pid" 2>/dev/null; then
        kill "$kitty_pid" 2>/dev/null || true
        wait "$kitty_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

env \
    TERM_INTEROP_BIN="$preview_bin" \
    TERM_INTEROP_FIXTURE="$fixture" \
    TERM_INTEROP_RESTORED="$restored_marker" \
    TERM_INTEROP_STARTED="$started" \
    TERM_INTEROP_START_FIFO="$start_fifo" \
    TERM_INTEROP_DURATION_MS=8000 \
    kitty --config NONE \
        --override allow_remote_control=socket-only \
        --listen-on "unix:$socket" \
        --start-as hidden \
        -- "$runner" \
        >"$run_dir/kitty.stdout" 2>"$run_dir/kitty.stderr" &
kitty_pid=$!

for _ in $(seq 1 180); do
    [[ -S "$socket" && -s "$started" ]] && break
    sleep 0.05
done
if [[ ! -S "$socket" || ! -s "$started" ]]; then
    printf 'text preview consumer did not become ready\n' >&2
    exit 1
fi
printf 'render\n' >"$start_fifo"

for _ in $(seq 1 180); do
    if kitty @ --to "unix:$socket" get-text --match all >"$live_text" 2>/dev/null \
        && rg -q --fixed-strings 'TEXT PREVIEW SAFE' "$live_text"; then
        break
    fi
    sleep 0.05
done
rg -q --fixed-strings 'TEXT PREVIEW SAFE' "$live_text"
rg -q --fixed-strings '␛[31mNOT_RED' "$live_text"
rg -q --fixed-strings 'Привет 世界' "$live_text"

kitty @ --to "unix:$socket" send-text --match all '\r'
sleep 0.4
if [[ -s "$restored_marker" ]]; then
    printf 'Enter closed the text preview, but it must remain available to the parent composer\n' >&2
    exit 1
fi
kitty @ --to "unix:$socket" get-text --match all >"$after_enter_text"
rg -q --fixed-strings 'TEXT PREVIEW SAFE' "$after_enter_text"

kitty @ --to "unix:$socket" send-text --match all 'q'
for _ in $(seq 1 180); do
    [[ -s "$restored_marker" ]] && break
    sleep 0.05
done
if [[ ! -s "$restored_marker" ]]; then
    printf 'q did not restore the text preview caller\n' >&2
    exit 1
fi
kitty @ --to "unix:$socket" get-text --match all >"$restored_text"
rg -q --fixed-strings 'TERM_INTEROP_RESTORED' "$restored_text"

wait "$kitty_pid"
kitty_pid=
if [[ -s "$run_dir/kitty.stderr" ]]; then
    printf 'Kitty emitted errors:\n' >&2
    sed -n '1,160p' "$run_dir/kitty.stderr" >&2
    exit 1
fi

jq -n \
    --arg schema 'urn:terminal-interop:text-preview-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg kitty_version "$(kitty --version)" \
    --arg live_text "$live_text" \
    --arg after_enter_text "$after_enter_text" \
    --arg restored_text "$restored_text" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        implementation: $kitty_version,
        assertions: {
            text_visible: true,
            escape_rendered_as_text: true,
            unicode_visible: true,
            enter_does_not_close: true,
            q_closes_and_restores: true
        },
        evidence: {
            live_text: $live_text,
            after_enter_text: $after_enter_text,
            restored_text: $restored_text
        }
    }' | tee "$run_dir/summary.json"

printf 'TEXT_E2E_ARTIFACTS=%s\n' "$run_dir"
