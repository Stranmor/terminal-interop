# Preview profile v1

The preview profile is an interactive consumer of artifact, capability, geometry, protocol, and
transport contracts. It is deliberately not the authority for any of those layers.

## State machine

```text
requested path or reference
    -> identity-resolved regular file
        -> bounded bytes
            -> safe text | validated raster
                -> observed viewport
                    -> renderer selected from live evidence
                        -> alternate-screen presentation
                            -> explicit close
                                -> renderer cleanup and caller restoration
```

No renderer bytes are written before validation, geometry observation, and capability selection
succeed.

## Raster selection

`auto` probes KGP first, then Sixel. A renderer is eligible only when its receipt reports available,
conformant capability evidence and its transport is `ready` or `not_required`. `kgp` and `sixel`
make the same proof mandatory but disable fallback to the other protocol.

KGP receives canonical metadata-free PNG in bounded chunks, uses an image number for lifecycle
identity, and emits a matching delete operation during restoration. Sixel receives RGBA resized to
the observed pixel viewport with Lanczos3 filtering and a pure-Rust encoder.

On Unix, the consumer first reads the PTY's `TIOCGWINSZ` cell and pixel extents and accepts the
pixel dimensions only when both belong to the active TTY surface. If the PTY does not carry pixel
extent, terminal geometry queries pair pixel-area (`CSI 14 t`) and cell-area (`CSI 18 t`)
observations. Query-derived pixels are likewise accepted only when the reported cell dimensions
match the current TTY surface; otherwise the consumer falls back to conservative cell-derived
geometry. This prevents an outer terminal window measurement from being misapplied to a smaller
multiplexer pane.

## Text

Text is decoded lossily as UTF-8 after binary/NUL rejection. Escape and control bytes are replaced
before any artifact-derived bytes reach the terminal. The pager owns navigation and never executes
content.

## Lifecycle and keys

The consumer switches to the alternate screen, hides the cursor, and enters raw mode only while it
owns the preview. `q` and `Esc` close. Enter has no preview meaning. Explicit restoration reports I/O
failure; a drop guard performs the same cleanup best-effort during unwinding or early error.

An embedding TUI must pause its own input reader and restore terminal modes before invoking the
preview, then re-enable them and redraw after the child exits. See [integration.md](integration.md).
