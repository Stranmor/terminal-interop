# Contributing

Contributions are welcome when they preserve the separation between semantic
contracts, protocol adapters, transport adapters, and consumer integrations.

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
shellcheck install.sh scripts/zv tests/*.sh tests/fixtures/*.sh
```

The real-terminal suite additionally requires Kitty, tmux, Zellij, `jq`, and
GNU `timeout`:

```bash
./tests/e2e-hidden-kitty.sh
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
- Do not include personal paths, hostnames, credentials, or private logs in a
  fixture, example, issue, or commit.
