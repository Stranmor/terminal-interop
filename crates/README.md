# Terminal Interop crate family

This crate is one replaceable component of Terminal Interop: a capability-negotiated handoff layer
for terminal applications and local artifacts.

The workspace separates protocol-neutral evidence, artifact identity, callback intents, terminal
protocols, transports, geometry, and the reference CLI so applications can depend on only the
contracts they own.

- [Repository and quick start](https://github.com/Stranmor/terminal-interop)
- [Architecture](https://github.com/Stranmor/terminal-interop/blob/main/docs/architecture.md)
- [Consumer integration](https://github.com/Stranmor/terminal-interop/blob/main/docs/integration.md)
- [Capability negotiation v1](https://github.com/Stranmor/terminal-interop/blob/main/docs/capability-negotiation-v1.md)
- [Compatibility evidence](https://github.com/Stranmor/terminal-interop/blob/main/docs/compatibility.md)
- [Security policy](https://github.com/Stranmor/terminal-interop/blob/main/SECURITY.md)

All crates use the same pre-1.0 workspace version and are dual-licensed under MIT or Apache-2.0.
