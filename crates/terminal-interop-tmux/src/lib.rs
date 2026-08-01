//! Transport adapter for tmux DCS passthrough.

use terminal_interop_core::AdapterIdentity;

const ESC: u8 = 0x1b;

/// Adapter implementation version.
pub const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Identity of the tmux transport adapter.
#[must_use]
pub fn adapter_identity() -> AdapterIdentity {
    AdapterIdentity {
        name: "tmux-dcs-passthrough".to_string(),
        version: ADAPTER_VERSION.to_string(),
    }
}

/// Wrap arbitrary terminal-protocol bytes in tmux DCS passthrough framing.
///
/// Every ESC byte in the payload is doubled as required by tmux passthrough.
#[must_use]
pub fn wrap_passthrough(payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len().saturating_add(9));
    output.extend_from_slice(b"\x1bPtmux;");
    for byte in payload {
        if *byte == ESC {
            output.push(ESC);
        }
        output.push(*byte);
    }
    output.extend_from_slice(b"\x1b\\");
    output
}

/// Build a tmux-passthrough primary-device-attributes readiness query.
#[must_use]
pub fn build_readiness_query() -> Vec<u8> {
    wrap_passthrough(b"\x1b[c")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_inner_terminal_sequences() {
        let input = b"\x1b_Gi=31;AAAA\x1b\\\x1b[c";
        let expected = b"\x1bPtmux;\x1b\x1b_Gi=31;AAAA\x1b\x1b\\\x1b\x1b[c\x1b\\";
        assert_eq!(wrap_passthrough(input), expected);
    }

    #[test]
    fn empty_payload_is_valid_passthrough_frame() {
        assert_eq!(wrap_passthrough(&[]), b"\x1bPtmux;\x1b\\");
    }

    #[test]
    fn readiness_query_is_a_wrapped_da1_request() {
        assert_eq!(build_readiness_query(), b"\x1bPtmux;\x1b\x1b[c\x1b\\");
    }
}
