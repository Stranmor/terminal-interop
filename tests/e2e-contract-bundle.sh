#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
project_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)
bundle_dir="$project_dir/contracts/v1"
interop_bin=${TERM_INTEROP_BIN:-"$project_dir/target/debug/term-interop"}

for required_command in cargo jq python3; do
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

temporary=$(mktemp -d "${TERM_INTEROP_E2E_DIR:-/var/tmp}/terminal-interop-contract-bundle.XXXXXX")
cleanup() {
    rm -rf -- "$temporary"
}
trap cleanup EXIT

while IFS=$'\t' read -r command filename; do
    "$interop_bin" schema "$command" --pretty >"$temporary/$filename"
    cmp --silent "$temporary/$filename" "$bundle_dir/schemas/$filename" || {
        printf 'checked-in schema is stale: %s\n' "$filename" >&2
        exit 1
    }
done <<'EOF'
receipt	probe-receipt.schema.json
negotiation	capability-negotiation.schema.json
artifact-ref	artifact-ref.schema.json
open-intent	open-intent.schema.json
intent-receipt	intent-receipt.schema.json
intent-ready	intent-ready.schema.json
EOF

jq -e '.format == 1 and (.schemas | length == 6) and (.vectors | length >= 13)' \
    "$bundle_dir/manifest.json" >/dev/null

while IFS=$'\t' read -r schema_id relative_path; do
    schema_file="$bundle_dir/$relative_path"
    jq -e --arg schema_id "$schema_id" \
        '."$id" == $schema_id and .properties.schema.const == $schema_id' \
        "$schema_file" >/dev/null
done < <(jq -r '.schemas[] | [.id, .path] | @tsv' "$bundle_dir/manifest.json")

manifest_vector_count=$(jq '.vectors | length' "$bundle_dir/manifest.json")
fixture_count=$(find "$bundle_dir/fixtures" -type f -name '*.json' -printf '.' | wc -c)
if [[ "$manifest_vector_count" -ne "$fixture_count" ]]; then
    printf 'manifest vectors do not inventory every fixture\n' >&2
    exit 1
fi

while IFS=$'\t' read -r expected relative_path; do
    document="$bundle_dir/$relative_path"
    case "$expected" in
        valid)
            "$interop_bin" validate --quiet "$document"
            ;;
        invalid)
            if "$interop_bin" validate --quiet "$document" >"$temporary/invalid.out" 2>&1; then
                printf 'invalid vector was accepted: %s\n' "$relative_path" >&2
                exit 1
            fi
            ;;
        *)
            printf 'unknown vector expectation: %s\n' "$expected" >&2
            exit 1
            ;;
    esac
done < <(jq -r '.vectors[] | [.expected, .path] | @tsv' "$bundle_dir/manifest.json")

"$interop_bin" validate --quiet < "$bundle_dir/fixtures/valid/artifact-ref.json"
python3 "$project_dir/examples/consume-negotiation.py" \
    "$bundle_dir/fixtures/valid/negotiation-selected.json" \
    | jq -e '.state == "selected" and .preference == 0' >/dev/null

if python3 "$project_dir/examples/consume-negotiation.py" \
    "$bundle_dir/fixtures/invalid/negotiation-forged-selection.json" \
    >"$temporary/python-invalid.out" 2>&1; then
    printf 'independent Python consumer accepted a forged selection\n' >&2
    exit 1
fi

jq -n \
    --arg implementation "$($interop_bin --version)" \
    '{
        schema: "urn:terminal-interop:contract-bundle-e2e:v1",
        implementation: $implementation,
        checked_in_schemas_match: true,
        reference_vectors_pass: true,
        independent_python_consumer_pass: true
    }'
