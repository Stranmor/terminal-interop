# Consumer integration

Terminal Interop exposes processes and data contracts rather than requiring an application to link
all Rust crates.

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
