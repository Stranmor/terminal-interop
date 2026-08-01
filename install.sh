#!/usr/bin/env bash
set -euo pipefail

script_dir=$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
install_prefix=${PREFIX:-${HOME:?HOME is required}/.local}
bin_dir="$install_prefix/bin"
release_root="$install_prefix/libexec/terminal-interop/releases"

cargo build \
    --manifest-path "$script_dir/Cargo.toml" \
    --release \
    --locked \
    -p terminal-interop-cli

built_binary="$script_dir/target/release/term-interop"
binary_sha=$(sha256sum "$built_binary" | awk '{print $1}')
release_dir="$release_root/$binary_sha"
release_binary="$release_dir/term-interop"

mkdir -p "$release_root" "$bin_dir"
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
cleanup_activation() {
    rm -f -- "$term_link" "$zv_candidate"
}
trap cleanup_activation EXIT
rm -f -- "$term_link"
ln -s "$release_binary" "$term_link"
install -m755 "$script_dir/scripts/zv" "$zv_candidate"
mv -Tf "$term_link" "$bin_dir/term-interop"
mv -Tf "$zv_candidate" "$bin_dir/zv"
trap - EXIT

"$bin_dir/term-interop" --version
printf 'installed term-interop -> %s\n' "$release_binary"
