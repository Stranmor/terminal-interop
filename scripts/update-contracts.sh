#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
schema_dir="$project_dir/contracts/v1/schemas"
interop_bin=${TERM_INTEROP_BIN:-"$project_dir/target/debug/term-interop"}

if [[ -z "${TERM_INTEROP_BIN:-}" ]]; then
    cargo build \
        --manifest-path "$project_dir/Cargo.toml" \
        --locked \
        -p terminal-interop-cli
fi

temporary=$(mktemp -d "${TMPDIR:-/tmp}/terminal-interop-contracts.XXXXXX")
cleanup() {
    rm -rf -- "$temporary"
}
trap cleanup EXIT

while IFS=' ' read -r command filename; do
    "$interop_bin" schema "$command" --pretty >"$temporary/$filename"
done <<'EOF'
receipt probe-receipt.schema.json
negotiation capability-negotiation.schema.json
artifact-ref artifact-ref.schema.json
open-intent open-intent.schema.json
intent-receipt intent-receipt.schema.json
intent-ready intent-ready.schema.json
EOF

mkdir -p -- "$schema_dir"
for schema in "$temporary"/*.json; do
    install -m 0644 -- "$schema" "$schema_dir/$(basename "$schema")"
done

printf 'updated %s\n' "$schema_dir"
