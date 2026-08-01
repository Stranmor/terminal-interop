# Changelog

All notable changes to this project will be documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project intends to use [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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

### Changed

- Raster placement now prefers pixel dimensions carried by the exact PTY winsize and validates
  their cell extent before use, avoiding oversized and clipped previews inside framed multiplexer
  panes.
- The Sixel framebuffer E2E accepts an explicit external fixture and records its dimensions and
  SHA-256, so photographic quality can be verified without replacing the canonical test image.
