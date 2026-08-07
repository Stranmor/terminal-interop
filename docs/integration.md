# Consumer integration

Terminal Interop exposes processes and data contracts rather than requiring an application to link
all Rust crates.

## Capability negotiation

Before committing to a pixel renderer, obtain an ordered negotiation receipt:

```bash
term-interop negotiate pixel --pretty --output ./capability.json
```

The reference profile probes KGP and then Sixel. Embedding applications may compose the protocol
crates directly, supply a different order, or add another adapter. They should apply
`receipt_is_eligible`/`negotiate_capabilities_v1` or preserve the same semantics in another
language: availability, conformance, and transport readiness must all be positive.

For process-only integrations, validate any stored or externally received document before using
it as actuation input:

```bash
term-interop validate --quiet ./capability.json
```

The checked-in [`contracts/v1`](../contracts/README.md) bundle lets another language vendor the
schemas and run the same positive and adversarial vectors without compiling the Rust workspace.
The dependency-free [`examples/consume-negotiation.py`](../examples/consume-negotiation.py)
demonstrates the semantic selection rule independently of the reference implementation.

## Agent or producer

After completing a file, register only the artifacts intentionally exposed to the user:

```bash
reference=$(term-interop offer -- ./output/report.md)
```

Send the short reference, URI, or a normal Markdown file link according to the consuming UI. Do not
scan the workspace or register unrelated files.

## Shell and SSH

`term-interop preview @TOKEN` runs on the machine that owns the registry and file. Over SSH, install
the CLI on the remote side and invoke it in the interactive session. Pixel protocol bytes traverse
the existing PTY/SSH chain; no artifact file transfer or HTTP service is introduced.

The repository includes a black-box SSH harness that verifies both pixel and text consumers over
an authenticated remote PTY. It requires a target already authorized for non-interactive login and
never records the target identity:

```bash
TERM_INTEROP_SSH_TARGET=user@host \
TERM_INTEROP_SSH_BIN=/absolute/remote/path/to/term-interop \
TERM_INTEROP_ALACRITTY_BIN=/path/to/sixel-terminal \
    ./tests/e2e-preview-ssh.sh
```

## Embedding TUI

An interactive parent cannot safely keep reading the TTY while the preview reads close keys. The
parent must perform this handoff:

1. pause its terminal event source;
2. leave any parent-owned alternate screen and restore terminal modes;
3. invoke `term-interop preview PATH_OR_REFERENCE` attached to the same TTY;
4. wait for the exact child to exit;
5. restore parent modes, flush stale input, resume events, and redraw.

This is a lifecycle contract, not a Codex-specific API. Any TUI, multiplexer plugin, editor, or
agent client can implement the same handoff.

For clickable OSC 8 file links, bind an [intent callback v1](intent-callback-v1.md) endpoint and
encode it into the hyperlink. The system URL handler then returns the click to this exact consumer;
the consumer must still verify that the received path belongs to its current artifact set before
performing the handoff above.

Multiplexers are part of the hyperlink transport. For Zellij, enable `osc8_hyperlinks true`; when
Zellij mouse mode is enabled, `Shift+click` is the standard bypass that lets the outer terminal
consume the OSC 8 link. This affects only how the click reaches the desktop handler. The callback
endpoint and same-TTY handoff remain multiplexer-neutral.

## Failure behavior

An embedding consumer should keep these failures distinct:

- reference or identity validation failure;
- unsupported/binary artifact;
- unavailable terminal geometry;
- capability unavailable or inconclusive;
- protocol encoding failure;
- terminal I/O or restoration failure;
- child executable missing.

A character-art renderer can be offered as a separately named lower-fidelity consumer, but it must
not be reported as successful pixel rendering.
