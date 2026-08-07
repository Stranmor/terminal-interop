# Contract bundle

`v1/` is the implementation-neutral interoperability surface. A consumer does not need Rust or
the reference CLI to vendor these files.

- `schemas/` contains JSON Schema draft 2020-12 documents with stable `$id` values.
- `fixtures/valid/` contains documents every conforming validator must accept.
- `fixtures/invalid/` contains documents that violate a named structural or semantic invariant.
- `manifest.json` is the machine-readable inventory used by CI.

JSON Schema establishes document shape. Some rules depend on relations between fields—for example,
the selected capability must be the first eligible candidate. Those semantic rules are defined in
the Markdown specifications and exercised by the conformance vectors.

The reference validator consumes a file or standard input:

```bash
term-interop validate contracts/v1/fixtures/valid/negotiation-selected.json
cat contracts/v1/fixtures/valid/artifact-ref.json | term-interop validate --quiet
```

[`examples/consume-negotiation.py`](../examples/consume-negotiation.py) is a dependency-free,
non-Rust implementation of the negotiation decision rule. It exists to prove that the contract is
not coupled to Rust types or CLI internals.

Regenerate checked-in schemas after changing a contract type:

```bash
./scripts/update-contracts.sh
./tests/e2e-contract-bundle.sh
```

An incompatible change gets a new version directory and schema identity. Existing v1 files are
never silently rewritten to mean something else.
