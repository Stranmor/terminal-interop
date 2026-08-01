#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
probe_bin=${TERM_INTEROP_BIN:-"$project_dir/target/release/term-interop"}
runner="$script_dir/fixtures/run-kgp-probe.sh"
zellij_layout="$script_dir/fixtures/zellij-kgp-probe.kdl"

for required_command in cargo jq kitty timeout tmux zellij; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

cargo build --manifest-path "$project_dir/Cargo.toml" --release -p terminal-interop-cli

artifact_parent=${TERM_INTEROP_E2E_DIR:-/var/tmp}
mkdir -p "$artifact_parent"
run_dir=$(mktemp -d "$artifact_parent/terminal-interop-e2e.XXXXXX")
summary_lines="$run_dir/summary.ndjson"
summary="$run_dir/summary.json"

active_kitty_pid=
active_tmux_socket=
active_zellij_session=

cleanup() {
    if [[ -n "$active_kitty_pid" ]] && kill -0 "$active_kitty_pid" 2>/dev/null; then
        kill "$active_kitty_pid" 2>/dev/null || true
        wait "$active_kitty_pid" 2>/dev/null || true
    fi
    if [[ -n "$active_tmux_socket" ]]; then
        tmux -L "$active_tmux_socket" kill-server >/dev/null 2>&1 || true
    fi
    if [[ -n "$active_zellij_session" ]]; then
        zellij delete-session "$active_zellij_session" --force >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

wait_for_file() {
    local path=$1
    local attempts=${2:-200}
    local attempt
    for ((attempt = 0; attempt < attempts; attempt += 1)); do
        [[ -s "$path" ]] && return 0
        sleep 0.05
    done
    return 1
}

wait_for_tmux_client() {
    local socket=$1
    local attempts=${2:-200}
    local attempt
    for ((attempt = 0; attempt < attempts; attempt += 1)); do
        tmux -L "$socket" list-clients >/dev/null 2>&1 && return 0
        sleep 0.05
    done
    return 1
}

wait_for_zellij_client() {
    local session=$1
    local attempts=${2:-200}
    local attempt
    for ((attempt = 0; attempt < attempts; attempt += 1)); do
        zellij --session "$session" action list-clients >/dev/null 2>&1 && return 0
        sleep 0.05
    done
    return 1
}

validate_receipt() {
    local receipt=$1
    local expected_transport=$2
    local availability_rule=$3
    local expected_readiness=$4

    jq -e \
        --arg transport "$expected_transport" \
        --arg availability_rule "$availability_rule" \
        --arg readiness "$expected_readiness" '
        .schema == "urn:terminal-interop:probe-receipt:v1"
        and .context.transport.adapter.name == $transport
        and .context.transport.readiness == $readiness
        and (.exchange.logical_request_base64 | length > 0)
        and (.exchange.wire_request_base64 | length > 0)
        and (.exchange.response_base64 | length > 0)
        and (
            if $availability_rule == "available" then
                .assessment.availability == "available"
                and .assessment.conformance == "conformant"
                and .exchange.stop_reason == "capability_and_barrier_observed"
            else
                (
                    .assessment.availability == "available"
                    and .assessment.conformance == "conformant"
                    and .exchange.stop_reason == "capability_and_barrier_observed"
                ) or (
                    .assessment.availability == "unavailable"
                    and .assessment.conformance == "not_applicable"
                    and .exchange.stop_reason == "barrier_observed"
                )
            end
        )
    ' "$receipt" >/dev/null
}

record_case() {
    local case_name=$1
    local receipt=$2

    jq --arg case_name "$case_name" '{
        case: $case_name,
        availability: .assessment.availability,
        conformance: .assessment.conformance,
        stop_reason: .exchange.stop_reason,
        transport: .context.transport.adapter.name,
        transport_readiness: .context.transport.readiness,
        transport_preparation_exchanges: (.context.transport.preparation_exchanges | length),
        elapsed_ms: .exchange.elapsed_ms,
        receipt: input_filename
    }' "$receipt" >>"$summary_lines"
}

run_direct_case() {
    local case_name=direct-kitty
    local case_dir="$run_dir/$case_name"
    local receipt="$case_dir/receipt.json"
    local started="$case_dir/started"
    local start_fifo="$case_dir/start.fifo"
    mkdir -p "$case_dir"
    mkfifo "$start_fifo"

    env \
        TERM_INTEROP_BIN="$probe_bin" \
        TERM_INTEROP_RECEIPT="$receipt" \
        TERM_INTEROP_PROBE_LOG="$case_dir/probe.log" \
        TERM_INTEROP_STARTED="$started" \
        TERM_INTEROP_START_FIFO="$start_fifo" \
        TERM_INTEROP_TIMEOUT_MS=2000 \
        timeout 15s kitty --config NONE --start-as hidden -- "$runner" \
        2>"$case_dir/kitty.log" &
    active_kitty_pid=$!

    wait_for_file "$started"
    printf 'start\n' >"$start_fifo"
    wait "$active_kitty_pid"
    active_kitty_pid=

    validate_receipt "$receipt" direct-tty available not_required
    record_case "$case_name" "$receipt"
}

run_tmux_case() {
    local case_name=$1
    local transport=$2
    local availability_rule=$3
    local expected_transport=$4
    local case_dir="$run_dir/$case_name"
    local receipt="$case_dir/receipt.json"
    local started="$case_dir/started"
    local start_fifo="$case_dir/start.fifo"
    local session=probe
    local socket="term-interop-$RANDOM-$$-$case_name"
    mkdir -p "$case_dir"
    mkfifo "$start_fifo"

    active_tmux_socket=$socket
    env \
        TERM_INTEROP_BIN="$probe_bin" \
        TERM_INTEROP_RECEIPT="$receipt" \
        TERM_INTEROP_PROBE_LOG="$case_dir/probe.log" \
        TERM_INTEROP_STARTED="$started" \
        TERM_INTEROP_START_FIFO="$start_fifo" \
        TERM_INTEROP_TRANSPORT="$transport" \
        TERM_INTEROP_TIMEOUT_MS=2000 \
        tmux -L "$socket" -f /dev/null new-session -d -s "$session" "$runner"
    tmux -L "$socket" set-option -p -t "$session":0.0 allow-passthrough all

    timeout 15s kitty --config NONE --start-as hidden -- \
        tmux -L "$socket" attach-session -t "$session" \
        2>"$case_dir/kitty.log" &
    active_kitty_pid=$!

    wait_for_file "$started"
    wait_for_tmux_client "$socket"
    printf 'start\n' >"$start_fifo"
    wait "$active_kitty_pid"
    active_kitty_pid=
    tmux -L "$socket" kill-server >/dev/null 2>&1 || true
    active_tmux_socket=

    if [[ "$expected_transport" == tmux-dcs-passthrough ]]; then
        validate_receipt "$receipt" "$expected_transport" "$availability_rule" ready
    else
        validate_receipt "$receipt" "$expected_transport" "$availability_rule" not_required
    fi
    record_case "$case_name" "$receipt"
}

run_zellij_case() {
    local case_name=zellij-kitty
    local case_dir="$run_dir/$case_name"
    local receipt="$case_dir/receipt.json"
    local started="$case_dir/started"
    local start_fifo="$case_dir/start.fifo"
    local session="term-interop-$RANDOM-$$"
    mkdir -p "$case_dir"
    mkfifo "$start_fifo"

    active_zellij_session=$session
    env \
        TERM_INTEROP_BIN="$probe_bin" \
        TERM_INTEROP_RECEIPT="$receipt" \
        TERM_INTEROP_PROBE_LOG="$case_dir/probe.log" \
        TERM_INTEROP_STARTED="$started" \
        TERM_INTEROP_START_FIFO="$start_fifo" \
        TERM_INTEROP_RUNNER="$runner" \
        TERM_INTEROP_TIMEOUT_MS=2000 \
        zellij attach --create-background "$session" options \
            --default-layout "$zellij_layout" \
            --show-startup-tips false \
            --show-release-notes false

    timeout 15s kitty --config NONE --start-as hidden -- zellij attach "$session" \
        2>"$case_dir/kitty.log" &
    active_kitty_pid=$!

    wait_for_file "$started"
    wait_for_zellij_client "$session"
    printf 'start\n' >"$start_fifo"
    wait "$active_kitty_pid"
    active_kitty_pid=
    zellij delete-session "$session" --force >/dev/null 2>&1 || true
    active_zellij_session=

    validate_receipt "$receipt" direct-tty available-or-unavailable not_required
    record_case "$case_name" "$receipt"
}

run_direct_case
run_tmux_case tmux-direct direct available-or-unavailable direct-tty
run_tmux_case tmux-passthrough tmux-passthrough available tmux-dcs-passthrough
run_zellij_case

jq -s \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg kitty_version "$(kitty --version)" \
    --arg tmux_version "$(tmux -V)" \
    --arg zellij_version "$(zellij --version)" \
    '{
        schema: "urn:terminal-interop:e2e-summary:v1",
        generated_at: $generated_at,
        implementations: {
            kitty: $kitty_version,
            tmux: $tmux_version,
            zellij: $zellij_version
        },
        cases: .
    }' "$summary_lines" >"$summary"

jq . "$summary"
printf 'E2E_ARTIFACTS=%s\n' "$run_dir"
