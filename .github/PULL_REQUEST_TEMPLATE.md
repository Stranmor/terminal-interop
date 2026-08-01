## Practical outcome

Describe the user- or consumer-visible capability this change creates.

## Contract impact

- [ ] No contract change
- [ ] Backward-compatible contract extension
- [ ] Versioned incompatible change

## Evidence

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --locked -- -D warnings`
- [ ] `cargo test --all-targets --locked`
- [ ] Relevant real-consumer or black-box conformance path
- [ ] Fixtures and logs contain no private paths, credentials, or host data

## Compatibility boundary

List the exact protocol, transport, terminal, or consumer implementations this
change claims to support. Keep untested combinations explicitly unknown.
