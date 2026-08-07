# Integration examples

Terminal Interop can be integrated as processes plus JSON. Linking Rust crates is optional.

The runnable [`consume-negotiation.py`](consume-negotiation.py) example uses only the Python
standard library. It independently validates the semantic relationship between ordered receipts,
derived dispositions, and the selected capability:

```bash
python3 examples/consume-negotiation.py \
    contracts/v1/fixtures/valid/negotiation-selected.json
```

This complements JSON Schema: schema validators establish document shape, while the example shows
the cross-field rule a non-Rust implementation must preserve.

## Producer

Register only a completed artifact intentionally exposed to the user:

```bash
artifact_ref=$(term-interop offer -- ./output/report.md)
printf 'Artifact: %s\n' "$artifact_ref"
```

Use `--format json` when the next consumer is software rather than a human.

## Capability gate

An application can obtain one complete selection receipt before it commits to pixel rendering:

```bash
receipt=$(mktemp)
term-interop negotiate pixel --pretty --output "$receipt"

if jq -e '.selection.state == "selected"' "$receipt" >/dev/null; then
    jq '.selection' "$receipt"
else
    jq '.candidates[] | {preference, disposition, assessment: .receipt.assessment}' "$receipt"
fi
```

The application may use a different preference order or add its own policy. It should never turn
an unknown receipt into a positive capability claim.

## Embedding TUI

A parent TUI must not read terminal input concurrently with the preview child:

```text
pause events
  -> leave parent alternate screen and restore modes
      -> spawn term-interop preview on the same TTY
          -> wait for that exact child
              -> restore modes, flush stale input, redraw, resume events
```

For external OSC 8 clicks, bind an intent endpoint and validate each received artifact against the
parent's current artifact set before executing that handoff. See
[docs/integration.md](../docs/integration.md) for the complete lifecycle.
