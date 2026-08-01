#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
if [[ -n "${TERM_INTEROP_BIN:-}" ]]; then
    preview_bin=$TERM_INTEROP_BIN
else
    cargo build --manifest-path "$project_dir/Cargo.toml" -p terminal-interop-cli
    target_dir=$(cargo metadata \
        --manifest-path "$project_dir/Cargo.toml" \
        --no-deps --format-version 1 | jq -r '.target_directory')
    preview_bin="$target_dir/debug/term-interop"
fi
run_dir=$(mktemp -d /var/tmp/terminal-interop-intent-e2e.XXXXXX)
runtime_dir=$(mktemp -d /var/tmp/ti-intent-runtime.XXXXXX)
endpoint=0123456789abcdef0123456789abcdef
target="$run_dir/agent artifact.png"
listener_stdout="$run_dir/listener.jsonl"
listener_stderr="$run_dir/listener.stderr"

cleanup() {
    if [[ -n "${listener_pid:-}" ]]; then
        kill "$listener_pid" 2>/dev/null || true
        wait "$listener_pid" 2>/dev/null || true
    fi
    rm -rf -- "$runtime_dir"
}
trap cleanup EXIT

printf 'fixture\n' >"$target"

XDG_RUNTIME_DIR="$runtime_dir" "$preview_bin" intent listen --once "$endpoint" \
    >"$listener_stdout" 2>"$listener_stderr" &
listener_pid=$!

for _ in $(seq 1 100); do
    if [[ -s "$listener_stdout" ]]; then
        break
    fi
    if ! kill -0 "$listener_pid" 2>/dev/null; then
        wait "$listener_pid"
    fi
    sleep 0.01
done

ready_schema=$(sed -n '1p' "$listener_stdout" | jq -r '.schema')
if [[ "$ready_schema" != urn:terminal-interop:intent-ready:v1 ]]; then
    printf 'listener did not emit a ready receipt\n' >&2
    exit 1
fi

uri=$("$preview_bin" intent uri "$endpoint" "$target")
receipt=$(XDG_RUNTIME_DIR="$runtime_dir" "$preview_bin" intent dispatch "$uri")
wait "$listener_pid"
listener_pid=

if [[ $(jq -r '.state' <<<"$receipt") != forwarded ]]; then
    printf 'dispatcher did not receive a forwarding receipt\n' >&2
    exit 1
fi

intent=$(sed -n '2p' "$listener_stdout")
if [[ $(jq -r '.schema' <<<"$intent") != urn:terminal-interop:open-intent:v1 ]]; then
    printf 'listener did not forward a typed open intent\n' >&2
    exit 1
fi

if XDG_RUNTIME_DIR="$runtime_dir" "$preview_bin" intent dispatch --quiet "$uri" \
    >"$run_dir/stale.stdout" 2>"$run_dir/stale.stderr"; then
    printf 'stale endpoint unexpectedly accepted a second callback\n' >&2
    exit 1
fi

jq -n \
    --arg schema 'urn:terminal-interop:intent-callback-e2e:v1' \
    --arg run_dir "$run_dir" \
    --arg uri "$uri" \
    --arg endpoint "$endpoint" \
    --arg target "$target" \
    --arg ready_schema "$ready_schema" \
    --arg receipt_state "$(jq -r '.state' <<<"$receipt")" \
    --arg intent_schema "$(jq -r '.schema' <<<"$intent")" \
    '{
        schema: $schema,
        run_dir: $run_dir,
        uri: $uri,
        endpoint: $endpoint,
        target: $target,
        assertions: {
            listener_ready: ($ready_schema == "urn:terminal-interop:intent-ready:v1"),
            dispatcher_forwarded: ($receipt_state == "forwarded"),
            typed_intent_received: ($intent_schema == "urn:terminal-interop:open-intent:v1"),
            stale_endpoint_failed_closed: true
        }
    }' | tee "$run_dir/summary.json"
