#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
install_prefix=${PREFIX:-${HOME:?HOME is required}/.local}
bin_dir="$install_prefix/bin"
release_root="$install_prefix/libexec/terminal-interop/releases"
user_data_home=${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}
data_home=${XDG_DATA_HOME:-$install_prefix/share}
applications_dir="$data_home/applications"
intent_desktop_name="terminal-interop-intent.desktop"

cargo build \
    --manifest-path "$script_dir/Cargo.toml" \
    --release \
    --locked \
    -p terminal-interop-cli

built_binary="$script_dir/target/release/term-interop"
binary_sha=$(sha256sum "$built_binary" | awk '{print $1}')
release_dir="$release_root/$binary_sha"
release_binary="$release_dir/term-interop"

mkdir -p "$release_root" "$bin_dir" "$applications_dir"
if [[ ! -e "$release_binary" ]]; then
    release_candidate=$(mktemp -d "$release_root/.candidate.XXXXXX")
    cleanup_candidate() {
        rm -rf -- "$release_candidate"
    }
    trap cleanup_candidate EXIT
    install -Dm755 "$built_binary" "$release_candidate/term-interop"
    printf '%s  term-interop\n' "$binary_sha" >"$release_candidate/SHA256SUMS"
    mv -- "$release_candidate" "$release_dir"
    trap - EXIT
elif ! cmp -s "$built_binary" "$release_binary"; then
    printf 'content-address collision at %s\n' "$release_dir" >&2
    exit 1
fi

term_link=$(mktemp "$bin_dir/.term-interop.XXXXXX")
zv_candidate=$(mktemp "$bin_dir/.zv.XXXXXX")
desktop_candidate=$(mktemp "$applications_dir/.terminal-interop-intent.desktop.XXXXXX")
cleanup_activation() {
    rm -f -- "$term_link" "$zv_candidate" "$desktop_candidate"
}
trap cleanup_activation EXIT
rm -f -- "$term_link"
ln -s "$release_binary" "$term_link"
install -m755 "$script_dir/scripts/zv" "$zv_candidate"
# The single-quoted sed expressions intentionally escape desktop Exec metacharacters literally.
# shellcheck disable=SC2016
desktop_exec=$(printf '%s' "$bin_dir/term-interop" | sed \
    -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/`/\\`/g' -e 's/\$/\\$/g')
{
    printf '%s\n' '[Desktop Entry]'
    printf '%s\n' 'Type=Application'
    printf '%s\n' 'Name=Terminal Interop Intent'
    printf '%s\n' 'Comment=Return a terminal hyperlink to its exact local interactive consumer'
    printf 'Exec="%s" intent dispatch --quiet %%u\n' "$desktop_exec"
    printf '%s\n' 'NoDisplay=true'
    printf '%s\n' 'Terminal=false'
    printf '%s\n' 'MimeType=x-scheme-handler/terminal-interop-intent;'
    printf '%s\n' 'StartupNotify=false'
} >"$desktop_candidate"
mv -Tf "$term_link" "$bin_dir/term-interop"
mv -Tf "$zv_candidate" "$bin_dir/zv"
mv -Tf "$desktop_candidate" "$applications_dir/$intent_desktop_name"
trap - EXIT

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications_dir"
fi

# A custom PREFIX is also used by package builds and release-smoke tests. Do not
# rewrite the live user's default URI handler from such an isolated install.
# Registration happens automatically only when the desktop entry is installed
# into the active user's XDG data home; callers can explicitly opt in or out.
register_handler=${TERM_INTEROP_REGISTER_HANDLER:-auto}
if [[ "$register_handler" == auto ]]; then
    if [[ "$data_home" == "$user_data_home" ]]; then
        register_handler=1
    else
        register_handler=0
    fi
fi
if [[ "$register_handler" != 0 && "$register_handler" != 1 ]]; then
    printf 'TERM_INTEROP_REGISTER_HANDLER must be auto, 0, or 1\n' >&2
    exit 1
fi

if [[ "$register_handler" == 1 ]] && command -v xdg-mime >/dev/null 2>&1; then
    xdg-mime default "$intent_desktop_name" x-scheme-handler/terminal-interop-intent
    actual_intent_handler=$(xdg-mime query default x-scheme-handler/terminal-interop-intent)
    if [[ "$actual_intent_handler" != "$intent_desktop_name" ]]; then
        printf 'failed to activate terminal intent handler: %s\n' "$actual_intent_handler" >&2
        exit 1
    fi
fi

"$bin_dir/term-interop" --version
printf 'installed term-interop -> %s\n' "$release_binary"
printf 'installed intent handler -> %s\n' "$applications_dir/$intent_desktop_name"
if [[ "$register_handler" == 1 ]]; then
    printf 'activated intent handler -> %s\n' "$intent_desktop_name"
else
    printf 'intent handler activation skipped for non-user data prefix\n'
fi
