//! Live terminal interoperability probe CLI.

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
    AdapterIdentity, EnvironmentHint, HintObservation, PROBE_RECEIPT_SCHEMA_V1, ProbeContext,
    ProbeReceiptV1, StopReason, TopologyObservation, TransportEvidence, TransportReadiness,
    WireExchange,
};
use terminal_interop_kgp::{
    ParsedExchange, adapter_identity, build_query, capability_id, parse_exchange,
};
use terminal_interop_tmux::{
    adapter_identity as tmux_adapter_identity, build_readiness_query as build_tmux_readiness_query,
    wrap_passthrough as wrap_tmux_passthrough,
};
use thiserror::Error;

const DEFAULT_TTY: &str = "/dev/tty";
const DEFAULT_TIMEOUT_MS: u64 = 750;
const DEFAULT_TRANSPORT_TIMEOUT_MS: u64 = 1_500;
const TRANSPORT_ATTEMPT_TIMEOUT_MS: u64 = 100;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Debug, Parser)]
#[command(name = "term-interop", version, about = "Evidence-grade terminal capability probes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
}

#[derive(Debug, Args)]
struct KgpProbeArgs {
    /// Positive KGP image id used to correlate the reply.
    #[arg(long, default_value_t = 31)]
    correlation_id: u32,
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

fn execute_tty_exchange(
    tty_path: &Path,
    request: &[u8],
    correlation_id: u32,
    timeout: Duration,
    max_response_bytes: usize,
) -> Result<LiveExchange, AppError> {
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
        let parsed = parse_exchange(&response, correlation_id);
        if parsed.barrier_seen {
            break if parsed.correlated_reply_seen {
                StopReason::CapabilityAndBarrierObserved
            } else {
                StopReason::BarrierObserved
            };
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
    parsed: &ParsedExchange,
) -> WireExchange {
    WireExchange {
        logical_request_base64: BASE64_STANDARD.encode(logical_request),
        wire_request_base64: BASE64_STANDARD.encode(wire_request),
        response_base64: BASE64_STANDARD.encode(&live.response),
        events: parsed.events.clone(),
        elapsed_ms: u64::try_from(live.elapsed.as_millis()).unwrap_or(u64::MAX),
        stop_reason: live.stop_reason,
    }
}

fn prepare_transport(args: &KgpProbeArgs) -> Result<TransportEvidence, AppError> {
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
            args.correlation_id,
            attempt_timeout,
            args.max_response_bytes,
        )?;
        let parsed = parse_exchange(&live.response, args.correlation_id);
        let barrier_seen = parsed.barrier_seen;
        preparation_exchanges.push(wire_exchange(logical_request, &wire_request, live, &parsed));
        if barrier_seen {
            readiness = TransportReadiness::Ready;
            break;
        }
    }

    Ok(TransportEvidence { adapter: tmux_adapter_identity(), readiness, preparation_exchanges })
}

fn kgp_receipt(args: &KgpProbeArgs) -> Result<ProbeReceiptV1, AppError> {
    let protocol_request = build_query(args.correlation_id)?;
    let transport = prepare_transport(args)?;
    let wire_request = match args.transport {
        TransportKind::Direct => protocol_request.clone(),
        TransportKind::TmuxPassthrough => wrap_tmux_passthrough(&protocol_request),
    };
    let live = execute_tty_exchange(
        &args.tty,
        &wire_request,
        args.correlation_id,
        Duration::from_millis(args.timeout_ms),
        args.max_response_bytes,
    )?;
    let parsed = parse_exchange(&live.response, args.correlation_id);

    Ok(ProbeReceiptV1 {
        schema: PROBE_RECEIPT_SCHEMA_V1.to_string(),
        observed_at_unix_ms: unix_time_ms()?,
        capability: capability_id(),
        adapter: adapter_identity(),
        correlation_id: args.correlation_id,
        context: ProbeContext {
            tty_endpoint: args.tty.display().to_string(),
            transport,
            environment_hints: environment_hints(),
            topology: TopologyObservation::Unknown,
        },
        exchange: wire_exchange(&protocol_request, &wire_request, live, &parsed),
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

fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Probe { protocol: ProbeProtocol::Kgp(args) } => {
            let receipt = kgp_receipt(&args)?;
            let bytes = serialized_json(&receipt, args.pretty)?;
            write_output(args.output.as_deref(), &bytes)
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
