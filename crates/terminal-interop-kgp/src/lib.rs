//! Kitty Graphics Protocol adapter for terminal interoperability probes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use std::collections::BTreeMap;
use terminal_interop_core::{
    AdapterIdentity, AssertionOutcome, AssertionResult, Assessment, Availability, CapabilityId,
    Conformance, ProtocolId, WireEvent, WireEventRole,
};

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
        correlation_id,
        status,
        fields,
        raw_base64: BASE64_STANDARD.encode(raw),
    };
    Some((ObservedEvent { offset, event }, raw_end))
}

fn csi_final(input: &[u8], from: usize) -> Option<usize> {
    input
        .get(from..)?
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
        .and_then(|index| from.checked_add(index))
}

fn parse_da1_event(input: &[u8], offset: usize, sequence: u32) -> Option<(ObservedEvent, usize)> {
    let prefix_end = offset.checked_add(2)?;
    if input.get(offset..prefix_end)? != [ESC, b'['] {
        return None;
    }
    let end = csi_final(input, prefix_end)?;
    if input.get(end) != Some(&b'c') {
        return None;
    }
    let parameters = String::from_utf8_lossy(input.get(prefix_end..end)?).into_owned();
    let mut fields = BTreeMap::new();
    fields.insert("parameters".to_string(), parameters);
    let raw_end = end.checked_add(1)?;
    let raw = input.get(offset..raw_end)?;
    let event = WireEvent {
        sequence,
        role: WireEventRole::BarrierReply,
        protocol: None,
        correlation_id: None,
        status: None,
        fields,
        raw_base64: BASE64_STANDARD.encode(raw),
    };
    Some((ObservedEvent { offset, event }, raw_end))
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
                    unmatched.event.correlation_id
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
    let kgp_events = events
        .iter()
        .filter(|observed| observed.event.role == WireEventRole::CapabilityReply)
        .collect::<Vec<_>>();
    let matching =
        kgp_events.iter().find(|observed| observed.event.correlation_id == Some(correlation_id));
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

    let correlated_reply_seen = observed.iter().any(|event| {
        event.event.role == WireEventRole::CapabilityReply
            && event.event.correlation_id == Some(correlation_id)
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
}
