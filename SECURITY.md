# Security policy

Terminal escape sequences cross a trust boundary: untrusted files and text
must never become executable control input merely because they are previewed.

## Supported versions

Security fixes currently target the latest commit on `main`. A stable release
support policy will be published with the first tagged release.

## Reporting a vulnerability

Use GitHub private vulnerability reporting after the repository is published.
Until that channel exists, do not post exploit details in a public issue.

Useful reports identify the exact command, terminal or multiplexer chain,
input type, observed effect, and whether the issue crosses one of these
boundaries:

- terminal control-sequence injection;
- reading a file other than the explicitly selected artifact;
- following an unsafe symlink or special file;
- unbounded memory, output, image, or decompression consumption;
- executing artifact content or a MIME-selected helper;
- leaking local paths, environment values, or file contents into receipts.

The v1 artifact profile rejects encoded inputs larger than 32 MiB before registry hashing or
preview decoding. Raster dimensions and decoded allocation have independent bounds.

## Design boundary

Protocol parsers accept untrusted bytes and produce typed observations only.
Actuators consume validated requests with explicit resource limits. Environment
hints never establish capability support or authorize an action.

`term-interop validate` proves document structure and protocol-neutral cross-field invariants. A
probe receipt is evidence, not a signed attestation: validation does not prove who ran the adapter
or that its protocol-specific assessment honestly follows from the retained wire bytes. A product
must trust the sensor that produced a receipt, rerun an eligible local adapter, or add its own
authenticated provenance before using receipts across a trust boundary.
