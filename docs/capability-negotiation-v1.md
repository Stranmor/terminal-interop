# Capability negotiation v1

Schema identity: `urn:terminal-interop:capability-negotiation:v1`.

The negotiation contract selects one capability from an explicitly ordered list of live
`ProbeReceiptV1` documents. It does not probe by itself, infer terminal identity, or define product
policy beyond eligibility and caller-provided preference.

## Eligibility

A candidate is `eligible` only when its receipt establishes all of these facts:

1. `assessment.availability` is `available`;
2. `assessment.conformance` is `conformant`; and
3. `context.transport.readiness` is `ready` or `not_required`.

Every other combination is `ineligible`. The original receipt remains embedded, so a downstream
consumer can distinguish unavailable support, inconclusive evidence, nonconformance, and transport
failure.

Disposition and selection are derived fields. The Rust reference type keeps the negotiation body
private, validates schema identity and canonical preference order, recomputes every disposition,
and rejects deserialization unless selection names the first eligible receipt with the same
capability and adapter identity. JSON Schema validates shape; implementations in other languages
must enforce these cross-field semantics separately.

## Preference and selection

Candidates retain their zero-based input order as `preference`. The first eligible candidate is
selected. The core never sorts by terminal name, protocol family, adapter version, environment, or
an implicit quality score.

When no candidate is eligible, selection is:

```json
{"state":"no_eligible_candidate"}
```

This is not equivalent to "the terminal cannot display pixels." It means only that none of the
supplied candidates established the required evidence on the observed chain.

## Reference CLI profile

```bash
term-interop negotiate pixel --pretty
```

The current `pixel` profile supplies KGP first and Sixel second. Other consumers may choose a
different order or supply additional adapters while preserving the schema semantics.

Generate the JSON Schema with:

```bash
term-interop schema negotiation --pretty
```

Validate a complete document with the reference implementation:

```bash
term-interop validate ./negotiation.json
```

The versioned schema, valid vector, forged-selection vector, and dependency-free Python consumer
live under [`contracts/v1`](../contracts/README.md). A conforming implementation must accept the
valid vector and reject the forged selection even if a shape-only JSON Schema validator accepts
both documents.
