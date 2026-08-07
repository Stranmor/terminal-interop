#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(CDPATH='' cd -- "$script_dir/.." && pwd)

for required_command in cargo jq; do
    if ! command -v "$required_command" >/dev/null 2>&1; then
        printf 'missing required command: %s\n' "$required_command" >&2
        exit 1
    fi
done

run_dir=$(mktemp -d /var/tmp/terminal-interop-install-e2e.XXXXXX)
test_home="$run_dir/home"
prefix="$run_dir/prefix"
cargo_home=${CARGO_HOME:-${HOME:?HOME is required}/.cargo}
rustup_home=${RUSTUP_HOME:-${HOME:?HOME is required}/.rustup}
mkdir -p "$test_home"

env -u XDG_DATA_HOME -u XDG_CONFIG_HOME \
    HOME="$test_home" \
    CARGO_HOME="$cargo_home" \
    RUSTUP_HOME="$rustup_home" \
    PREFIX="$prefix" \
    "$repo_dir/install.sh" >"$run_dir/install.stdout" 2>"$run_dir/install.stderr"

desktop="$prefix/share/applications/terminal-interop-intent.desktop"
if [[ ! -f "$desktop" ]]; then
    printf 'custom-prefix install did not keep its desktop entry inside the prefix\n' >&2
    exit 1
fi
if [[ -e "$test_home/.local/share/applications/terminal-interop-intent.desktop" ]]; then
    printf 'custom-prefix install escaped into the user data home\n' >&2
    exit 1
fi
if [[ -e "$test_home/.config/mimeapps.list" ]]; then
    printf 'custom-prefix install rewrote the user MIME defaults\n' >&2
    exit 1
fi

expected_exec="$prefix/bin/term-interop"
actual_exec=$(sed -n 's/^Exec="\([^"]*\)".*/\1/p' "$desktop")
if [[ "$actual_exec" != "$expected_exec" ]] || [[ ! -x "$actual_exec" ]]; then
    printf 'desktop handler does not reference the installed stable entrypoint\n' >&2
    exit 1
fi

resolved=$(readlink -f "$actual_exec")
case "$resolved" in
    "$prefix"/libexec/terminal-interop/releases/*/term-interop) ;;
    *)
        printf 'installed entrypoint does not resolve to a content-addressed release\n' >&2
        exit 1
        ;;
esac

jq -n \
    --arg schema 'urn:terminal-interop:install-isolation-e2e:v1' \
    --arg generated_at "$(date --utc +%Y-%m-%dT%H:%M:%SZ)" \
    --arg prefix "$prefix" \
    --arg desktop "$desktop" \
    --arg exec "$actual_exec" \
    --arg resolved "$resolved" \
    '{
        schema: $schema,
        generated_at: $generated_at,
        custom_prefix: $prefix,
        desktop: { path: $desktop, exec: $exec },
        release: { resolved_binary: $resolved, content_addressed: true },
        isolation: { user_data_untouched: true, user_mime_defaults_untouched: true }
    }' | tee "$run_dir/summary.json"

printf 'INSTALL_ISOLATION_E2E_ARTIFACTS=%s\n' "$run_dir"
