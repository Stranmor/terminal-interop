# Releasing

A release has three distinct surfaces: repository source, Rust crates, and the reference CLI
archive. Keep them on one version while the workspace is pre-1.0.

## Preconditions

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
shellcheck install.sh scripts/zv tests/*.sh tests/fixtures/*.sh
./tests/e2e-contracts.sh
cargo deny check
cargo package --workspace --no-verify
```

Run every real-consumer test required by the compatibility claims in the release notes. A green
portable CI job does not replace terminal, multiplexer, framebuffer, callback, or SSH evidence.

Confirm that `CHANGELOG.md`, `Cargo.lock`, schema identities, the compatibility table, public demo
evidence, and workspace versions describe the same snapshot. Inspect package archives for private
paths or logs before publishing.

## Crates.io dependency order

Publish only crates intended for the release, in dependency order. Wait for each crate to become
available in the registry before publishing a dependent crate.

```text
terminal-interop-core
terminal-interop-geometry
terminal-interop-intent
terminal-interop-da1
terminal-interop-tmux
terminal-interop-artifact
terminal-interop-ref
terminal-interop-kgp
terminal-interop-sixel
terminal-interop-cli
```

`cargo package --workspace --no-verify` checks the archives before the internal dependencies exist
on crates.io. After each dependency is published, run `cargo publish --dry-run -p CRATE` for the
next crate before the real publish command.

Publishing crates is intentionally not automated: it is an irreversible registry action and needs
the maintainer to review the exact package set and order.

## GitHub release

Push an annotated `vX.Y.Z` tag only after source and crate state are final. The Release workflow
builds the Linux x86_64 CLI archive from `Cargo.lock`, includes both licenses and `zv`, writes a
SHA-256 sidecar, and creates or updates the matching GitHub release.

The archive is a reference binary, not a claim of support for every Linux distribution or terminal
chain. Keep platform and compatibility boundaries explicit in the release notes.
