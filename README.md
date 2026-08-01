# Terminal Interop

Executable, protocol-neutral evidence for terminal capability negotiation.

The first adapter probes the Kitty Graphics Protocol (KGP) through the real
TTY chain. The receipt keeps wire evidence separate from interpretation, so a
terminal, multiplexer, SSH transport, test runner, or application can consume
the same result without inheriting product-specific assumptions.

## Contract

`terminal-interop-core` owns the versioned JSON receipt and assessment model.
Protocol crates only own encoding and parsing. Transport crates only transform
protocol bytes for one path. The CLI owns live TTY I/O.

```text
consumer
   -> terminal-interop-core receipt
      -> protocol adapter (KGP first)
         -> transport adapter (direct TTY or tmux passthrough)
            -> live TTY chain
```

The system does not infer topology or support from `$TERM`. Environment values
are recorded only as bounded hints; support claims require wire evidence.

## Build and test

```bash
cargo test --all-targets
cargo run -p terminal-interop-cli -- schema receipt
cargo run -p terminal-interop-cli -- probe kgp --pretty
./tests/e2e-hidden-kitty.sh
```

For non-interactive runs, write the receipt directly to a file:

```bash
term-interop probe kgp --output /tmp/kgp-receipt.json --pretty
term-interop probe kgp --transport tmux-passthrough --pretty
```

## Scope of the first vertical slice

- versioned, machine-readable probe receipts;
- exact protocol request, transformed wire request, and response bytes;
- transport readiness evidence kept separate from capability evidence;
- independent availability and conformance assessments;
- explicit `unknown` when the barrier or protocol reply is absent;
- KGP direct-RGB query plus primary-device-attributes barrier;
- real PTY execution suitable for direct terminals and nested multiplexers.

Rendering, image lifecycle, security limits, and placement conformance are
separate profiles that can build on the same core contract.

The E2E suite starts isolated hidden Kitty, tmux, and Zellij consumers. It
retains a receipt for every path plus a compact summary under `/var/tmp`; it
never attaches to an existing multiplexer session.
