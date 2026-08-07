#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
interop_bin=${TERM_INTEROP_BIN:-"$project_dir/target/release/term-interop"}
runner="$script_dir/fixtures/run-pixel-negotiation.sh"

for required_command in cargo jq kitty timeout; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build \
        --manifest-path "$project_dir/Cargo.toml" \
        --release \
        --locked \
        -p terminal-interop-cli
fi

run_dir=$(mktemp -d "${TERM_INTEROP_E2E_DIR:-/var/tmp}/terminal-interop-negotiation.XXXXXX")
receipt="$run_dir/negotiation.json"
started="$run_dir/started"
start_fifo="$run_dir/start.fifo"
mkfifo "$start_fifo"

active_kitty_pid=
cleanup() {
    if [[ -n "$active_kitty_pid" ]] && kill -0 "$active_kitty_pid" 2>/dev/null; then
        kill "$active_kitty_pid" 2>/dev/null || true
        wait "$active_kitty_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

env \
    TERM_INTEROP_BIN="$interop_bin" \
    TERM_INTEROP_RECEIPT="$receipt" \
    TERM_INTEROP_NEGOTIATION_LOG="$run_dir/negotiation.log" \
    TERM_INTEROP_STARTED="$started" \
    TERM_INTEROP_START_FIFO="$start_fifo" \
    timeout 15s kitty --config NONE --start-as hidden -- "$runner" \
    2>"$run_dir/kitty.log" &
active_kitty_pid=$!

for _ in $(seq 1 200); do
    [[ -s "$started" ]] && break
    sleep 0.05
done
if [[ ! -s "$started" ]]; then
    printf 'negotiation runner did not become ready\n' >&2
    exit 1
fi

printf 'start\n' >"$start_fifo"
wait "$active_kitty_pid"
active_kitty_pid=

jq -e '
    .schema == "urn:terminal-interop:capability-negotiation:v1"
    and (.candidates | length == 2)
    and .candidates[0].preference == 0
    and .candidates[0].disposition == "eligible"
    and .candidates[0].receipt.assessment.availability == "available"
    and .candidates[0].receipt.assessment.conformance == "conformant"
    and .selection.state == "selected"
    and .selection.preference == 0
    and .selection.capability.protocol.name == "terminal-graphics-protocol"
' "$receipt" >/dev/null

jq \
    --arg implementation "$("$interop_bin" --version)" \
    --arg terminal "$(kitty --version)" \
    '{
    schema: "urn:terminal-interop:negotiation-e2e:v1",
    implementation: $implementation,
    terminal: $terminal,
    selection: .selection,
    candidates: [.candidates[] | {
        preference,
        disposition,
        capability: .receipt.capability,
        assessment: .receipt.assessment,
        transport: .receipt.context.transport.readiness
    }]
}' \
    "$receipt"

printf 'E2E_ARTIFACTS=%s\n' "$run_dir"
