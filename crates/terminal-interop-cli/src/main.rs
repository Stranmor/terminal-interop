//! Live terminal interoperability probe CLI.

mod preview;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use clap::{Args, Parser, Subcommand, ValueEnum};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::termios::{FlushArg, SetArg, Termios, cfmakeraw, tcflush, tcgetattr, tcsetattr};
use schemars::schema_for;
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use terminal_interop_core::{
    AdapterIdentity, CorrelationId, EnvironmentHint, HintObservation, PROBE_RECEIPT_SCHEMA_V1,
    ProbeContext, ProbeReceiptV1, StopReason, TopologyObservation, TransportEvidence,
    TransportReadiness, WireEvent, WireEventRole, WireExchange,
};
use terminal_interop_da1::parse_responses as parse_da1_responses;
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

#[derive(Debug, Parser)]
#[command(
    name = "term-interop",
    version,
    about = "Same-TTY text and pixel previews with evidence-grade capability negotiation"
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
    /// Execute a live protocol probe through one TTY chain.
    Probe {
        #[command(subcommand)]
        protocol: ProbeProtocol,
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
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SchemaDocument {
    Receipt,
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

#[derive(Debug, Args)]
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
    #[error("transport timeout must be greater than zero")]
    ZeroTransportTimeout,
    #[error("max response bytes must be greater than zero")]
    ZeroResponseLimit,
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

fn offer_artifact(args: &OfferArgs) -> Result<(), AppError> {
    let entry = terminal_interop_ref::Registry::discover()?.register(&args.path)?;
    let bytes = match args.format {
        OfferFormat::Short => format!("{}\n", entry.short_ref()).into_bytes(),
        OfferFormat::Uri => format!("{}\n", entry.uri()).into_bytes(),
        OfferFormat::Json => serialized_json(&entry, true)?,
    };
    write_output(None, &bytes)
}

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Offer(args) => offer_artifact(&args),
        Command::Preview(args) => preview::run(&args),
        Command::Probe { protocol: ProbeProtocol::Kgp(args) } => {
            let receipt = kgp_receipt(&args)?;
            emit_probe_receipt(&receipt, &args.common)
        },
        Command::Probe { protocol: ProbeProtocol::Sixel(args) } => {
            let receipt = sixel_receipt(&args)?;
            emit_probe_receipt(&receipt, &args.common)
        },
        Command::Schema { document: SchemaDocument::Receipt, pretty } => {
            let schema = schema_for!(ProbeReceiptV1);
            let bytes = serialized_json(&schema, pretty)?;
            write_output(None, &bytes)
        },
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
}
