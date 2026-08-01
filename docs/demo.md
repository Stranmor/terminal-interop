# Reproducible evidence demo

This project treats terminal compatibility as a property of one exact consumed chain. A protocol
reply alone is not the demo: the E2E harnesses capture the real terminal framebuffer or terminal
text, compare it with an independently generated fixture, exercise the close lifecycle, and emit a
machine-readable JSON summary.

## Cold start

```bash
git clone REPOSITORY_URL terminal-interop
cd terminal-interop
./install.sh

reference=$(term-interop offer ./path/to/report.md)
zv "$reference"
```

`REPOSITORY_URL` intentionally remains a placeholder until a repository is actually published.
The installer builds from `Cargo.lock`, installs an immutable content-addressed binary, and
atomically activates `term-interop` and `zv` under the selected prefix.

## Evidence commands

```bash
cargo test --all-targets --locked
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

Every script writes its evidence to a newly allocated directory under `/var/tmp` by default and
prints that directory only to the local caller. Evidence paths, hostnames, and target identities
are not committed or included in the JSON compatibility record.

## Reference snapshot

The following snapshot was measured on August 1, 2026. It demonstrates the strength and shape of
the evidence; it is not a promise about an untested implementation or future release.

| Consumed chain | Consumer evidence |
|---|---|
| Kitty 0.48.1 / KGP | capability available and conformant; 4,014 framebuffer colors; normalized fixture RMSE 0.0259; caller restored |
| tmux 3.7b direct / KGP | unavailable on the direct path, reported explicitly rather than guessed |
| tmux 3.7b DCS passthrough / KGP | transport ready; capability available and conformant |
| Alacritty graphics revision `3d658d2e` / Sixel | 776 colors; normalized fixture RMSE 0.0146; restored frame RMSE 0.3691 |
| Zellij 0.44.3 -> Alacritty graphics revision `3d658d2e` / Sixel | 1,400 colors; normalized fixture RMSE 0.0985; restored frame RMSE 0.3692 |
| OpenSSH_10.4p1 PTY -> remote `term-interop 0.1.0` -> Sixel | 773 colors; fixture RMSE 0.0141; Enter delta 0; restored frame RMSE 0.3669; exactly one window |
| OpenSSH_10.4p1 PTY -> remote text pager | Unicode visible; escape sequence rendered inert; Enter kept the preview open; `q` restored the caller |

The SSH harness emits a redacted record shaped like this:

```json
{
  "schema": "urn:terminal-interop:ssh-preview-e2e:v1",
  "transport": {
    "adapter": "openssh-pty",
    "remote_consumer": "term-interop 0.1.0"
  },
  "image": {
    "renderer": "sixel",
    "unique_colors": 773,
    "normalized_rmse_to_fixture": 0.0141299,
    "normalized_rmse_after_enter": 0,
    "restored_rmse_to_fixture": 0.366932,
    "q_restores": true,
    "separate_window_never_created": true
  },
  "text": {
    "text_visible": true,
    "escape_rendered_as_text": true,
    "unicode_visible": true,
    "enter_does_not_close": true,
    "q_restores": true
  }
}
```

The important invariant is not a particular terminal brand. An adapter is eligible only when live
typed evidence proves the relevant protocol and transport on the current chain. Missing evidence
remains `unknown`; an unavailable protocol is not replaced by low-quality character art and
reported as successful pixel rendering.
