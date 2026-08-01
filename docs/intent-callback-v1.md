# Intent callback v1

Schema identities:

- `urn:terminal-interop:open-intent:v1`
- `urn:terminal-interop:intent-ready:v1`
- `urn:terminal-interop:intent-receipt:v1`

An OSC 8 hyperlink is normally opened by the terminal emulator through a desktop URL handler. The
handler is outside the originating TUI and does not inherit its PTY, pane, input lifecycle, or
application state. Starting another terminal is therefore not a same-terminal handoff.

Intent callback v1 lets the originating consumer bind an unguessable local endpoint and encode
that endpoint into the hyperlink. The desktop handler delivers the typed intent over a private
Unix socket; the original consumer then decides whether the target is still authorized and owns
the normal same-TTY preview lifecycle.

```text
TUI consumer
  -> bind endpoint under $XDG_RUNTIME_DIR/terminal-interop/intent-v1
  -> render terminal-interop-intent://v1/open/ENDPOINT/PATH_BASE64URL
      -> terminal OSC 8 click
          -> desktop scheme handler
              -> term-interop intent dispatch URI
                  -> exact bound consumer
                      -> validate current artifact relation
                          -> same-TTY preview
```

## URI

`terminal-interop-intent://v1/open/ENDPOINT/PATH_BASE64URL`

- `ENDPOINT` is 128 random bits represented as 32 lowercase hexadecimal characters.
- `PATH_BASE64URL` is the exact absolute path bytes encoded with unpadded URL-safe Base64.
- query and fragment components are invalid;
- empty, relative, NUL-containing, or larger-than-16-KiB paths are invalid;
- the URI is a live local callback capability, not a durable artifact identity.

## Local transport

The reference Unix transport binds `ENDPOINT.sock` below a private `0700` runtime directory. The
socket is `0600`. Requests and receipts are length-prefixed, bounded JSON documents. A dispatcher
receives `forwarded` only after the listener has written the validated intent to its consumer
handoff. This receipt proves local forwarding, not that the TUI rendered the artifact; visible
consumer proof remains a separate boundary.

The endpoint disappears with the consumer. A stale hyperlink fails explicitly instead of choosing
another terminal, pane, session, or process by heuristic.

## Multiplexer transport

The callback protocol does not require a multiplexer API, but the multiplexer must preserve OSC 8
metadata on its outer terminal stream. Zellij consumers enable this with `osc8_hyperlinks true`.
When `mouse_mode true`, use `Shift+click` to bypass Zellij mouse handling; direct terminal sessions
use an ordinary click. A missing OSC 8 transport is distinct from a missing callback capability.

## Embedding

```bash
endpoint=$(term-interop intent endpoint)
term-interop intent listen "$endpoint"
term-interop intent uri "$endpoint" /absolute/path/to/image.png
term-interop intent dispatch 'terminal-interop-intent://v1/open/...'
```

`intent listen` emits one `IntentReadyV1` JSON line, followed by one validated `OpenIntentV1` JSON
line per delivered callback. An embedding TUI should treat those lines as sensor output, validate
the target against its own current authorization model, and only then actuate its preview.
