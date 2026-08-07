//! Protocol-neutral contracts for terminal interoperability evidence.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

/// Maximum encoded artifact size accepted by the version-one preview profile: 32 MiB.
///
/// Registration and consumption share this bound so a short reference cannot authorize work
/// that the eventual preview consumer must reject after hashing an unbounded file.
pub const MAX_ARTIFACT_INPUT_BYTES_V1: usize = 32 * 1024 * 1024;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable schema identity for [`ProbeReceiptV1`].
pub const PROBE_RECEIPT_SCHEMA_V1: &str = "urn:terminal-interop:probe-receipt:v1";
/// Stable schema identity for [`CapabilityNegotiationV1`].
pub const CAPABILITY_NEGOTIATION_SCHEMA_V1: &str = "urn:terminal-interop:capability-negotiation:v1";

/// A protocol or standards family independent of any implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtocolId {
    /// Reverse-domain or standards-owned namespace.
    pub namespace: String,
    /// Human-readable protocol name inside the namespace.
    pub name: String,
    /// Profile or protocol revision used by this exchange.
    pub revision: String,
}

/// One capability with semantics defined by a protocol profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityId {
    /// Protocol family that owns the capability semantics.
    pub protocol: ProtocolId,
    /// Stable capability name inside the protocol profile.
    pub name: String,
}

/// Identity of the adapter that produced the wire exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdapterIdentity {
    /// Adapter implementation name.
    pub name: String,
    /// Adapter implementation version.
    pub version: String,
}

/// Protocol-scoped correlation identity for one request-response relation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrelationId {
    /// Stable namespace defining how the value is interpreted.
    pub namespace: String,
    /// Exact protocol value retained without imposing a numeric representation.
    pub value: String,
}

/// How an environment hint was retained without promoting it to capability evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HintObservation {
    /// The variable was present; its value was intentionally not retained.
    Present,
    /// A bounded non-sensitive value was retained verbatim.
    Value(String),
}

/// A bounded environment observation that never establishes topology or support.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentHint {
    /// Environment variable name.
    pub name: String,
    /// Observed presence or allowlisted value.
    pub observation: HintObservation,
}

/// Whether the real transport topology is known from direct evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TopologyObservation {
    /// No direct topology sensor was available.
    Unknown,
    /// The caller explicitly supplied an ordered path.
    Declared {
        /// Ordered hops from client to terminal consumer.
        hops: Vec<PathHop>,
    },
}

/// One explicitly declared transport or consumer hop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathHop {
    /// Generic role such as `multiplexer`, `transport`, or `terminal`.
    pub role: String,
    /// Implementation name when known.
    pub implementation: String,
    /// Implementation version when known.
    pub version: Option<String>,
}

/// Context in which a live probe was executed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeContext {
    /// TTY endpoint used for both request and response.
    pub tty_endpoint: String,
    /// Evidence about preparation of the active transport path.
    pub transport: TransportEvidence,
    /// Bounded observations useful for diagnosis but not capability claims.
    pub environment_hints: Vec<EnvironmentHint>,
    /// Directly evidenced or explicitly unknown topology.
    pub topology: TopologyObservation,
}

/// Whether a transport path was ready for an application protocol exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportReadiness {
    /// This adapter requires no preparation exchange.
    NotRequired,
    /// A transport-owned readiness exchange completed.
    Ready,
    /// Preparation did not establish readiness before its deadline.
    Unknown,
}

/// Transport identity and any preparation exchanges executed before the probe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TransportEvidence {
    /// Adapter that transformed logical requests into wire requests.
    pub adapter: AdapterIdentity,
    /// Evidence-backed readiness of this transport path.
    pub readiness: TransportReadiness,
    /// Ordered transport-owned exchanges retained for independent inspection.
    pub preparation_exchanges: Vec<WireExchange>,
}

/// Semantic role of a parsed wire event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WireEventRole {
    /// Reply to the capability probe.
    CapabilityReply,
    /// Reply to the ordering barrier sent after the probe.
    BarrierReply,
    /// Bytes retained as evidence but not classified by the active adapter.
    Unclassified,
}

/// One parsed event with its exact wire representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireEvent {
    /// Zero-based event order in the received byte stream.
    pub sequence: u32,
    /// Adapter-independent event role.
    pub role: WireEventRole,
    /// Protocol family that parsed this event, if any.
    pub protocol: Option<ProtocolId>,
    /// Correlation value recovered from the protocol response.
    pub correlation: Option<String>,
    /// Protocol-defined status token such as `OK`.
    pub status: Option<String>,
    /// Structured protocol fields retained without changing the core schema.
    pub fields: BTreeMap<String, String>,
    /// Exact event bytes encoded with standard Base64.
    pub raw_base64: String,
}

/// Why the live exchange stopped collecting response bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Both the capability reply and its ordering barrier were observed.
    CapabilityAndBarrierObserved,
    /// The ordering barrier arrived without a capability reply.
    BarrierObserved,
    /// The configured deadline elapsed.
    Timeout,
    /// The bounded response buffer was exhausted.
    ResourceLimit,
    /// The TTY returned end-of-input.
    EndOfInput,
}

/// Exact bytes and parsed events from one request-response exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireExchange {
    /// Logical request before any transport-specific transformation.
    pub logical_request_base64: String,
    /// Exact request written to the TTY after transport transformation.
    pub wire_request_base64: String,
    /// Exact bytes read before the exchange stopped, encoded with standard Base64.
    pub response_base64: String,
    /// Parsed events in wire order.
    pub events: Vec<WireEvent>,
    /// Monotonic elapsed time rounded to milliseconds.
    pub elapsed_ms: u64,
    /// Terminal condition for response collection.
    pub stop_reason: StopReason,
}

/// Evidence-backed availability of the requested capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// A correlated protocol reply established implementation support.
    Available,
    /// The barrier established that no protocol reply was produced.
    Unavailable,
    /// The exchange could not establish either state.
    Unknown,
}

/// Conformance of the observed exchange to the active profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Conformance {
    /// All applicable assertions passed.
    Conformant,
    /// At least one applicable assertion failed.
    Nonconformant,
    /// The implementation did not expose the capability, so profile conformance was not tested.
    NotApplicable,
    /// Evidence was insufficient to evaluate the profile.
    Inconclusive,
}

/// Result of one stable conformance assertion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssertionOutcome {
    /// The evidence satisfies the assertion.
    Pass,
    /// The evidence contradicts the assertion.
    Fail,
    /// The evidence cannot decide the assertion.
    Unknown,
    /// The assertion does not apply to this capability state.
    NotApplicable,
}

/// One independently consumable assertion result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssertionResult {
    /// Stable profile-scoped assertion identifier.
    pub id: String,
    /// Typed result rather than a boolean that would erase uncertainty.
    pub outcome: AssertionOutcome,
    /// Concise evidence-bound explanation.
    pub detail: String,
}

/// Interpretation kept separate from the wire exchange that supports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    /// Whether the capability was observed as available.
    pub availability: Availability,
    /// Whether the exchange conformed to the active profile.
    pub conformance: Conformance,
    /// Stable, independently inspectable assertion results.
    pub assertions: Vec<AssertionResult>,
}

/// Version-one portable receipt for a live terminal capability probe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeReceiptV1 {
    /// Stable schema identifier.
    pub schema: String,
    /// Wall-clock observation time as milliseconds since the Unix epoch.
    pub observed_at_unix_ms: u64,
    /// Capability whose semantics govern the assessment.
    pub capability: CapabilityId,
    /// Adapter implementation that emitted and parsed the exchange.
    pub adapter: AdapterIdentity,
    /// Protocol-level correlation identity when this profile defines one.
    pub correlation: Option<CorrelationId>,
    /// Execution context with unknowns preserved.
    pub context: ProbeContext,
    /// Exact transport evidence.
    pub exchange: WireExchange,
    /// Interpretation derived from the exchange.
    pub assessment: Assessment,
}

/// Structural failure in a serialized probe receipt independent of protocol policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeReceiptValidationError {
    /// The document does not use the v1 schema identity.
    UnsupportedSchema(String),
    /// One wire-evidence field is not canonical standard Base64.
    InvalidBase64 {
        /// Stable field path inside the receipt.
        field: String,
    },
    /// Parsed wire events are not numbered in exact stream order.
    NoncanonicalEventSequence {
        /// Stable exchange path inside the receipt.
        exchange: String,
        /// Expected zero-based sequence number.
        expected: u32,
        /// Serialized sequence number.
        actual: u32,
    },
}

impl std::fmt::Display for ProbeReceiptValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported probe receipt schema: {schema:?}")
            },
            Self::InvalidBase64 { field } => {
                write!(formatter, "probe receipt field {field} is not standard Base64")
            },
            Self::NoncanonicalEventSequence { exchange, expected, actual } => write!(
                formatter,
                "noncanonical event sequence in {exchange}: expected {expected}, received {actual}"
            ),
        }
    }
}

impl std::error::Error for ProbeReceiptValidationError {}

fn validate_base64(field: String, value: &str) -> Result<(), ProbeReceiptValidationError> {
    BASE64_STANDARD
        .decode(value)
        .map(|_| ())
        .map_err(|_| ProbeReceiptValidationError::InvalidBase64 { field })
}

fn validate_exchange(
    path: &str,
    exchange: &WireExchange,
) -> Result<(), ProbeReceiptValidationError> {
    validate_base64(format!("{path}.logical_request_base64"), &exchange.logical_request_base64)?;
    validate_base64(format!("{path}.wire_request_base64"), &exchange.wire_request_base64)?;
    validate_base64(format!("{path}.response_base64"), &exchange.response_base64)?;
    for (index, event) in exchange.events.iter().enumerate() {
        let expected = u32::try_from(index).unwrap_or(u32::MAX);
        if event.sequence != expected {
            return Err(ProbeReceiptValidationError::NoncanonicalEventSequence {
                exchange: path.to_owned(),
                expected,
                actual: event.sequence,
            });
        }
        validate_base64(format!("{path}.events[{index}].raw_base64"), &event.raw_base64)?;
    }
    Ok(())
}

impl ProbeReceiptV1 {
    /// Validate protocol-neutral structural invariants retained by the v1 receipt.
    ///
    /// Protocol-specific parsers remain responsible for proving that parsed events and
    /// assessments follow from the exact wire bytes. This method validates only invariants that
    /// every implementation can check without knowing the active protocol profile.
    ///
    /// # Errors
    ///
    /// Returns an error for a foreign schema identity, malformed Base64 evidence, or reordered
    /// event sequence numbers.
    pub fn validate(&self) -> Result<(), ProbeReceiptValidationError> {
        if self.schema != PROBE_RECEIPT_SCHEMA_V1 {
            return Err(ProbeReceiptValidationError::UnsupportedSchema(self.schema.clone()));
        }
        for (index, exchange) in self.context.transport.preparation_exchanges.iter().enumerate() {
            validate_exchange(
                &format!("context.transport.preparation_exchanges[{index}]"),
                exchange,
            )?;
        }
        validate_exchange("exchange", &self.exchange)
    }
}

/// Whether one observed candidate is safe to actuate under the v1 negotiation profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDisposition {
    /// Availability, profile conformance, and transport readiness are all established.
    Eligible,
    /// At least one required claim is unavailable, negative, or unknown.
    Ineligible,
}

/// One capability receipt in caller-defined preference order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NegotiationCandidateV1 {
    /// Zero-based preference rank supplied by the negotiating consumer.
    pub preference: u64,
    /// Derived eligibility; the complete evidence remains in `receipt`.
    pub disposition: CandidateDisposition,
    /// Exact live receipt from the candidate adapter.
    pub receipt: ProbeReceiptV1,
}

/// Result of evaluating all supplied candidates without erasing negative or unknown evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NegotiationSelectionV1 {
    /// The first eligible candidate in explicit preference order.
    Selected {
        /// Preference rank of the selected candidate.
        preference: u64,
        /// Selected protocol capability.
        capability: CapabilityId,
        /// Adapter that produced the selected evidence.
        adapter: AdapterIdentity,
    },
    /// No supplied candidate had sufficient positive evidence.
    NoEligibleCandidate,
}

/// Portable receipt for deterministic capability selection from ordered live evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityNegotiationV1 {
    /// Stable schema identifier.
    schema: String,
    /// Candidates in exact caller-defined preference order.
    candidates: Vec<NegotiationCandidateV1>,
    /// Selected candidate or an explicit evidence-backed absence.
    selection: NegotiationSelectionV1,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityNegotiationWireV1 {
    schema: String,
    candidates: Vec<NegotiationCandidateV1>,
    selection: NegotiationSelectionV1,
}

/// Semantic failure in a serialized capability negotiation receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NegotiationValidationError {
    /// The document does not use the v1 schema identity.
    UnsupportedSchema(String),
    /// A candidate preference does not match its exact vector position.
    NoncanonicalPreference {
        /// Expected zero-based preference.
        expected: u64,
        /// Serialized preference.
        actual: u64,
    },
    /// Serialized disposition contradicts the embedded probe receipt.
    DispositionMismatch {
        /// Candidate preference whose disposition is invalid.
        preference: u64,
    },
    /// Selection does not point to the first eligible candidate.
    SelectionMismatch,
    /// Selection duplicates a capability or adapter other than the selected candidate's identity.
    SelectedIdentityMismatch {
        /// Selected preference whose duplicated identity is invalid.
        preference: u64,
    },
    /// An embedded candidate receipt violates protocol-neutral v1 structure.
    InvalidReceipt {
        /// Candidate preference whose receipt is invalid.
        preference: u64,
        /// Exact structural failure in the embedded receipt.
        source: ProbeReceiptValidationError,
    },
}

impl std::fmt::Display for NegotiationValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported capability negotiation schema: {schema:?}")
            },
            Self::NoncanonicalPreference { expected, actual } => write!(
                formatter,
                "noncanonical capability preference: expected {expected}, received {actual}"
            ),
            Self::DispositionMismatch { preference } => write!(
                formatter,
                "capability disposition contradicts receipt at preference {preference}"
            ),
            Self::SelectionMismatch => {
                formatter.write_str("selection is not the first eligible capability")
            },
            Self::SelectedIdentityMismatch { preference } => write!(
                formatter,
                "selected capability identity does not match preference {preference}"
            ),
            Self::InvalidReceipt { preference, source } => {
                write!(formatter, "invalid probe receipt at preference {preference}: {source}")
            },
        }
    }
}

impl std::error::Error for NegotiationValidationError {}

const fn preference_from_index(index: usize) -> u64 {
    index as u64
}

impl<'de> Deserialize<'de> for CapabilityNegotiationV1 {
    fn deserialize<Deserializer>(deserializer: Deserializer) -> Result<Self, Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        let wire = CapabilityNegotiationWireV1::deserialize(deserializer)?;
        let value =
            Self { schema: wire.schema, candidates: wire.candidates, selection: wire.selection };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl CapabilityNegotiationV1 {
    /// Stable schema identity retained in this receipt.
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Ordered candidate evidence.
    #[must_use]
    pub fn candidates(&self) -> &[NegotiationCandidateV1] {
        &self.candidates
    }

    /// Deterministic selection derived from the ordered candidates.
    #[must_use]
    pub const fn selection(&self) -> &NegotiationSelectionV1 {
        &self.selection
    }

    /// Validate every derived field against the embedded evidence.
    ///
    /// # Errors
    ///
    /// Returns a semantic error when schema identity, preference order, disposition, or selection
    /// contradicts the v1 contract.
    pub fn validate(&self) -> Result<(), NegotiationValidationError> {
        if self.schema != CAPABILITY_NEGOTIATION_SCHEMA_V1 {
            return Err(NegotiationValidationError::UnsupportedSchema(self.schema.clone()));
        }

        let mut first_eligible = None;
        for (index, candidate) in self.candidates.iter().enumerate() {
            let expected = preference_from_index(index);
            if candidate.preference != expected {
                return Err(NegotiationValidationError::NoncanonicalPreference {
                    expected,
                    actual: candidate.preference,
                });
            }

            candidate.receipt.validate().map_err(|source| {
                NegotiationValidationError::InvalidReceipt {
                    preference: candidate.preference,
                    source,
                }
            })?;

            let expected_disposition = if receipt_is_eligible(&candidate.receipt) {
                CandidateDisposition::Eligible
            } else {
                CandidateDisposition::Ineligible
            };
            if candidate.disposition != expected_disposition {
                return Err(NegotiationValidationError::DispositionMismatch {
                    preference: candidate.preference,
                });
            }
            if first_eligible.is_none()
                && matches!(candidate.disposition, CandidateDisposition::Eligible)
            {
                first_eligible = Some(candidate);
            }
        }

        match (&self.selection, first_eligible) {
            (NegotiationSelectionV1::NoEligibleCandidate, None) => Ok(()),
            (
                NegotiationSelectionV1::Selected { preference, capability, adapter },
                Some(candidate),
            ) if *preference == candidate.preference => {
                if capability != &candidate.receipt.capability
                    || adapter != &candidate.receipt.adapter
                {
                    return Err(NegotiationValidationError::SelectedIdentityMismatch {
                        preference: *preference,
                    });
                }
                Ok(())
            },
            _ => Err(NegotiationValidationError::SelectionMismatch),
        }
    }
}

/// Return whether a probe receipt establishes every condition required for actuation.
#[must_use]
pub const fn receipt_is_eligible(receipt: &ProbeReceiptV1) -> bool {
    matches!(receipt.assessment.availability, Availability::Available)
        && matches!(receipt.assessment.conformance, Conformance::Conformant)
        && matches!(
            receipt.context.transport.readiness,
            TransportReadiness::NotRequired | TransportReadiness::Ready
        )
}

/// Select the first eligible capability while preserving every supplied receipt.
///
/// Input order is the complete preference policy. The function does not infer terminal identity,
/// reorder candidates, or introduce a fallback that was not supplied by the caller.
#[must_use]
pub fn negotiate_capabilities_v1(receipts: Vec<ProbeReceiptV1>) -> CapabilityNegotiationV1 {
    let mut selection = NegotiationSelectionV1::NoEligibleCandidate;
    let candidates = receipts
        .into_iter()
        .enumerate()
        .map(|(index, receipt)| {
            let preference = preference_from_index(index);
            let disposition = if receipt_is_eligible(&receipt) {
                if matches!(selection, NegotiationSelectionV1::NoEligibleCandidate) {
                    selection = NegotiationSelectionV1::Selected {
                        preference,
                        capability: receipt.capability.clone(),
                        adapter: receipt.adapter.clone(),
                    };
                }
                CandidateDisposition::Eligible
            } else {
                CandidateDisposition::Ineligible
            };
            NegotiationCandidateV1 { preference, disposition, receipt }
        })
        .collect();

    CapabilityNegotiationV1 {
        schema: CAPABILITY_NEGOTIATION_SCHEMA_V1.to_owned(),
        candidates,
        selection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_schema_rejects_implicit_topology() {
        let schema = schemars::schema_for!(ProbeReceiptV1);
        let rendered = serde_json::to_string(&schema).expect("schema should serialize");
        assert!(rendered.contains("TopologyObservation"));
        assert!(rendered.contains("unknown"));
        assert!(rendered.contains("declared"));
    }

    fn receipt(
        name: &str,
        availability: Availability,
        conformance: Conformance,
        readiness: TransportReadiness,
    ) -> ProbeReceiptV1 {
        let protocol = ProtocolId {
            namespace: "org.example".to_owned(),
            name: name.to_owned(),
            revision: "v1".to_owned(),
        };
        let adapter = AdapterIdentity { name: format!("{name}-adapter"), version: "1".to_owned() };
        ProbeReceiptV1 {
            schema: PROBE_RECEIPT_SCHEMA_V1.to_owned(),
            observed_at_unix_ms: 0,
            capability: CapabilityId { protocol, name: "pixel-preview".to_owned() },
            adapter: adapter.clone(),
            correlation: None,
            context: ProbeContext {
                tty_endpoint: "/dev/tty".to_owned(),
                transport: TransportEvidence {
                    adapter,
                    readiness,
                    preparation_exchanges: Vec::new(),
                },
                environment_hints: Vec::new(),
                topology: TopologyObservation::Unknown,
            },
            exchange: WireExchange {
                logical_request_base64: String::new(),
                wire_request_base64: String::new(),
                response_base64: String::new(),
                events: Vec::new(),
                elapsed_ms: 0,
                stop_reason: StopReason::Timeout,
            },
            assessment: Assessment { availability, conformance, assertions: Vec::new() },
        }
    }

    #[test]
    fn negotiation_selects_first_eligible_candidate_without_erasing_evidence() {
        let first = receipt(
            "first",
            Availability::Unknown,
            Conformance::Inconclusive,
            TransportReadiness::Unknown,
        );
        let second = receipt(
            "second",
            Availability::Available,
            Conformance::Conformant,
            TransportReadiness::Ready,
        );
        let third = receipt(
            "third",
            Availability::Available,
            Conformance::Conformant,
            TransportReadiness::NotRequired,
        );

        let result = negotiate_capabilities_v1(vec![first, second, third]);

        assert_eq!(result.candidates.len(), 3);
        let dispositions: Vec<_> =
            result.candidates.iter().map(|candidate| candidate.disposition).collect();
        assert_eq!(
            dispositions,
            vec![
                CandidateDisposition::Ineligible,
                CandidateDisposition::Eligible,
                CandidateDisposition::Eligible,
            ]
        );
        assert!(matches!(result.selection, NegotiationSelectionV1::Selected { preference: 1, .. }));
    }

    #[test]
    fn negotiation_schema_preserves_explicit_no_candidate_state() {
        let schema = schemars::schema_for!(CapabilityNegotiationV1);
        let rendered = serde_json::to_string(&schema).expect("schema should serialize");
        assert!(rendered.contains("no_eligible_candidate"));
        assert!(rendered.contains("ineligible"));
    }

    #[test]
    fn negotiation_deserialization_rejects_forged_derived_state() {
        let first = receipt(
            "first",
            Availability::Unknown,
            Conformance::Inconclusive,
            TransportReadiness::Unknown,
        );
        let second = receipt(
            "second",
            Availability::Available,
            Conformance::Conformant,
            TransportReadiness::Ready,
        );
        let negotiation = negotiate_capabilities_v1(vec![first, second]);
        let mut forged = serde_json::to_value(&negotiation).expect("negotiation should serialize");
        let preference = forged.pointer_mut("/selection/preference");
        assert!(preference.is_some());
        if let Some(preference) = preference {
            *preference = serde_json::json!(0);
        }

        let error = serde_json::from_value::<CapabilityNegotiationV1>(forged)
            .expect_err("forged selection must fail closed");

        assert!(error.to_string().contains("first eligible"));
    }
}
