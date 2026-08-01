//! Protocol-neutral contracts for terminal interoperability evidence.

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
}
