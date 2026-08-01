//! Sixel capability and encoding adapter.

use icy_sixel::{EncodeOptions, SixelError, sixel_encode};
use terminal_interop_core::{
    AdapterIdentity, AssertionOutcome, AssertionResult, Assessment, Availability, CapabilityId,
    Conformance, ProtocolId, WireEvent, WireEventRole,
};
use terminal_interop_da1::{build_query as build_da1_query, parse_responses};

/// DA1 extension identifier assigned to Sixel graphics.
pub const SIXEL_DA1_EXTENSION: u16 = 4;

/// Adapter implementation version.
pub const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed Sixel capability exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedExchange {
    /// Recognized DA1 response events.
    pub events: Vec<WireEvent>,
    /// Evidence-backed capability assessment.
    pub assessment: Assessment,
    /// Whether a valid DA1 response was observed.
    pub response_seen: bool,
    /// Whether a response advertised extension identifier 4.
    pub sixel_advertised: bool,
}

/// Sixel protocol identity used by this profile.
#[must_use]
pub fn protocol_id() -> ProtocolId {
    ProtocolId {
        namespace: "org.dec".to_string(),
        name: "sixel-raster-graphics".to_string(),
        revision: "da1-extension-4-v1".to_string(),
    }
}

/// Capability identity established by a DA1 response.
#[must_use]
pub fn capability_id() -> CapabilityId {
    CapabilityId { protocol: protocol_id(), name: "raster-image-display".to_string() }
}

/// Identity of this adapter implementation.
#[must_use]
pub fn adapter_identity() -> AdapterIdentity {
    AdapterIdentity {
        name: "terminal-interop-sixel".to_string(),
        version: ADAPTER_VERSION.to_string(),
    }
}

/// Build the standard DA1 query used to discover Sixel advertisement.
#[must_use]
pub fn build_query() -> Vec<u8> {
    build_da1_query()
}

fn assertion(id: &str, outcome: AssertionOutcome, detail: &str) -> AssertionResult {
    AssertionResult { id: id.to_string(), outcome, detail: detail.to_string() }
}

/// Parse a DA1 response and assess whether Sixel is advertised.
#[must_use]
pub fn parse_exchange(input: &[u8]) -> ParsedExchange {
    let responses = parse_responses(input);
    let response_seen = !responses.is_empty();
    let sixel_advertised =
        responses.iter().any(|response| response.advertises(SIXEL_DA1_EXTENSION));
    let events = responses
        .iter()
        .enumerate()
        .map(|(sequence, response)| {
            response.wire_event(
                u32::try_from(sequence).unwrap_or(u32::MAX),
                WireEventRole::CapabilityReply,
            )
        })
        .collect();

    let assessment = if sixel_advertised {
        Assessment {
            availability: Availability::Available,
            conformance: Conformance::Conformant,
            assertions: vec![
                assertion(
                    "sixel.da1.response",
                    AssertionOutcome::Pass,
                    "a valid primary device attributes response was observed",
                ),
                assertion(
                    "sixel.da1.extension-4",
                    AssertionOutcome::Pass,
                    "the response advertised Sixel extension identifier 4",
                ),
            ],
        }
    } else if response_seen {
        Assessment {
            availability: Availability::Unavailable,
            conformance: Conformance::NotApplicable,
            assertions: vec![
                assertion(
                    "sixel.da1.response",
                    AssertionOutcome::Pass,
                    "a valid primary device attributes response was observed",
                ),
                assertion(
                    "sixel.da1.extension-4",
                    AssertionOutcome::NotApplicable,
                    "the response did not advertise Sixel extension identifier 4",
                ),
            ],
        }
    } else {
        Assessment {
            availability: Availability::Unknown,
            conformance: Conformance::Inconclusive,
            assertions: vec![
                assertion(
                    "sixel.da1.response",
                    AssertionOutcome::Unknown,
                    "no complete primary device attributes response was observed",
                ),
                assertion(
                    "sixel.da1.extension-4",
                    AssertionOutcome::Unknown,
                    "Sixel advertisement cannot be evaluated without a DA1 response",
                ),
            ],
        }
    };

    ParsedExchange { events, assessment, response_seen, sixel_advertised }
}

/// Encode one bounded RGBA image as a complete Sixel DCS sequence.
///
/// # Errors
///
/// Returns [`SixelError`] when dimensions, buffer length, or quantization are invalid.
pub fn encode_rgba(
    rgba: &[u8],
    width: usize,
    height: usize,
    options: &EncodeOptions,
) -> Result<Vec<u8>, SixelError> {
    sixel_encode(rgba, width, height, options).map(String::into_bytes)
}

/// Encode one bounded RGBA image with the interoperable default profile.
///
/// # Errors
///
/// Returns [`SixelError`] when dimensions, buffer length, or quantization are invalid.
pub fn encode_rgba_default(
    rgba: &[u8],
    width: usize,
    height: usize,
) -> Result<Vec<u8>, SixelError> {
    encode_rgba(rgba, width, height, &EncodeOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_four_is_available_and_conformant() {
        let parsed = parse_exchange(b"\x1b[?62;4;52c");
        assert!(parsed.response_seen);
        assert!(parsed.sixel_advertised);
        assert_eq!(parsed.assessment.availability, Availability::Available);
        assert_eq!(parsed.assessment.conformance, Conformance::Conformant);
    }

    #[test]
    fn response_without_extension_is_unavailable() {
        let parsed = parse_exchange(b"\x1b[?62;52c");
        assert!(parsed.response_seen);
        assert!(!parsed.sixel_advertised);
        assert_eq!(parsed.assessment.availability, Availability::Unavailable);
        assert_eq!(parsed.assessment.conformance, Conformance::NotApplicable);
    }

    #[test]
    fn missing_response_preserves_unknown() {
        let parsed = parse_exchange(b"");
        assert!(!parsed.response_seen);
        assert_eq!(parsed.assessment.availability, Availability::Unknown);
        assert_eq!(parsed.assessment.conformance, Conformance::Inconclusive);
    }

    #[test]
    fn encodes_a_complete_sixel_sequence() {
        let rgba = [255, 0, 0, 255];
        let encoded = encode_rgba(&rgba, 1, 1, &EncodeOptions::default()).expect("encode");
        assert!(encoded.starts_with(b"\x1bP"));
        assert!(encoded.ends_with(b"\x1b\\"));
    }
}
