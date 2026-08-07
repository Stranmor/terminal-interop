# Contributing

Contributions are welcome when they preserve the separation between semantic
contracts, protocol adapters, transport adapters, and consumer integrations.

The repository pins its development formatter and linter in `rust-toolchain.toml`; the separate
MSRV job proves that library and CLI code still builds and tests on Rust 1.90.0.

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
shellcheck install.sh scripts/zv scripts/*.sh tests/*.sh tests/fixtures/*.sh
./tests/e2e-contracts.sh
./tests/e2e-contract-bundle.sh
cargo deny check
```

The real-terminal suite additionally requires Kitty, tmux, Zellij, `jq`, and
GNU `timeout`:

```bash
./tests/e2e-hidden-kitty.sh
./tests/e2e-negotiation-kitty.sh
./tests/e2e-preview-kitty.sh
./tests/e2e-preview-text.sh
```

It creates isolated sessions and writes evidence under `/var/tmp`; it must not
attach to an existing user session.

Sixel framebuffer verification additionally requires an explicitly supplied Alacritty binary:

```bash
TERM_INTEROP_ALACRITTY_BIN=/path/to/alacritty ./tests/e2e-preview-sixel.sh
```

SSH transport verification additionally requires an already authorized BatchMode target and an
absolute remote CLI path. The test creates and removes only its own validated temporary directory:

```bash
TERM_INTEROP_SSH_TARGET=user@host \
TERM_INTEROP_SSH_BIN=/absolute/remote/path/to/term-interop \
TERM_INTEROP_ALACRITTY_BIN=/path/to/sixel-terminal \
    ./tests/e2e-preview-ssh.sh
```

## Contract changes

- Preserve exact wire evidence separately from interpretation.
- Represent absence of evidence as `unknown`, not `false`.
- Keep vendor or product behavior in thin adapters.
- Add black-box conformance evidence for every compatibility claim.
- Version incompatible schema or semantic changes explicitly.
- Regenerate `contracts/v1/schemas` and add both positive and adversarial vectors for every
  contract change.
- Do not include personal paths, hostnames, credentials, or private logs in a
  fixture, example, issue, or commit.

## Useful contributions

The highest-value changes usually add one of these without broadening an existing interface:

- a protocol parser/renderer adapter with bounded malformed-input tests;
- a transport adapter whose readiness evidence is separate from the inner protocol;
- an implementation of the published schemas in another language;
- an exact-chain conformance receipt for a previously unknown terminal or multiplexer path; or
- a TUI integration that demonstrates the pause, child ownership, restore, and redraw lifecycle.

Open a compatibility report before adding a product-name heuristic. A new allowlist entry is not a
substitute for a live probe or a typed unknown state.

Release mechanics are documented in [RELEASING.md](RELEASING.md).
