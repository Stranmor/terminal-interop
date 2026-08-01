# Probe receipt v1

Schema identity: `urn:terminal-interop:probe-receipt:v1`.

A receipt records one live capability exchange without collapsing observation, transport, and
interpretation into a terminal-name boolean.

## Layers

- `capability`: protocol namespace, name, and active profile revision.
- `adapter`: the encoder/parser implementation that produced the interpretation.
- `correlation`: protocol-level request identity when the profile supports one.
- `context.transport`: direct or transformed wire path plus readiness exchanges.
- `context.environment_hints`: bounded observations; sensitive variables are presence-only.
- `context.topology`: declared or `unknown`, never inferred from adjacency.
- `exchange`: exact logical request, transformed wire request, response bytes, parsed wire events,
  elapsed time, and stop reason.
- `assessment`: independent availability, conformance, and assertion outcomes.

All wire byte strings are standard Base64. Unknown and not-applicable states are first-class; they
must not be rewritten as `false`, `available`, or `conformant` by a downstream consumer.

## Active protocol profiles

- KGP: a correlated 1x1 direct-RGB query followed by a primary-device-attributes barrier.
  Availability requires the correlated KGP reply; the barrier prevents silence from being treated
  as support.
- Sixel: primary-device-attributes extension `4`. This proves the consumer advertised the Sixel
  profile and that the response parsed conformantly. It is not, by itself, framebuffer proof for
  every downstream layer.
- tmux passthrough: a transport-owned readiness exchange is recorded separately from the inner
  protocol exchange.

Generate the JSON Schema with:

```bash
term-interop schema receipt --pretty
```
