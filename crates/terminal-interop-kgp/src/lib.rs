//! Kitty Graphics Protocol adapter for terminal interoperability probes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use std::collections::BTreeMap;
use terminal_interop_core::{
    AdapterIdentity, AssertionOutcome, AssertionResult, Assessment, Availability, CapabilityId,
    Conformance, ProtocolId, WireEvent, WireEventRole,
};
use terminal_interop_da1::parse_response_at;

const ESC: u8 = 0x1b;
const ST: [u8; 2] = [ESC, b'\\'];

/// Adapter implementation version.
pub const ADAPTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Error returned when a KGP probe cannot be constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeBuildError {
    /// KGP reserves image identifier zero.
    ZeroCorrelationId,
}

/// Error returned when a PNG display transmission cannot be constructed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayBuildError {
    /// Empty payload is not a valid PNG transmission.
    EmptyPayload,
    /// Placement dimensions must occupy at least one cell.
    ZeroPlacement,
    /// Image number zero is reserved by the protocol.
    ZeroImageNumber,
}

impl std::fmt::Display for DisplayBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPayload => formatter.write_str("KGP PNG payload must not be empty"),
            Self::ZeroPlacement => formatter.write_str("KGP placement dimensions must be non-zero"),
            Self::ZeroImageNumber => formatter.write_str("KGP image number must be non-zero"),
        }
    }
}

impl std::error::Error for DisplayBuildError {}

/// Cell placement and lifecycle identity for one PNG display transmission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngDisplayOptions {
    /// Image number used to avoid collisions with unrelated image identifiers.
    pub image_number: u32,
    /// Placement width in terminal cells.
    pub columns: u16,
    /// Placement height in terminal cells.
    pub rows: u16,
}

impl std::fmt::Display for ProbeBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroCorrelationId => formatter.write_str("KGP correlation id must be non-zero"),
        }
    }
}

impl std::error::Error for ProbeBuildError {}

/// Parsed response and its profile assessment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedExchange {
    /// Recognized events in wire order.
    pub events: Vec<WireEvent>,
    /// Evidence-backed interpretation of the query profile.
    pub assessment: Assessment,
    /// Whether the correlated KGP reply was observed.
    pub correlated_reply_seen: bool,
    /// Whether the primary-device-attributes barrier reply was observed.
    pub barrier_seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObservedEvent {
    offset: usize,
    event: WireEvent,
}

/// Protocol identity for the first KGP interoperability profile.
#[must_use]
pub fn protocol_id() -> ProtocolId {
    ProtocolId {
        namespace: "org.kitty".to_string(),
        name: "terminal-graphics-protocol".to_string(),
        revision: "query-direct-rgb-v1".to_string(),
    }
}

/// Capability identity established by the query probe.
#[must_use]
pub fn capability_id() -> CapabilityId {
    CapabilityId { protocol: protocol_id(), name: "direct-rgb-query".to_string() }
}

/// Identity of this adapter implementation.
#[must_use]
pub fn adapter_identity() -> AdapterIdentity {
    AdapterIdentity {
        name: "terminal-interop-kgp".to_string(),
        version: ADAPTER_VERSION.to_string(),
    }
}

/// Build the official one-pixel KGP query followed by a DA1 ordering barrier.
///
/// # Errors
///
/// Returns [`ProbeBuildError::ZeroCorrelationId`] when `correlation_id` is zero.
pub fn build_query(correlation_id: u32) -> Result<Vec<u8>, ProbeBuildError> {
    if correlation_id == 0 {
        return Err(ProbeBuildError::ZeroCorrelationId);
    }

    let mut request =
        format!("\x1b_Gi={correlation_id},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\").into_bytes();
    request.extend_from_slice(b"\x1b[c");
    Ok(request)
}

/// Encode PNG bytes as a chunked direct KGP transmit-and-display sequence.
///
/// The result uses an image number, transient usage hint, fixed cell placement,
/// no cursor movement, and quiet mode. Chunks never exceed 4096 Base64 bytes.
///
/// # Errors
///
/// Returns [`DisplayBuildError`] for empty data or invalid placement identity.
pub fn encode_png_display(
    png: &[u8],
    options: PngDisplayOptions,
) -> Result<Vec<u8>, DisplayBuildError> {
    if png.is_empty() {
        return Err(DisplayBuildError::EmptyPayload);
    }
    if options.columns == 0 || options.rows == 0 {
        return Err(DisplayBuildError::ZeroPlacement);
    }
    if options.image_number == 0 {
        return Err(DisplayBuildError::ZeroImageNumber);
    }

    let encoded = BASE64_STANDARD.encode(png);
    let chunks = encoded.as_bytes().chunks(4096).collect::<Vec<_>>();
    let mut output =
        Vec::with_capacity(encoded.len().saturating_add(chunks.len().saturating_mul(96)));
    for (index, chunk) in chunks.iter().enumerate() {
        let more = u8::from(index.saturating_add(1) < chunks.len());
        if index == 0 {
            output.extend_from_slice(
                format!(
                    "\x1b_Ga=T,f=100,I={},c={},r={},C=1,N=1,q=2,m={more};",
                    options.image_number, options.columns, options.rows
                )
                .as_bytes(),
            );
        } else {
            output.extend_from_slice(format!("\x1b_Gm={more},q=2;").as_bytes());
        }
        output.extend_from_slice(chunk);
        output.extend_from_slice(b"\x1b\\");
    }
    Ok(output)
}

/// Build a hard-delete command for the newest image with one image number.
#[must_use]
pub fn delete_image_number(image_number: u32) -> Vec<u8> {
    format!("\x1b_Ga=d,d=N,I={image_number},q=2\x1b\\").into_bytes()
}

fn find_st(input: &[u8], from: usize) -> Option<usize> {
    input
        .get(from..)?
        .windows(ST.len())
        .position(|window| window == ST)
        .and_then(|relative| from.checked_add(relative))
}

fn parse_control_fields(control: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(control)
        .split(',')
        .filter_map(|field| {
            let (key, value) = field.split_once('=')?;
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn parse_apc_event(input: &[u8], offset: usize, sequence: u32) -> Option<(ObservedEvent, usize)> {
    let prefix_end = offset.checked_add(3)?;
    if input.get(offset..prefix_end)? != [ESC, b'_', b'G'] {
        return None;
    }
    let end = find_st(input, prefix_end)?;
    let body = input.get(prefix_end..end)?;
    let (control, status) =
        body.iter().position(|byte| *byte == b';').map_or((body, None), |separator| {
            let status_start = separator.saturating_add(1);
            let control = body.get(..separator).unwrap_or_default();
            let status = body.get(status_start..).unwrap_or_default();
            (control, Some(String::from_utf8_lossy(status).into_owned()))
        });
    let fields = parse_control_fields(control);
    let correlation_id = fields.get("i").and_then(|value| value.parse::<u32>().ok());
    let raw_end = end.checked_add(ST.len())?;
    let raw = input.get(offset..raw_end)?;
    let event = WireEvent {
        sequence,
        role: WireEventRole::CapabilityReply,
        protocol: Some(protocol_id()),
        correlation: correlation_id.map(|value| value.to_string()),
        status,
        fields,
        raw_base64: BASE64_STANDARD.encode(raw),
    };
    Some((ObservedEvent { offset, event }, raw_end))
}

fn parse_da1_event(input: &[u8], offset: usize, sequence: u32) -> Option<(ObservedEvent, usize)> {
    let response = parse_response_at(input, offset)?;
    let next = response.end_offset;
    let event = response.wire_event(sequence, WireEventRole::BarrierReply);
    Some((ObservedEvent { offset, event }, next))
}

fn assertion(id: &str, outcome: AssertionOutcome, detail: impl Into<String>) -> AssertionResult {
    AssertionResult { id: id.to_string(), outcome, detail: detail.into() }
}

fn assess_matching_reply(
    reply: &ObservedEvent,
    barrier: Option<&ObservedEvent>,
    correlation_id: u32,
) -> Assessment {
    let status_ok = reply.event.status.as_deref() == Some("OK");
    let ordered = barrier.is_none_or(|barrier| reply.offset < barrier.offset);
    let barrier_outcome =
        if barrier.is_some() { AssertionOutcome::Pass } else { AssertionOutcome::Unknown };
    let assertions = vec![
        assertion(
            "kgp.query.correlated-reply",
            AssertionOutcome::Pass,
            format!("received reply for image id {correlation_id}"),
        ),
        assertion(
            "kgp.query.reply-before-da1-barrier",
            if barrier.is_none() {
                AssertionOutcome::Unknown
            } else if ordered {
                AssertionOutcome::Pass
            } else {
                AssertionOutcome::Fail
            },
            if barrier.is_none() {
                "DA1 barrier was not observed"
            } else if ordered {
                "KGP reply preceded the DA1 barrier"
            } else {
                "KGP reply arrived after the DA1 barrier"
            },
        ),
        assertion(
            "kgp.query.direct-rgb-load",
            if status_ok { AssertionOutcome::Pass } else { AssertionOutcome::Fail },
            reply.event.status.as_deref().map_or_else(
                || "KGP reply did not contain a status".to_string(),
                |status| format!("KGP status was {status}"),
            ),
        ),
        assertion(
            "kgp.query.da1-barrier",
            barrier_outcome,
            if barrier.is_some() {
                "DA1 barrier reply was observed"
            } else {
                "DA1 barrier reply was not observed"
            },
        ),
    ];
    let conformance = if barrier.is_none() {
        Conformance::Inconclusive
    } else if status_ok && ordered {
        Conformance::Conformant
    } else {
        Conformance::Nonconformant
    };
    Assessment { availability: Availability::Available, conformance, assertions }
}

fn assess_unmatched_reply(
    unmatched: &ObservedEvent,
    barrier: Option<&ObservedEvent>,
    correlation_id: u32,
) -> Assessment {
    Assessment {
        availability: Availability::Available,
        conformance: Conformance::Nonconformant,
        assertions: vec![
            assertion(
                "kgp.query.correlated-reply",
                AssertionOutcome::Fail,
                format!(
                    "received KGP reply for {:?}, expected {correlation_id}",
                    unmatched.event.correlation
                ),
            ),
            assertion(
                "kgp.query.da1-barrier",
                if barrier.is_some() { AssertionOutcome::Pass } else { AssertionOutcome::Unknown },
                "barrier state retained independently of the malformed correlation",
            ),
        ],
    }
}

fn assess_unavailable() -> Assessment {
    Assessment {
        availability: Availability::Unavailable,
        conformance: Conformance::NotApplicable,
        assertions: vec![
            assertion(
                "kgp.query.correlated-reply",
                AssertionOutcome::NotApplicable,
                "DA1 barrier arrived without a KGP reply",
            ),
            assertion(
                "kgp.query.da1-barrier",
                AssertionOutcome::Pass,
                "DA1 barrier established the end of the ordered query window",
            ),
        ],
    }
}

fn assess_unknown() -> Assessment {
    Assessment {
        availability: Availability::Unknown,
        conformance: Conformance::Inconclusive,
        assertions: vec![
            assertion(
                "kgp.query.correlated-reply",
                AssertionOutcome::Unknown,
                "no correlated KGP reply was observed",
            ),
            assertion(
                "kgp.query.da1-barrier",
                AssertionOutcome::Unknown,
                "no DA1 barrier reply was observed",
            ),
        ],
    }
}

fn assess(events: &[ObservedEvent], correlation_id: u32) -> Assessment {
    let expected_correlation = correlation_id.to_string();
    let kgp_events = events
        .iter()
        .filter(|observed| observed.event.role == WireEventRole::CapabilityReply)
        .collect::<Vec<_>>();
    let matching = kgp_events.iter().find(|observed| {
        observed.event.correlation.as_deref() == Some(expected_correlation.as_str())
    });
    let barrier = events.iter().find(|observed| observed.event.role == WireEventRole::BarrierReply);

    if let Some(reply) = matching {
        return assess_matching_reply(reply, barrier, correlation_id);
    }
    if let Some(unmatched) = kgp_events.first() {
        return assess_unmatched_reply(unmatched, barrier, correlation_id);
    }
    if barrier.is_some() {
        return assess_unavailable();
    }
    assess_unknown()
}

/// Parse KGP and DA1 replies while preserving their order and exact bytes.
#[must_use]
pub fn parse_exchange(input: &[u8], correlation_id: u32) -> ParsedExchange {
    let mut observed = Vec::new();
    let mut offset = 0usize;
    let mut sequence = 0u32;

    while offset < input.len() {
        if let Some((event, next)) = parse_apc_event(input, offset, sequence) {
            observed.push(event);
            sequence = sequence.saturating_add(1);
            offset = next;
            continue;
        }
        if let Some((event, next)) = parse_da1_event(input, offset, sequence) {
            observed.push(event);
            sequence = sequence.saturating_add(1);
            offset = next;
            continue;
        }
        offset = offset.saturating_add(1);
    }

    let expected_correlation = correlation_id.to_string();
    let correlated_reply_seen = observed.iter().any(|event| {
        event.event.role == WireEventRole::CapabilityReply
            && event.event.correlation.as_deref() == Some(expected_correlation.as_str())
    });
    let barrier_seen = observed.iter().any(|event| event.event.role == WireEventRole::BarrierReply);
    let assessment = assess(&observed, correlation_id);
    let events = observed.into_iter().map(|event| event.event).collect();
    ParsedExchange { events, assessment, correlated_reply_seen, barrier_seen }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_official_query_and_barrier() {
        let request = build_query(31).expect("valid query");
        assert_eq!(request, b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c");
    }

    #[test]
    fn matching_ok_before_barrier_is_conformant() {
        let parsed = parse_exchange(b"\x1b_Gi=31;OK\x1b\\\x1b[?1;2c", 31);
        assert_eq!(parsed.assessment.availability, Availability::Available);
        assert_eq!(parsed.assessment.conformance, Conformance::Conformant);
        assert!(parsed.correlated_reply_seen);
        assert!(parsed.barrier_seen);
    }

    #[test]
    fn barrier_without_kgp_reply_is_unavailable_not_failed() {
        let parsed = parse_exchange(b"\x1b[?1;2c", 31);
        assert_eq!(parsed.assessment.availability, Availability::Unavailable);
        assert_eq!(parsed.assessment.conformance, Conformance::NotApplicable);
    }

    #[test]
    fn wrong_correlation_is_nonconformant() {
        let parsed = parse_exchange(b"\x1b_Gi=32;OK\x1b\\\x1b[?1;2c", 31);
        assert_eq!(parsed.assessment.availability, Availability::Available);
        assert_eq!(parsed.assessment.conformance, Conformance::Nonconformant);
    }

    #[test]
    fn timeout_without_events_preserves_unknown() {
        let parsed = parse_exchange(b"noise", 31);
        assert_eq!(parsed.assessment.availability, Availability::Unknown);
        assert_eq!(parsed.assessment.conformance, Conformance::Inconclusive);
    }

    #[test]
    fn delayed_reply_after_barrier_is_nonconformant() {
        let parsed = parse_exchange(b"\x1b[?1;2c\x1b_Gi=31;OK\x1b\\", 31);
        assert_eq!(parsed.assessment.availability, Availability::Available);
        assert_eq!(parsed.assessment.conformance, Conformance::Nonconformant);
    }

    #[test]
    fn png_display_uses_cell_placement_and_quiet_chunking() {
        let png = vec![0xabu8; 4_000];
        let encoded =
            encode_png_display(&png, PngDisplayOptions { image_number: 17, columns: 80, rows: 24 })
                .expect("valid display request");
        let rendered = String::from_utf8(encoded).expect("KGP framing is ASCII");
        assert!(rendered.starts_with("\u{1b}_Ga=T,f=100,I=17,c=80,r=24,C=1,N=1,q=2,m=1;"));
        assert!(rendered.contains("\u{1b}\\\u{1b}_Gm=0,q=2;"));
        assert!(rendered.ends_with("\u{1b}\\"));
    }

    #[test]
    fn png_display_rejects_invalid_identity_and_geometry() {
        let valid = PngDisplayOptions { image_number: 1, columns: 1, rows: 1 };
        assert_eq!(encode_png_display(&[], valid), Err(DisplayBuildError::EmptyPayload));
        assert_eq!(
            encode_png_display(b"png", PngDisplayOptions { image_number: 0, columns: 1, rows: 1 }),
            Err(DisplayBuildError::ZeroImageNumber)
        );
        assert_eq!(
            encode_png_display(b"png", PngDisplayOptions { image_number: 1, columns: 0, rows: 1 }),
            Err(DisplayBuildError::ZeroPlacement)
        );
    }
}
