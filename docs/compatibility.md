# Compatibility evidence

Compatibility is attached to exact consumed chains, not product families. This table records the
local black-box evidence snapshot from 2026-08-01; unlisted combinations remain unknown.

| Chain | Capability/render result | Evidence |
|---|---|---|
| `term-interop -> Kitty 0.48.1` | KGP available and conformant; high-detail framebuffer preview; clean restore | `tests/e2e-hidden-kitty.sh`, `tests/e2e-preview-kitty.sh` |
| `term-interop text pager -> Kitty 0.48.1` | sanitized text and Unicode visible; Enter ignored; `q` restores caller | `tests/e2e-preview-text.sh` |
| `term-interop -> tmux 3.7b -> Kitty 0.48.1` direct bytes | KGP unavailable on the direct path | `tests/e2e-hidden-kitty.sh` |
| `term-interop -> tmux 3.7b DCS passthrough -> Kitty 0.48.1` | transport ready; KGP available and conformant | `tests/e2e-hidden-kitty.sh` |
| `term-interop -> Zellij 0.44.3 -> Kitty 0.48.1` | KGP unavailable on this chain | `tests/e2e-hidden-kitty.sh` |
| `term-interop -> Alacritty 0.17.0-dev`, graphics revision `3d658d2e280d` | Sixel advertised; high-detail framebuffer preview; clean restore | `tests/e2e-preview-sixel.sh` |
| `term-interop -> Zellij 0.44.3 -> Alacritty 0.17.0-dev`, graphics revision `3d658d2e280d` | Sixel framebuffer preview and clean restore in an isolated Zellij configuration | `tests/e2e-preview-sixel.sh` |
| `term-interop 0.1.0 -> OpenSSH_10.4p1 PTY -> Alacritty 0.17.0-dev`, graphics revision `3d658d2e280d` | Remote Sixel framebuffer matches the fixture; Enter is inert; `q` restores; one terminal window throughout | `tests/e2e-preview-ssh.sh` |
| `term-interop 0.1.0 text pager -> OpenSSH_10.4p1 PTY -> Kitty 0.48.1` | Remote Unicode text visible; escape bytes inert; Enter is inert; `q` restores | `tests/e2e-preview-ssh.sh` |

The Alacritty row is an exact experimental source revision, not a claim about the stable Alacritty
release line. Supply its binary explicitly when running the suite:

```bash
TERM_INTEROP_ALACRITTY_BIN=/path/to/alacritty ./tests/e2e-preview-sixel.sh
```

The Sixel suite starts fresh Xvfb, Alacritty, and Zellij instances. It does not reuse or mutate a
user session. Its JSON summary keeps implementation versions, framebuffer dimensions, color
counts, screenshots, and restoration evidence.

The SSH rows use a real authenticated OpenSSH connection and allocated remote PTY against an
isolated loopback target. The test copies only generated fixtures and its bounded runner into a
private remote temporary directory, removes that directory afterward, and omits the target name
from its summary. This proves the SSH transport transition on that exact chain; it does not imply
that every mobile SSH client implements KGP or Sixel.

An in-protocol capability reply proves that layer only. The framebuffer tests are the stronger
consumer proof for the exact rows above.
