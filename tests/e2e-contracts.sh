#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
interop_bin=${TERM_INTEROP_BIN:-"$project_dir/target/debug/term-interop"}

for required_command in cargo jq; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build \
        --manifest-path "$project_dir/Cargo.toml" \
        --locked \
        -p terminal-interop-cli
fi

run_dir=$(mktemp -d "${TERM_INTEROP_E2E_DIR:-/var/tmp}/terminal-interop-contracts.XXXXXX")
cleanup() {
    rm -rf -- "$run_dir"
}
trap cleanup EXIT

"$interop_bin" schema receipt >"$run_dir/probe.json"
"$interop_bin" schema negotiation >"$run_dir/negotiation.json"
"$interop_bin" schema artifact-ref >"$run_dir/artifact-ref.json"
"$interop_bin" schema open-intent >"$run_dir/open-intent.json"
"$interop_bin" schema intent-receipt >"$run_dir/intent-receipt.json"
"$interop_bin" schema intent-ready >"$run_dir/intent-ready.json"

jq -e '.title == "ProbeReceiptV1"' "$run_dir/probe.json" >/dev/null
jq -e '.title == "CapabilityNegotiationV1"' "$run_dir/negotiation.json" >/dev/null
jq -e '.title == "ArtifactRefV1"' "$run_dir/artifact-ref.json" >/dev/null
jq -e '.title == "OpenIntentV1"' "$run_dir/open-intent.json" >/dev/null
jq -e '.title == "IntentReceiptV1"' "$run_dir/intent-receipt.json" >/dev/null
jq -e '.title == "IntentReadyV1"' "$run_dir/intent-ready.json" >/dev/null

printf 'contract fixture\n' >"$run_dir/artifact.txt"
TERM_INTEROP_STATE_DIR="$run_dir/state" \
    "$interop_bin" offer --format json "$run_dir/artifact.txt" >"$run_dir/offered.json"

jq -e '
    .schema == "urn:terminal-interop:artifact-ref:v1"
    and (.token | length == 13)
    and .path_encoding == "unix-bytes-v1"
    and (.path_base64 | length > 0)
    and .identity.size == 17
    and (.identity.content_sha256_base64 | length > 0)
' "$run_dir/offered.json" >/dev/null

jq -n \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg version "$("$interop_bin" --version)" \
    '{
        schema: "urn:terminal-interop:contract-e2e:v1",
        generated_at: $generated_at,
        implementation: $version,
        schemas: [
            "probe-receipt-v1",
            "capability-negotiation-v1",
            "artifact-ref-v1",
            "open-intent-v1",
            "intent-receipt-v1",
            "intent-ready-v1"
        ],
        artifact_offer_round_trip: true
    }'
