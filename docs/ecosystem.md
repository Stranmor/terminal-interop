# Ecosystem position

Terminal Interop does not aim to replace mature terminal image renderers. It owns a different
layer: safe artifact identity, live capability evidence, deterministic selection, same-TTY
lifecycle, callback routing, and compatibility receipts.

## When another tool is enough

Use a direct viewer when a human already has a path and only needs to display an image:

- [Chafa](https://github.com/hpjansson/chafa) supports many graphics and character formats and has
  a reusable rendering library.
- [viu](https://github.com/atanunq/viu) is a compact Rust image viewer with native terminal graphics
  support and a character fallback.
- terminal-owned helpers such as Kitty's `icat` or WezTerm's `imgcat` are excellent when the
  terminal family is already fixed.
- [ueberzugpp](https://github.com/jstkdng/ueberzugpp) is useful for overlay-style image placement and
  file-manager integrations.

## When Terminal Interop adds value

Use Terminal Interop when the producer and consumer are separate, the path may cross chat or a
tool boundary, a multiplexer or SSH hop changes the active capabilities, another TUI owns the
terminal lifecycle, or a compatibility claim must be inspectable after the fact.

The reference implementation currently includes its own KGP and Sixel adapters so the end-to-end
contract is executable. A future adapter can delegate rendering to an existing viewer without
changing artifact references, negotiation receipts, or the parent TUI handoff.

## Deliberate non-goals

- becoming a universal image decoder or media player;
- inferring support from a growing terminal-name allowlist;
- hiding low-fidelity character output behind a successful pixel-preview result;
- owning desktop window management or selecting a terminal process heuristically;
- defining application-specific artifact authorization policy; or
- collapsing every protocol and transport into one lowest-common-denominator interface.
