# Contributing

Contributions are welcome when they preserve the separation between semantic
contracts, protocol adapters, transport adapters, and consumer integrations.

## Development checks

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
shellcheck tests/e2e-hidden-kitty.sh tests/fixtures/run-kgp-probe.sh
```

The real-terminal suite additionally requires Kitty, tmux, Zellij, `jq`, and
GNU `timeout`:

```bash
./tests/e2e-hidden-kitty.sh
```

It creates isolated sessions and writes evidence under `/var/tmp`; it must not
attach to an existing user session.

## Contract changes

- Preserve exact wire evidence separately from interpretation.
- Represent absence of evidence as `unknown`, not `false`.
- Keep vendor or product behavior in thin adapters.
- Add black-box conformance evidence for every compatibility claim.
- Version incompatible schema or semantic changes explicitly.
- Do not include personal paths, hostnames, credentials, or private logs in a
  fixture, example, issue, or commit.
