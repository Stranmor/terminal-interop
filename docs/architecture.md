# Architecture

Terminal Interop is a family of small contracts and adapters. Its purpose is to let independent
producers, terminal applications, transports, and renderers compose without importing one
product's environment heuristics or lifecycle assumptions.

## Invariants

1. **A reference identifies one observed file identity.** Replacing the path target never silently
   rebinds an existing token.
2. **Capability comes from the live consumed chain.** Terminal names and environment variables are
   diagnostic hints, not sufficient evidence.
3. **Unknown remains a legal result.** Timeout, incomplete topology, unavailable support, and
   nonconformant replies are distinct states.
4. **Selection policy is explicit input order.** The core never invents a renderer or fallback the
   consumer did not supply.
5. **Untrusted bytes stop at parsers.** Typed observations cross into selection and actuation;
   artifact bytes never become terminal control input.
6. **The interactive owner is singular.** A parent TUI pauses its input source before handing the
   same TTY to a preview and redraws only after the exact child exits.
7. **Compatibility claims bind to an exact chain.** A protocol reply is weaker evidence than a
   visible framebuffer plus restoration receipt.

## Contract graph

```text
ArtifactRefV1
    -> identity-resolved regular file
        -> safe text | bounded canonical raster

ProbeReceiptV1[]
    -> CapabilityNegotiationV1
        -> selected capability | no eligible candidate

selected capability + artifact + viewport
    -> protocol adapter
        -> transport adapter
            -> current TTY

external OSC 8 click
    -> OpenIntentV1
        -> exact private endpoint
            -> parent-owned authorization and TTY handoff
```

The arrows are legal transitions, not an inheritance hierarchy. A consumer may replace any adapter
while retaining the contracts on either side.

## Evidence and policy remain separate

`ProbeReceiptV1` records what happened on the wire. `CapabilityNegotiationV1` applies one narrow
eligibility rule to an ordered list of those receipts. A product can then add its own policy—for
example, preferring a lower-bandwidth renderer over SSH—without modifying the observation.

The v1 eligibility rule requires all three conditions:

- capability availability is `available`;
- profile conformance is `conformant`; and
- transport readiness is `ready` or `not_required`.

Anything else is ineligible for actuation, but its receipt is retained for diagnosis and future
policy.

## Adapter boundary

A protocol adapter owns:

- construction of a bounded logical request;
- parsing and correlation of protocol replies;
- stable conformance assertions;
- conversion of validated raster data into protocol bytes; and
- renderer-owned cleanup operations.

A transport adapter owns only transformation and readiness of the path carrying those bytes. For
example, tmux DCS passthrough wraps a KGP request but does not interpret KGP.

Environment detection belongs to neither adapter. It may select which transport to *attempt*, but
only live evidence can establish the result.

## Adding a protocol

1. Define a stable `ProtocolId`, `CapabilityId`, and profile revision.
2. Implement request construction and a bounded parser in a separate crate.
3. Emit exact logical request, wire request, response, parsed events, and assertion outcomes in a
   `ProbeReceiptV1`.
4. Keep transport transformations outside the protocol crate.
5. Add parser tests for malformed, partial, reordered, and oversized input.
6. Add a black-box test against the exact consumer chain being claimed.
7. Leave untested chains unknown in the compatibility table.

## Why the CLI is not the standard

`term-interop` is one reference composition. The schemas, state transitions, and conformance
evidence are the compatibility surface. Other implementations may expose a library API, a daemon,
a TUI plugin, or a different CLI as long as they preserve those semantics.
