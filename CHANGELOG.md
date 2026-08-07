# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project intends to use [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Capability negotiation v1: ordered live receipts, explicit eligibility, deterministic selection,
  and a first-class `no_eligible_candidate` state.
- Fail-closed semantic deserialization for negotiation receipts, including preference,
  disposition, selected capability, and adapter consistency.
- A versioned language-neutral contract bundle with stable JSON Schema `$id` values, positive and
  adversarial vectors, a bounded auto-detecting validator, and an independent standard-library
  Python negotiation consumer.
- `term-interop negotiate pixel` plus generated negotiation and artifact-reference JSON Schemas.
- Publish-ready metadata and versioned path dependencies for the independently reusable Rust
  crates.
- Architecture, ecosystem-positioning, and process/JSON integration documentation.
- Pinned GitHub Actions, dependency/license/advisory policy, weekly update automation, and a
  tag-gated Linux x86_64 release archive with SHA-256 sidecar.
- Versioned terminal capability probe receipt and JSON Schema.
- Kitty Graphics Protocol direct-RGB query adapter.
- Sixel capability and pure-Rust raster encoding adapter.
- tmux DCS passthrough transport adapter with evidence-based readiness.
- Bounded text/raster artifact validation and geometry-aware placement.
- Identity-bound short artifact references and typed local URIs.
- Same-TTY interactive preview with explicit close and screen restoration.
- Isolated real-terminal and framebuffer E2E coverage for Kitty, tmux, Zellij, and an exact
  Alacritty graphics candidate.
- Authenticated OpenSSH PTY E2E for remote Sixel images and sanitized Unicode text, including
  inert Enter, explicit close, restoration, and one-window assertions.
- Content-addressed local installer and `zv` convenience command.
- Intent callback v1: a terminal-neutral, private Unix-socket return channel that lets OSC 8
  hyperlinks reopen artifacts in their exact originating TUI instead of launching or guessing a
  terminal session.
- A generated desktop scheme handler and a black-box callback E2E covering ready, forwarded, and
  stale-endpoint states.
- Zellij integration guidance for preserving OSC 8 metadata and using the standard Shift-click
  mouse bypass without coupling the callback protocol to Zellij.
- A physical OSC 8 Shift-click E2E through Alacritty, Zellij, the desktop scheme handler, and the
  bound private callback listener.
- Installer-isolation E2E for content-addressed custom-prefix installs.

### Changed

- Raster placement now prefers pixel dimensions carried by the exact PTY winsize and validates
  their cell extent before use, avoiding oversized and clipped previews inside framed multiplexer
  panes.
- The Sixel framebuffer E2E accepts an explicit external fixture and records its dimensions and
  SHA-256, so photographic quality can be verified without replacing the canonical test image.
- Custom `PREFIX` installs keep desktop metadata inside that prefix and no longer replace the live
  user's URI-handler registration during packaging or release-smoke tests.
