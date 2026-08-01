# Terminal Interop

Open agent-linked images and text over SSH without reconstructing wrapped paths, opening a GUI
window, or pretending character art is a pixel preview.

Terminal Interop is an open, terminal-neutral set of versioned contracts plus a Rust reference
implementation. It derives capability from live evidence instead of terminal-name heuristics.

An agent can offer one completed file as an opaque short reference. A human or another tool can
then preview that exact file in the current terminal: sanitized text in a pager, or real pixels
through the best capability observed on the live TTY chain.

```console
$ term-interop offer ./artifacts/design.png
@6W4D9F2K8M7QH

$ term-interop preview @6W4D9F2K8M7QH
# q or Esc returns to the caller; Enter remains untouched
```

The convenience command installed alongside the CLI is shorter:

```console
$ zv @6W4D9F2K8M7QH
```

## What is different

- **No copied path.** `@TOKEN` and `terminal-interop://artifact/TOKEN` preserve exact path bytes in
  a private local registry, so line wrapping cannot corrupt the reference.
- **No character-art image fallback disguised as success.** Raster previews use Kitty Graphics
  Protocol (KGP) or Sixel. If neither live protocol probe succeeds, the CLI fails explicitly.
- **No separate window in the core.** The preview owns the current TTY temporarily, uses the
  alternate screen, and restores its caller after `q` or `Esc`.
- **No `$TERM` capability guesses.** Environment values are bounded hints. Availability and
  protocol conformance come from live wire evidence.
- **No vendor-shaped core.** Artifact validation, references, probes, geometry, transports, KGP,
  Sixel, and the interactive consumer are replaceable crates with narrow contracts.

## Install

Rust 1.90 or newer is required.

```bash
./install.sh
```

The installer builds with `--locked`, creates an immutable content-addressed release below
`$PREFIX/libexec/terminal-interop`, and atomically activates `term-interop` and `zv` in
`$PREFIX/bin`. `PREFIX` defaults to `$HOME/.local`.

To build without installing:

```bash
cargo build --release --locked -p terminal-interop-cli
```

## Commands

```text
term-interop offer PATH [--format short|uri|json]
term-interop preview PATH_OR_REFERENCE [--backend auto|kgp|sixel]
term-interop probe kgp [--transport direct|tmux-passthrough] [--pretty]
term-interop probe sixel [--transport direct|tmux-passthrough] [--pretty]
term-interop schema receipt --pretty
```

Text navigation: `Space`/`j` next page, `k` previous page, `g` first page, `G` last page,
`q`/`Esc` close. Raster previews use `q`/`Esc`; other keys, including Enter, do nothing.

`--transport auto` enables tmux DCS passthrough only when a tmux environment marker is present.
Direct TTY remains the default for terminals, Zellij, and SSH sessions.

## Contracts

```text
agent or producer
    -> artifact-ref-v1 (opaque identity-bound reference)
        -> validated artifact (text or canonical RGBA/PNG)
            -> live capability receipt
                -> protocol renderer (KGP | Sixel)
                    -> transport adapter (direct | tmux DCS)
                        -> current TTY consumer
```

The schemas and semantics are documented independently:

- [Artifact reference v1](docs/artifact-ref-v1.md)
- [Probe receipt v1](docs/probe-receipt-v1.md)
- [Preview profile v1](docs/preview-profile-v1.md)
- [Compatibility evidence](docs/compatibility.md)
- [Consumer integration](docs/integration.md)
- [Reproducible evidence demo](docs/demo.md)

## Security model

- Only an explicitly offered regular file can become a short reference.
- Registration and preview share a 32 MiB encoded-input limit.
- References bind canonical path bytes, device/inode where available, size, modification time, and
  SHA-256; a changed file fails closed rather than silently rebinding.
- Registry directories are private (`0700` on Unix), entries are atomically persisted, and paths
  are data rather than shell source.
- Raster decoding is bounded by dimensions and decoded allocation, then re-encoded as
  metadata-free PNG or RGBA before rendering.
- Text control bytes are sanitized; artifact content is never emitted as terminal control input.

See [SECURITY.md](SECURITY.md) for the reporting and trust boundary.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
shellcheck install.sh scripts/zv tests/*.sh tests/fixtures/*.sh
./tests/e2e-hidden-kitty.sh
./tests/e2e-preview-kitty.sh
./tests/e2e-preview-text.sh
TERM_INTEROP_ALACRITTY_BIN=/path/to/sixel-terminal \
    ./tests/e2e-preview-sixel.sh
TERM_INTEROP_SSH_TARGET=user@host \
TERM_INTEROP_SSH_BIN=/absolute/remote/path/to/term-interop \
TERM_INTEROP_ALACRITTY_BIN=/path/to/sixel-terminal \
    ./tests/e2e-preview-ssh.sh
```

The framebuffer E2E suite launches isolated terminals, renders a high-frequency raster fixture,
captures the live and restored screens, and records machine-readable evidence under `/var/tmp`.
It never attaches to an existing user multiplexer session. Sixel/Alacritty evidence has additional
prerequisites described in [Compatibility evidence](docs/compatibility.md).

## Status

This repository is pre-1.0. Version-one contracts are explicit and additive changes are preferred,
but no ecosystem compatibility promise is implied until a tagged release exists. Unsupported or
untested chains remain `unknown`; they are not converted into optimistic booleans.
