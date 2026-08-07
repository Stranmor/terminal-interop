//! Live terminal interoperability probe CLI.

mod preview;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use clap::{Args, Parser, Subcommand, ValueEnum};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::termios::{FlushArg, SetArg, Termios, cfmakeraw, tcflush, tcgetattr, tcsetattr};
use schemars::{JsonSchema, schema_for};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use terminal_interop_core::{
    AdapterIdentity, CAPABILITY_NEGOTIATION_SCHEMA_V1, CapabilityNegotiationV1, CorrelationId,
    EnvironmentHint, HintObservation, PROBE_RECEIPT_SCHEMA_V1, ProbeContext, ProbeReceiptV1,
    StopReason, TopologyObservation, TransportEvidence, TransportReadiness, WireEvent,
    WireEventRole, WireExchange, negotiate_capabilities_v1,
};
use terminal_interop_da1::parse_responses as parse_da1_responses;
use terminal_interop_intent::{
    EndpointId, INTENT_READY_SCHEMA_V1, INTENT_RECEIPT_SCHEMA_V1, IntentDeliveryState,
    IntentListener, IntentReadyV1, OPEN_INTENT_SCHEMA_V1, OpenIntentV1,
    dispatch as dispatch_intent, runtime_root as intent_runtime_root,
};
use terminal_interop_kgp::{
    adapter_identity as kgp_adapter_identity, build_query as build_kgp_query,
    capability_id as kgp_capability_id, parse_exchange as parse_kgp_exchange,
};
use terminal_interop_sixel::{
    adapter_identity as sixel_adapter_identity, build_query as build_sixel_query,
    capability_id as sixel_capability_id, parse_exchange as parse_sixel_exchange,
};
use terminal_interop_tmux::{
    adapter_identity as tmux_adapter_identity, build_readiness_query as build_tmux_readiness_query,
    wrap_passthrough as wrap_tmux_passthrough,
};
use thiserror::Error;

use preview::PreviewArgs;

const DEFAULT_TTY: &str = "/dev/tty";
const DEFAULT_TIMEOUT_MS: u64 = 750;
const DEFAULT_TRANSPORT_TIMEOUT_MS: u64 = 1_500;
const TRANSPORT_ATTEMPT_TIMEOUT_MS: u64 = 100;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_CONTRACT_DOCUMENT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "term-interop",
    version,
    about = "Artifact handoff and live capability negotiation for terminal applications"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Register one artifact and emit a short path-independent reference.
    Offer(OfferArgs),
    /// Preview one text or raster artifact in the current terminal context.
    Preview(PreviewArgs),
    /// Create or deliver a local callback intent to an exact interactive consumer.
    Intent {
        #[command(subcommand)]
        command: IntentCommand,
    },
    /// Execute a live protocol probe through one TTY chain.
    Probe {
        #[command(subcommand)]
        protocol: ProbeProtocol,
    },
    /// Select an eligible capability from ordered live probe evidence.
    Negotiate {
        #[command(subcommand)]
        profile: NegotiationProfile,
    },
    /// Emit a machine-readable interoperability schema.
    Schema {
        /// Contract to emit.
        #[arg(value_enum, default_value_t = SchemaDocument::Receipt)]
        document: SchemaDocument,
        /// Pretty-print JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Validate one versioned contract document from a file or standard input.
    Validate(ValidateArgs),
}

#[derive(Debug, Args)]
struct ValidateArgs {
    /// JSON document path, or `-` to read standard input.
    #[arg(default_value = "-")]
    input: PathBuf,
    /// Emit no success message; the exit status remains authoritative.
    #[arg(long)]
    quiet: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SchemaDocument {
    Receipt,
    ArtifactRef,
    Negotiation,
    OpenIntent,
    IntentReceipt,
    IntentReady,
}

#[derive(Debug, Subcommand)]
enum IntentCommand {
    /// Generate one unguessable endpoint identity.
    Endpoint,
    /// Build an exact-path open URI for a bound endpoint.
    Uri {
        /// Endpoint identity printed by `intent endpoint` or supplied by the embedding consumer.
        endpoint: String,
        /// Absolute artifact path carried as data in the intent.
        path: PathBuf,
    },
    /// Bind a private local endpoint and forward validated intents as JSON Lines on stdout.
    Listen {
        /// Exact endpoint identity owned by this consumer.
        endpoint: String,
        /// Exit after forwarding one intent.
        #[arg(long)]
        once: bool,
    },
    /// Deliver one callback URI to its bound local consumer.
    Dispatch {
        /// Versioned `terminal-interop-intent://` URI.
        uri: String,
        /// Read and write deadline in milliseconds.
        #[arg(long, default_value_t = 1_500)]
        timeout_ms: u64,
        /// Suppress the successful forwarding receipt.
        #[arg(long)]
        quiet: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OfferFormat {
    Short,
    Uri,
    Json,
}

#[derive(Debug, Args)]
struct OfferArgs {
    /// Completed regular file to expose through a short reference.
    path: PathBuf,
    /// Output representation for humans, hyperlinks, or adapters.
    #[arg(long, value_enum, default_value_t = OfferFormat::Short)]
    format: OfferFormat,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TransportKind {
    /// Write protocol bytes directly to the TTY.
    Direct,
    /// Wrap protocol bytes in tmux DCS passthrough framing.
    TmuxPassthrough,
}

#[derive(Debug, Subcommand)]
enum ProbeProtocol {
    /// Query Kitty Graphics Protocol direct-RGB support with a DA1 barrier.
    Kgp(KgpProbeArgs),
    /// Query Sixel support through the DA1 extension advertisement.
    Sixel(SixelProbeArgs),
}

#[derive(Debug, Subcommand)]
enum NegotiationProfile {
    /// Probe KGP then Sixel and emit the complete pixel-preview selection receipt.
    Pixel(PixelNegotiationArgs),
}

#[derive(Debug, Args)]
struct PixelNegotiationArgs {
    /// Response deadline for each protocol probe in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS)]
    timeout_ms: u64,
    /// Maximum response bytes retained from each probe.
    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES)]
    max_response_bytes: usize,
    /// TTY endpoint used for the bidirectional exchanges.
    #[arg(long, default_value = DEFAULT_TTY)]
    tty: PathBuf,
    /// Transport policy for the pixel profile.
    #[arg(long, value_enum, default_value_t = NegotiationTransport::Auto)]
    transport: NegotiationTransport,
    /// Deadline for a transport-owned readiness exchange in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TRANSPORT_TIMEOUT_MS)]
    transport_timeout_ms: u64,
    /// Persist the negotiation receipt atomically instead of writing it to stdout.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Pretty-print the negotiation receipt JSON.
    #[arg(long)]
    pretty: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum NegotiationTransport {
    /// Use tmux passthrough only when a tmux environment marker is present.
    Auto,
    /// Write protocol bytes directly to the TTY.
    Direct,
    /// Wrap protocol bytes in tmux DCS passthrough framing.
    TmuxPassthrough,
}

#[derive(Debug, Args)]
struct KgpProbeArgs {
    /// Positive KGP image id used to correlate the reply.
    #[arg(long, default_value_t = 31)]
    correlation_id: u32,
    #[command(flatten)]
    common: TtyProbeArgs,
}

#[derive(Debug, Args)]
struct SixelProbeArgs {
    #[command(flatten)]
    common: TtyProbeArgs,
}

#[derive(Clone, Debug, Args)]
struct TtyProbeArgs {
    /// Response deadline in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS)]
    timeout_ms: u64,
    /// Maximum response bytes retained from the TTY.
    #[arg(long, default_value_t = DEFAULT_MAX_RESPONSE_BYTES)]
    max_response_bytes: usize,
    /// TTY endpoint used for the bidirectional exchange.
    #[arg(long, default_value = DEFAULT_TTY)]
    tty: PathBuf,
    /// Transformation applied to protocol bytes before writing to the TTY.
    #[arg(long, value_enum, default_value_t = TransportKind::Direct)]
    transport: TransportKind,
    /// Deadline for a transport-owned readiness exchange in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TRANSPORT_TIMEOUT_MS)]
    transport_timeout_ms: u64,
    /// Persist the receipt atomically instead of writing it to stdout.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Pretty-print the receipt JSON.
    #[arg(long)]
    pretty: bool,
}

#[derive(Debug, Error)]
enum AppError {
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal operation failed: {0}")]
    Terminal(#[from] Errno),
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("cannot build KGP probe: {0}")]
    ProbeBuild(#[from] terminal_interop_kgp::ProbeBuildError),
    #[error("cannot build KGP display request: {0}")]
    DisplayBuild(#[from] terminal_interop_kgp::DisplayBuildError),
    #[error("artifact preparation failed: {0}")]
    Artifact(#[from] terminal_interop_artifact::ArtifactError),
    #[error("artifact reference failed: {0}")]
    Registry(#[from] terminal_interop_ref::RegistryError),
    #[error("intent callback failed: {0}")]
    Intent(#[from] terminal_interop_intent::IntentError),
    #[error("intent consumer rejected the request: {0}")]
    IntentRejected(String),
    #[error("Sixel encoding failed: {0}")]
    SixelEncoding(String),
    #[error("artifact is not a regular file: {0}")]
    NotRegularFile(PathBuf),
    #[error("artifact has {actual} bytes; preview limit is {limit}")]
    PreviewInputLimit { actual: u64, limit: usize },
    #[error("terminal cell geometry is unavailable for {0}")]
    TerminalGeometryUnavailable(PathBuf),
    #[error("no evidence-backed pixel renderer is available (KGP: {kgp}; Sixel: {sixel})")]
    NoPixelRenderer { kgp: String, sixel: String },
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("system clock is before the Unix epoch")]
    ClockBeforeEpoch,
    #[error("timeout must be greater than zero")]
    ZeroTimeout,
    #[error("timeout is outside the monotonic clock range")]
    TimeoutOutOfRange,
    #[error("intent timeout must be greater than zero")]
    ZeroIntentTimeout,
    #[error("transport timeout must be greater than zero")]
    ZeroTransportTimeout,
    #[error("max response bytes must be greater than zero")]
    ZeroResponseLimit,
    #[error("contract document exceeds the {MAX_CONTRACT_DOCUMENT_BYTES}-byte validation limit")]
    ContractLimit,
    #[error("contract document has no string schema identity")]
    ContractSchemaMissing,
    #[error("unsupported contract schema: {0:?}")]
    UnsupportedContractSchema(String),
    #[error("contract validation failed: {0}")]
    ContractInvalid(String),
}

struct RawModeGuard {
    tty: File,
    original: Termios,
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = tcsetattr(&self.tty, SetArg::TCSANOW, &self.original);
    }
}

#[derive(Debug)]
struct LiveExchange {
    response: Vec<u8>,
    elapsed: Duration,
    stop_reason: StopReason,
}

fn environment_hints() -> Vec<EnvironmentHint> {
    const VALUE_HINTS: &[&str] = &["TERM", "TERM_PROGRAM", "COLORTERM"];
    const PRESENCE_HINTS: &[&str] =
        &["ZELLIJ", "ZELLIJ_SESSION_NAME", "TMUX", "SSH_CONNECTION", "SSH_TTY"];

    let mut hints = Vec::new();
    for name in VALUE_HINTS {
        if let Some(value) = env::var_os(name) {
            hints.push(EnvironmentHint {
                name: (*name).to_string(),
                observation: HintObservation::Value(value.to_string_lossy().into_owned()),
            });
        }
    }
    for name in PRESENCE_HINTS {
        if env::var_os(name).is_some() {
            hints.push(EnvironmentHint {
                name: (*name).to_string(),
                observation: HintObservation::Present,
            });
        }
    }
    hints.sort_by(|left, right| left.name.cmp(&right.name));
    hints
}

fn remaining_timeout(deadline: Instant) -> PollTimeout {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let millis = remaining.as_millis().clamp(1, u128::from(u16::MAX));
    PollTimeout::from(u16::try_from(millis).unwrap_or(u16::MAX))
}

fn execute_tty_exchange<F>(
    tty_path: &Path,
    request: &[u8],
    timeout: Duration,
    max_response_bytes: usize,
    stop_when: F,
) -> Result<LiveExchange, AppError>
where
    F: Fn(&[u8]) -> Option<StopReason>,
{
    if timeout.is_zero() {
        return Err(AppError::ZeroTimeout);
    }
    if max_response_bytes == 0 {
        return Err(AppError::ZeroResponseLimit);
    }

    let mut tty = OpenOptions::new().read(true).write(true).open(tty_path)?;
    let original = tcgetattr(&tty)?;
    tcflush(&tty, FlushArg::TCIFLUSH)?;
    let mut raw = original.clone();
    cfmakeraw(&mut raw);
    tcsetattr(&tty, SetArg::TCSANOW, &raw)?;
    let _raw_guard = RawModeGuard { tty: tty.try_clone()?, original };

    tty.write_all(request)?;
    tty.flush()?;

    let started = Instant::now();
    let deadline = started.checked_add(timeout).ok_or(AppError::TimeoutOutOfRange)?;
    let mut response = Vec::new();
    let stop_reason = loop {
        if let Some(reason) = stop_when(&response) {
            break reason;
        }
        if response.len() >= max_response_bytes {
            break StopReason::ResourceLimit;
        }
        if Instant::now() >= deadline {
            break StopReason::Timeout;
        }

        let mut poll_fds = [PollFd::new(tty.as_fd(), PollFlags::POLLIN)];
        match poll(&mut poll_fds, remaining_timeout(deadline)) {
            Ok(0) => break StopReason::Timeout,
            Ok(_) => {},
            Err(Errno::EINTR) => continue,
            Err(error) => return Err(AppError::Terminal(error)),
        }

        let available = max_response_bytes.saturating_sub(response.len());
        let mut buffer = vec![0u8; available.min(4096)];
        let read = tty.read(&mut buffer)?;
        if read == 0 {
            break StopReason::EndOfInput;
        }
        let bytes = buffer.get(..read).ok_or_else(|| {
            AppError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "TTY read exceeded the supplied buffer",
            ))
        })?;
        response.extend_from_slice(bytes);
    };

    Ok(LiveExchange { response, elapsed: started.elapsed(), stop_reason })
}

fn kgp_stop_reason(response: &[u8], correlation_id: u32) -> Option<StopReason> {
    let parsed = parse_kgp_exchange(response, correlation_id);
    if !parsed.barrier_seen {
        return None;
    }
    Some(if parsed.correlated_reply_seen {
        StopReason::CapabilityAndBarrierObserved
    } else {
        StopReason::BarrierObserved
    })
}

fn da1_stop_reason(response: &[u8]) -> Option<StopReason> {
    (!parse_da1_responses(response).is_empty()).then_some(StopReason::BarrierObserved)
}

fn unix_time_ms() -> Result<u64, AppError> {
    let elapsed =
        SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| AppError::ClockBeforeEpoch)?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn direct_transport_identity() -> AdapterIdentity {
    AdapterIdentity {
        name: "direct-tty".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn wire_exchange(
    logical_request: &[u8],
    wire_request: &[u8],
    live: LiveExchange,
    events: Vec<WireEvent>,
) -> WireExchange {
    WireExchange {
        logical_request_base64: BASE64_STANDARD.encode(logical_request),
        wire_request_base64: BASE64_STANDARD.encode(wire_request),
        response_base64: BASE64_STANDARD.encode(&live.response),
        events,
        elapsed_ms: u64::try_from(live.elapsed.as_millis()).unwrap_or(u64::MAX),
        stop_reason: live.stop_reason,
    }
}

fn prepare_transport(args: &TtyProbeArgs) -> Result<TransportEvidence, AppError> {
    if matches!(args.transport, TransportKind::Direct) {
        return Ok(TransportEvidence {
            adapter: direct_transport_identity(),
            readiness: TransportReadiness::NotRequired,
            preparation_exchanges: Vec::new(),
        });
    }
    if args.transport_timeout_ms == 0 {
        return Err(AppError::ZeroTransportTimeout);
    }

    let logical_request = b"\x1b[c";
    let wire_request = build_tmux_readiness_query();
    let timeout = Duration::from_millis(args.transport_timeout_ms);
    let started = Instant::now();
    let deadline = started.checked_add(timeout).ok_or(AppError::TimeoutOutOfRange)?;
    let mut preparation_exchanges = Vec::new();
    let mut readiness = TransportReadiness::Unknown;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let attempt_timeout = remaining.min(Duration::from_millis(TRANSPORT_ATTEMPT_TIMEOUT_MS));
        let live = execute_tty_exchange(
            &args.tty,
            &wire_request,
            attempt_timeout,
            args.max_response_bytes,
            da1_stop_reason,
        )?;
        let responses = parse_da1_responses(&live.response);
        let barrier_seen = !responses.is_empty();
        let events = responses
            .iter()
            .enumerate()
            .map(|(sequence, response)| {
                response.wire_event(
                    u32::try_from(sequence).unwrap_or(u32::MAX),
                    WireEventRole::BarrierReply,
                )
            })
            .collect();
        preparation_exchanges.push(wire_exchange(logical_request, &wire_request, live, events));
        if barrier_seen {
            readiness = TransportReadiness::Ready;
            break;
        }
    }

    Ok(TransportEvidence { adapter: tmux_adapter_identity(), readiness, preparation_exchanges })
}

fn kgp_receipt(args: &KgpProbeArgs) -> Result<ProbeReceiptV1, AppError> {
    let common = &args.common;
    let protocol_request = build_kgp_query(args.correlation_id)?;
    let transport = prepare_transport(common)?;
    let wire_request = match common.transport {
        TransportKind::Direct => protocol_request.clone(),
        TransportKind::TmuxPassthrough => wrap_tmux_passthrough(&protocol_request),
    };
    let live = execute_tty_exchange(
        &common.tty,
        &wire_request,
        Duration::from_millis(common.timeout_ms),
        common.max_response_bytes,
        |response| kgp_stop_reason(response, args.correlation_id),
    )?;
    let parsed = parse_kgp_exchange(&live.response, args.correlation_id);
    let exchange = wire_exchange(&protocol_request, &wire_request, live, parsed.events.clone());

    Ok(ProbeReceiptV1 {
        schema: PROBE_RECEIPT_SCHEMA_V1.to_string(),
        observed_at_unix_ms: unix_time_ms()?,
        capability: kgp_capability_id(),
        adapter: kgp_adapter_identity(),
        correlation: Some(CorrelationId {
            namespace: "org.kitty.image-id".to_string(),
            value: args.correlation_id.to_string(),
        }),
        context: ProbeContext {
            tty_endpoint: common.tty.display().to_string(),
            transport,
            environment_hints: environment_hints(),
            topology: TopologyObservation::Unknown,
        },
        exchange,
        assessment: parsed.assessment,
    })
}

fn sixel_receipt(args: &SixelProbeArgs) -> Result<ProbeReceiptV1, AppError> {
    let common = &args.common;
    let protocol_request = build_sixel_query();
    let transport = prepare_transport(common)?;
    let wire_request = match common.transport {
        TransportKind::Direct => protocol_request.clone(),
        TransportKind::TmuxPassthrough => wrap_tmux_passthrough(&protocol_request),
    };
    let live = execute_tty_exchange(
        &common.tty,
        &wire_request,
        Duration::from_millis(common.timeout_ms),
        common.max_response_bytes,
        da1_stop_reason,
    )?;
    let parsed = parse_sixel_exchange(&live.response);
    let exchange = wire_exchange(&protocol_request, &wire_request, live, parsed.events.clone());

    Ok(ProbeReceiptV1 {
        schema: PROBE_RECEIPT_SCHEMA_V1.to_string(),
        observed_at_unix_ms: unix_time_ms()?,
        capability: sixel_capability_id(),
        adapter: sixel_adapter_identity(),
        correlation: None,
        context: ProbeContext {
            tty_endpoint: common.tty.display().to_string(),
            transport,
            environment_hints: environment_hints(),
            topology: TopologyObservation::Unknown,
        },
        exchange,
        assessment: parsed.assessment,
    })
}

fn serialized_json<T: serde::Serialize>(value: &T, pretty: bool) -> Result<Vec<u8>, AppError> {
    let mut bytes =
        if pretty { serde_json::to_vec_pretty(value)? } else { serde_json::to_vec(value)? };
    bytes.push(b'\n');
    Ok(bytes)
}

fn schema_document<T: JsonSchema>(id: &str, pretty: bool) -> Result<Vec<u8>, AppError> {
    let mut value = serde_json::to_value(schema_for!(T))?;
    let root = value.as_object_mut().ok_or_else(|| {
        AppError::ContractInvalid("generated JSON Schema is not an object".into())
    })?;
    root.insert("$id".to_owned(), serde_json::Value::String(id.to_owned()));
    if let Some(property) = root
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|properties| properties.get_mut("schema"))
    {
        let description = property.get("description").cloned();
        let mut exact = serde_json::Map::new();
        if let Some(description) = description {
            exact.insert("description".to_owned(), description);
        }
        exact.insert("const".to_owned(), serde_json::Value::String(id.to_owned()));
        *property = serde_json::Value::Object(exact);
    }
    serialized_json(&value, pretty)
}

fn emit_schema_document(document: SchemaDocument, pretty: bool) -> Result<(), AppError> {
    let bytes =
        match document {
            SchemaDocument::Receipt => {
                schema_document::<ProbeReceiptV1>(PROBE_RECEIPT_SCHEMA_V1, pretty)?
            },
            SchemaDocument::ArtifactRef => schema_document::<terminal_interop_ref::ArtifactRefV1>(
                terminal_interop_ref::ARTIFACT_REF_SCHEMA_V1,
                pretty,
            )?,
            SchemaDocument::Negotiation => schema_document::<CapabilityNegotiationV1>(
                CAPABILITY_NEGOTIATION_SCHEMA_V1,
                pretty,
            )?,
            SchemaDocument::OpenIntent => {
                schema_document::<OpenIntentV1>(OPEN_INTENT_SCHEMA_V1, pretty)?
            },
            SchemaDocument::IntentReceipt => schema_document::<
                terminal_interop_intent::IntentReceiptV1,
            >(INTENT_RECEIPT_SCHEMA_V1, pretty)?,
            SchemaDocument::IntentReady => {
                schema_document::<IntentReadyV1>(INTENT_READY_SCHEMA_V1, pretty)?
            },
        };
    write_output(None, &bytes)
}

fn read_contract_document(path: &Path) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    if path == Path::new("-") {
        std::io::stdin()
            .lock()
            .take(MAX_CONTRACT_DOCUMENT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
    } else {
        File::open(path)?
            .take(MAX_CONTRACT_DOCUMENT_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
    }
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CONTRACT_DOCUMENT_BYTES {
        return Err(AppError::ContractLimit);
    }
    Ok(bytes)
}

fn validate_contract(args: &ValidateArgs) -> Result<(), AppError> {
    let bytes = read_contract_document(&args.input)?;
    let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
    let schema = envelope
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or(AppError::ContractSchemaMissing)?
        .to_owned();

    match schema.as_str() {
        PROBE_RECEIPT_SCHEMA_V1 => {
            let receipt: ProbeReceiptV1 = serde_json::from_slice(&bytes)?;
            receipt.validate().map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        CAPABILITY_NEGOTIATION_SCHEMA_V1 => {
            let negotiation: CapabilityNegotiationV1 = serde_json::from_slice(&bytes)?;
            negotiation.validate().map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        terminal_interop_ref::ARTIFACT_REF_SCHEMA_V1 => {
            let reference: terminal_interop_ref::ArtifactRefV1 = serde_json::from_slice(&bytes)?;
            reference.validate().map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        OPEN_INTENT_SCHEMA_V1 => {
            let intent: OpenIntentV1 = serde_json::from_slice(&bytes)?;
            intent
                .validate_portable()
                .map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        INTENT_RECEIPT_SCHEMA_V1 => {
            let receipt: terminal_interop_intent::IntentReceiptV1 = serde_json::from_slice(&bytes)?;
            receipt.validate().map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        INTENT_READY_SCHEMA_V1 => {
            let ready: IntentReadyV1 = serde_json::from_slice(&bytes)?;
            ready.validate().map_err(|error| AppError::ContractInvalid(error.to_string()))?;
        },
        _ => return Err(AppError::UnsupportedContractSchema(schema)),
    }

    if !args.quiet {
        write_output(None, format!("valid\t{schema}\n").as_bytes())?;
    }
    Ok(())
}

fn write_output(path: Option<&Path>, bytes: &[u8]) -> Result<(), AppError> {
    let Some(path) = path else {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.flush()?;
        return Ok(());
    };

    if path.exists() {
        return Err(AppError::OutputExists(path.to_path_buf()));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).map_err(|error| AppError::Io(error.error))?;
    Ok(())
}

fn emit_probe_receipt(receipt: &ProbeReceiptV1, args: &TtyProbeArgs) -> Result<(), AppError> {
    let bytes = serialized_json(receipt, args.pretty)?;
    write_output(args.output.as_deref(), &bytes)
}

const fn resolved_negotiation_transport(
    requested: NegotiationTransport,
    tmux_present: bool,
) -> TransportKind {
    match requested {
        NegotiationTransport::Auto if tmux_present => TransportKind::TmuxPassthrough,
        NegotiationTransport::Auto | NegotiationTransport::Direct => TransportKind::Direct,
        NegotiationTransport::TmuxPassthrough => TransportKind::TmuxPassthrough,
    }
}

fn negotiation_probe_args(args: &PixelNegotiationArgs) -> TtyProbeArgs {
    TtyProbeArgs {
        timeout_ms: args.timeout_ms,
        max_response_bytes: args.max_response_bytes,
        tty: args.tty.clone(),
        transport: resolved_negotiation_transport(args.transport, env::var_os("TMUX").is_some()),
        transport_timeout_ms: args.transport_timeout_ms,
        output: None,
        pretty: false,
    }
}

fn negotiate_pixel(args: &PixelNegotiationArgs) -> Result<(), AppError> {
    let common = negotiation_probe_args(args);
    let kgp = kgp_receipt(&KgpProbeArgs {
        correlation_id: std::process::id().max(1),
        common: common.clone(),
    })?;
    let sixel = sixel_receipt(&SixelProbeArgs { common })?;
    let negotiation = negotiate_capabilities_v1(vec![kgp, sixel]);
    let bytes = serialized_json(&negotiation, args.pretty)?;
    write_output(args.output.as_deref(), &bytes)
}

fn offer_artifact(args: &OfferArgs) -> Result<(), AppError> {
    let entry = terminal_interop_ref::Registry::discover()?.register(&args.path)?;
    let bytes = match args.format {
        OfferFormat::Short => format!("{}\n", entry.short_ref()).into_bytes(),
        OfferFormat::Uri => format!("{}\n", entry.uri()).into_bytes(),
        OfferFormat::Json => serialized_json(&entry, true)?,
    };
    write_output(None, &bytes)
}

fn intent_endpoint() -> Result<(), AppError> {
    write_output(None, format!("{}\n", EndpointId::generate()?.as_str()).as_bytes())
}

fn intent_uri(endpoint: &str, path: &Path) -> Result<(), AppError> {
    let endpoint = EndpointId::parse(endpoint)?;
    let intent = OpenIntentV1::from_path(endpoint, path)?;
    write_output(None, format!("{}\n", intent.uri()?).as_bytes())
}

fn listen_for_intents(endpoint: &str, once: bool) -> Result<(), AppError> {
    let endpoint = EndpointId::parse(endpoint)?;
    let listener = IntentListener::bind(&intent_runtime_root()?, endpoint.clone())?;
    write_output(None, &serialized_json(&IntentReadyV1::new(endpoint.clone()), false)?)?;

    loop {
        let incoming = match listener.accept() {
            Ok(incoming) => incoming,
            Err(error) => {
                eprintln!("term-interop: rejected malformed intent: {error}");
                continue;
            },
        };
        let bytes = serialized_json(&incoming.intent, false)?;
        let forward_result = write_output(None, &bytes);
        let receipt = match &forward_result {
            Ok(()) => terminal_interop_intent::IntentReceiptV1::forwarded(endpoint.clone()),
            Err(error) => terminal_interop_intent::IntentReceiptV1::rejected(
                endpoint.clone(),
                format!("consumer handoff failed: {error}"),
            ),
        };
        incoming.respond(&receipt)?;
        forward_result?;
        if once {
            return Ok(());
        }
    }
}

fn dispatch_callback(uri: &str, timeout_ms: u64, quiet: bool) -> Result<(), AppError> {
    if timeout_ms == 0 {
        return Err(AppError::ZeroIntentTimeout);
    }
    let intent = OpenIntentV1::parse_uri(uri)?;
    let receipt =
        dispatch_intent(&intent_runtime_root()?, &intent, Duration::from_millis(timeout_ms))?;
    receipt.validate()?;
    if receipt.state != IntentDeliveryState::Forwarded {
        return Err(AppError::IntentRejected(receipt.detail));
    }
    if !quiet {
        write_output(None, &serialized_json(&receipt, false)?)?;
    }
    Ok(())
}

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Offer(args) => offer_artifact(&args),
        Command::Preview(args) => preview::run(&args),
        Command::Intent { command: IntentCommand::Endpoint } => intent_endpoint(),
        Command::Intent { command: IntentCommand::Uri { endpoint, path } } => {
            intent_uri(&endpoint, &path)
        },
        Command::Intent { command: IntentCommand::Listen { endpoint, once } } => {
            listen_for_intents(&endpoint, once)
        },
        Command::Intent { command: IntentCommand::Dispatch { uri, timeout_ms, quiet } } => {
            dispatch_callback(&uri, timeout_ms, quiet)
        },
        Command::Probe { protocol: ProbeProtocol::Kgp(args) } => {
            let receipt = kgp_receipt(&args)?;
            emit_probe_receipt(&receipt, &args.common)
        },
        Command::Probe { protocol: ProbeProtocol::Sixel(args) } => {
            let receipt = sixel_receipt(&args)?;
            emit_probe_receipt(&receipt, &args.common)
        },
        Command::Negotiate { profile: NegotiationProfile::Pixel(args) } => negotiate_pixel(&args),
        Command::Schema { document, pretty } => emit_schema_document(document, pretty),
        Command::Validate(args) => validate_contract(&args),
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("term-interop: {error}");
            ExitCode::FAILURE
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_transport_values_are_presence_only() {
        let hints = environment_hints();
        for hint in hints {
            if matches!(hint.name.as_str(), "SSH_CONNECTION" | "SSH_TTY" | "TMUX" | "ZELLIJ") {
                assert_eq!(hint.observation, HintObservation::Present);
            }
        }
    }

    #[test]
    fn automatic_negotiation_transport_is_only_a_bounded_attempt_policy() {
        assert!(matches!(
            resolved_negotiation_transport(NegotiationTransport::Auto, false),
            TransportKind::Direct
        ));
        assert!(matches!(
            resolved_negotiation_transport(NegotiationTransport::Auto, true),
            TransportKind::TmuxPassthrough
        ));
    }
}
