use super::{
    AppError, DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS, DEFAULT_TRANSPORT_TIMEOUT_MS,
    KgpProbeArgs, RawModeGuard, SixelProbeArgs, StopReason, TransportKind, TtyProbeArgs,
    execute_tty_exchange, kgp_receipt, sixel_receipt,
};
use clap::{Args, ValueEnum};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::termios::{FlushArg, SetArg, cfmakeraw, tcflush, tcgetattr, tcsetattr};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use terminal_interop_artifact::{
    DEFAULT_MAX_INPUT_BYTES, RasterArtifact, RasterLimits, Viewport, decode_raster, fit_raster,
    resize_rgba, sanitize_text,
};
use terminal_interop_core::{Availability, Conformance, ProbeReceiptV1, TransportReadiness};
use terminal_interop_geometry::{build_geometry_query, parse_window_cells, parse_window_pixels};
use terminal_interop_kgp::{PngDisplayOptions, delete_image_number, encode_png_display};
use terminal_interop_sixel::encode_rgba_default;
use terminal_interop_tmux::wrap_passthrough;
use terminal_size::{Height, Width, terminal_size_of};

const DEFAULT_MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_GEOMETRY_TIMEOUT_MS: u64 = 500;
const INPUT_POLL_MS: u16 = 200;

/// Renderer selection for an interactive preview.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PreviewBackend {
    /// Prefer KGP, then Sixel, using only live capability evidence.
    Auto,
    /// Require Kitty Graphics Protocol.
    Kgp,
    /// Require Sixel raster graphics.
    Sixel,
}

/// Transport selection for an interactive preview.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PreviewTransport {
    /// Use tmux passthrough only when a tmux environment hint is present.
    Auto,
    /// Write protocol bytes directly to the current TTY consumer.
    Direct,
    /// Wrap pixel protocol bytes in tmux DCS passthrough.
    TmuxPassthrough,
}

/// Arguments for the consumer-facing artifact preview.
#[derive(Debug, Args)]
pub struct PreviewArgs {
    /// Text or raster artifact to preview.
    path: PathBuf,
    /// Pixel renderer policy for raster artifacts.
    #[arg(long, value_enum, default_value_t = PreviewBackend::Auto)]
    backend: PreviewBackend,
    /// Terminal transport policy for pixel protocol bytes.
    #[arg(long, value_enum, default_value_t = PreviewTransport::Auto)]
    transport: PreviewTransport,
    /// TTY endpoint used for probing, rendering, and input.
    #[arg(long, default_value = super::DEFAULT_TTY)]
    tty: PathBuf,
    /// Capability response deadline in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS)]
    timeout_ms: u64,
    /// Transport readiness deadline in milliseconds.
    #[arg(long, default_value_t = DEFAULT_TRANSPORT_TIMEOUT_MS)]
    transport_timeout_ms: u64,
    /// Geometry response deadline in milliseconds.
    #[arg(long, default_value_t = DEFAULT_GEOMETRY_TIMEOUT_MS)]
    geometry_timeout_ms: u64,
    /// Exit automatically after a bounded delay; intended for conformance tests.
    #[arg(long, hide = true)]
    exit_after_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Renderer {
    Kgp,
    Sixel,
}

enum PreparedArtifact {
    Raster(RasterArtifact),
    Text(String),
}

struct ScreenGuard {
    tty: File,
    _raw: RawModeGuard,
    image_delete: Vec<u8>,
    restored: bool,
}

impl ScreenGuard {
    fn enter(mut tty: File, image_delete: Vec<u8>) -> Result<Self, AppError> {
        let original = tcgetattr(&tty)?;
        tcflush(&tty, FlushArg::TCIFLUSH)?;
        let mut raw = original.clone();
        cfmakeraw(&mut raw);
        tcsetattr(&tty, SetArg::TCSANOW, &raw)?;
        let raw_guard = RawModeGuard { tty: tty.try_clone()?, original };
        tty.write_all(b"\x1b[?1049h\x1b[2J\x1b[H\x1b[?25l")?;
        tty.flush()?;
        Ok(Self { tty, _raw: raw_guard, image_delete, restored: false })
    }

    const fn tty_mut(&mut self) -> &mut File {
        &mut self.tty
    }

    fn restore(&mut self) -> Result<(), AppError> {
        if self.restored {
            return Ok(());
        }
        self.tty.write_all(&self.image_delete)?;
        self.tty.write_all(b"\x1b[?25h\x1b[?1049l")?;
        self.tty.flush()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for ScreenGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn resolved_transport(requested: PreviewTransport) -> TransportKind {
    match requested {
        PreviewTransport::Auto if env::var_os("TMUX").is_some() => TransportKind::TmuxPassthrough,
        PreviewTransport::Auto | PreviewTransport::Direct => TransportKind::Direct,
        PreviewTransport::TmuxPassthrough => TransportKind::TmuxPassthrough,
    }
}

fn common_probe_args(args: &PreviewArgs, transport: TransportKind) -> TtyProbeArgs {
    TtyProbeArgs {
        timeout_ms: args.timeout_ms,
        max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        tty: args.tty.clone(),
        transport,
        transport_timeout_ms: args.transport_timeout_ms,
        output: None,
        pretty: false,
    }
}

const fn receipt_usable(receipt: &ProbeReceiptV1) -> bool {
    matches!(receipt.assessment.availability, Availability::Available)
        && matches!(receipt.assessment.conformance, Conformance::Conformant)
        && matches!(
            receipt.context.transport.readiness,
            TransportReadiness::NotRequired | TransportReadiness::Ready
        )
}

fn receipt_summary(receipt: &ProbeReceiptV1) -> String {
    format!(
        "availability={:?}, conformance={:?}, transport={:?}",
        receipt.assessment.availability,
        receipt.assessment.conformance,
        receipt.context.transport.readiness
    )
}

fn probe_kgp(args: &PreviewArgs, transport: TransportKind) -> Result<ProbeReceiptV1, AppError> {
    let correlation_id = std::process::id().max(1);
    kgp_receipt(&KgpProbeArgs { correlation_id, common: common_probe_args(args, transport) })
}

fn probe_sixel(args: &PreviewArgs, transport: TransportKind) -> Result<ProbeReceiptV1, AppError> {
    sixel_receipt(&SixelProbeArgs { common: common_probe_args(args, transport) })
}

fn select_renderer(args: &PreviewArgs, transport: TransportKind) -> Result<Renderer, AppError> {
    match args.backend {
        PreviewBackend::Kgp => {
            let kgp = probe_kgp(args, transport)?;
            if receipt_usable(&kgp) {
                return Ok(Renderer::Kgp);
            }
            Err(AppError::NoPixelRenderer {
                kgp: receipt_summary(&kgp),
                sixel: "not_probed".to_string(),
            })
        },
        PreviewBackend::Sixel => {
            let sixel = probe_sixel(args, transport)?;
            if receipt_usable(&sixel) {
                return Ok(Renderer::Sixel);
            }
            Err(AppError::NoPixelRenderer {
                kgp: "not_probed".to_string(),
                sixel: receipt_summary(&sixel),
            })
        },
        PreviewBackend::Auto => {
            let kgp = probe_kgp(args, transport)?;
            if receipt_usable(&kgp) {
                return Ok(Renderer::Kgp);
            }
            let sixel = probe_sixel(args, transport)?;
            if receipt_usable(&sixel) {
                return Ok(Renderer::Sixel);
            }
            Err(AppError::NoPixelRenderer {
                kgp: receipt_summary(&kgp),
                sixel: receipt_summary(&sixel),
            })
        },
    }
}

fn transport_payload(transport: TransportKind, payload: &[u8]) -> Vec<u8> {
    match transport {
        TransportKind::Direct => payload.to_vec(),
        TransportKind::TmuxPassthrough => wrap_passthrough(payload),
    }
}

fn load_artifact(path: &Path) -> Result<(PathBuf, PreparedArtifact), AppError> {
    let requested = match path.to_str() {
        Some("@latest") => terminal_interop_ref::Registry::discover()?.resolve_latest()?,
        Some(reference)
            if reference.starts_with('@')
                || reference.starts_with(terminal_interop_ref::ARTIFACT_REF_URI_PREFIX) =>
        {
            terminal_interop_ref::Registry::discover()?.resolve(reference)?
        },
        _ => path.to_path_buf(),
    };
    let canonical = fs::canonicalize(requested)?;
    let file = File::open(&canonical)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(AppError::NotRegularFile(canonical));
    }
    let limit_u64 = u64::try_from(DEFAULT_MAX_INPUT_BYTES).unwrap_or(u64::MAX);
    if metadata.len() > limit_u64 {
        return Err(AppError::PreviewInputLimit {
            actual: metadata.len(),
            limit: DEFAULT_MAX_INPUT_BYTES,
        });
    }

    let read_limit = u64::try_from(DEFAULT_MAX_INPUT_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len())
            .unwrap_or(DEFAULT_MAX_INPUT_BYTES)
            .min(DEFAULT_MAX_INPUT_BYTES),
    );
    file.take(read_limit).read_to_end(&mut bytes)?;
    if bytes.len() > DEFAULT_MAX_INPUT_BYTES {
        return Err(AppError::PreviewInputLimit {
            actual: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            limit: DEFAULT_MAX_INPUT_BYTES,
        });
    }

    match decode_raster(&bytes, RasterLimits::default()) {
        Ok(raster) => Ok((canonical, PreparedArtifact::Raster(raster))),
        Err(image_error) => {
            let text = sanitize_text(&bytes, DEFAULT_MAX_TEXT_BYTES)
                .map_err(|_| AppError::Artifact(image_error))?;
            Ok((canonical, PreparedArtifact::Text(text)))
        },
    }
}

fn geometry_stop_reason(response: &[u8]) -> Option<StopReason> {
    (!parse_window_pixels(response).is_empty() && !parse_window_cells(response).is_empty())
        .then_some(StopReason::CapabilityAndBarrierObserved)
}

fn validated_tty_pixels(
    columns: u16,
    rows: u16,
    observed_columns: u16,
    observed_rows: u16,
    pixel_width: u16,
    pixel_height: u16,
) -> (Option<u32>, Option<u32>) {
    if columns != observed_columns || rows != observed_rows || pixel_width == 0 || pixel_height == 0
    {
        return (None, None);
    }
    (Some(u32::from(pixel_width)), Some(u32::from(pixel_height)))
}

fn observe_viewport(args: &PreviewArgs, transport: TransportKind) -> Result<Viewport, AppError> {
    let tty = OpenOptions::new().read(true).write(true).open(&args.tty)?;
    let (Width(columns), Height(rows)) = terminal_size_of(&tty)
        .ok_or_else(|| AppError::TerminalGeometryUnavailable(args.tty.clone()))?;
    let (tty_pixel_width, tty_pixel_height) =
        rustix::termios::tcgetwinsize(&tty).ok().map_or((None, None), |winsize| {
            validated_tty_pixels(
                columns,
                rows,
                winsize.ws_col,
                winsize.ws_row,
                winsize.ws_xpixel,
                winsize.ws_ypixel,
            )
        });
    if tty_pixel_width.is_some() && tty_pixel_height.is_some() {
        return Ok(Viewport {
            columns,
            rows,
            pixel_width: tty_pixel_width,
            pixel_height: tty_pixel_height,
        });
    }

    let logical_request = build_geometry_query();
    let wire_request = transport_payload(transport, &logical_request);
    let live = execute_tty_exchange(
        &args.tty,
        &wire_request,
        Duration::from_millis(args.geometry_timeout_ms),
        DEFAULT_MAX_RESPONSE_BYTES,
        geometry_stop_reason,
    )?;
    let pixels = parse_window_pixels(&live.response).into_iter().next();
    let cells = parse_window_cells(&live.response).into_iter().next();
    let same_coordinate_surface =
        cells.as_ref().is_some_and(|observed| observed.columns == columns && observed.rows == rows);

    Ok(Viewport {
        columns,
        rows,
        pixel_width: same_coordinate_surface
            .then(|| pixels.as_ref().map(|value| value.width))
            .flatten(),
        pixel_height: same_coordinate_surface
            .then(|| pixels.as_ref().map(|value| value.height))
            .flatten(),
    })
}

fn safe_label(path: &Path, columns: u16) -> String {
    let source = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
    source
        .chars()
        .map(|character| if character.is_control() { '�' } else { character })
        .take(usize::from(columns))
        .collect()
}

fn close_deadline(exit_after_ms: Option<u64>) -> Option<Instant> {
    exit_after_ms
        .and_then(|milliseconds| Instant::now().checked_add(Duration::from_millis(milliseconds)))
}

fn read_key(tty: &mut File, deadline: Option<Instant>) -> Result<Option<u8>, AppError> {
    loop {
        if deadline.is_some_and(|value| Instant::now() >= value) {
            return Ok(None);
        }
        let mut poll_fds = [PollFd::new(tty.as_fd(), PollFlags::POLLIN)];
        match poll(&mut poll_fds, PollTimeout::from(INPUT_POLL_MS)) {
            Ok(0) | Err(Errno::EINTR) => continue,
            Ok(_) => {},
            Err(error) => return Err(AppError::Terminal(error)),
        }
        let mut buffer = [0u8; 32];
        let read = tty.read(&mut buffer)?;
        if read == 0 {
            return Ok(None);
        }
        return Ok(buffer.first().copied());
    }
}

fn wrapped_text_lines(text: &str, columns: u16) -> Vec<String> {
    let width = usize::from(columns.max(1));
    let mut output = Vec::new();
    for line in text.lines() {
        let characters = line.chars().collect::<Vec<_>>();
        if characters.is_empty() {
            output.push(String::new());
            continue;
        }
        output.extend(characters.chunks(width).map(|chunk| chunk.iter().collect()));
    }
    if output.is_empty() {
        output.push(String::new());
    }
    output
}

fn render_text_page(
    tty: &mut File,
    label: &str,
    lines: &[String],
    offset: usize,
    viewport: Viewport,
) -> Result<(), AppError> {
    let content_rows = usize::from(viewport.rows.saturating_sub(2).max(1));
    let last_line = offset.saturating_add(content_rows).min(lines.len());
    tty.write_all(b"\x1b[2J\x1b[H\x1b[7m")?;
    write!(tty, " {label}")?;
    tty.write_all(b"\x1b[0m\r\n")?;
    for line in lines.iter().take(last_line).skip(offset) {
        tty.write_all(line.as_bytes())?;
        tty.write_all(b"\x1b[K\r\n")?;
    }
    write!(
        tty,
        "\x1b[{};1H\x1b[2K\x1b[2m q/Esc: close  Space/j: next  k: previous  {}/{}\x1b[0m",
        viewport.rows,
        offset.saturating_add(1).min(lines.len()),
        lines.len()
    )?;
    tty.flush()?;
    Ok(())
}

fn show_text(
    args: &PreviewArgs,
    path: &Path,
    text: &str,
    viewport: Viewport,
) -> Result<(), AppError> {
    let tty = OpenOptions::new().read(true).write(true).open(&args.tty)?;
    let mut screen = ScreenGuard::enter(tty, Vec::new())?;
    let label = safe_label(path, viewport.columns.saturating_sub(2));
    let lines = wrapped_text_lines(text, viewport.columns);
    let page_rows = usize::from(viewport.rows.saturating_sub(2).max(1));
    let mut offset = 0usize;
    let deadline = close_deadline(args.exit_after_ms);

    loop {
        render_text_page(screen.tty_mut(), &label, &lines, offset, viewport)?;
        let Some(key) = read_key(screen.tty_mut(), deadline)? else {
            break;
        };
        match key {
            b'q' | 0x1b => break,
            b' ' | b'j' => {
                offset = offset.saturating_add(page_rows).min(lines.len().saturating_sub(1));
            },
            b'k' => offset = offset.saturating_sub(page_rows),
            b'g' => offset = 0,
            b'G' => offset = lines.len().saturating_sub(page_rows),
            _ => {},
        }
    }
    screen.restore()
}

fn show_raster(
    args: &PreviewArgs,
    path: &Path,
    artifact: &RasterArtifact,
    viewport: Viewport,
    renderer: Renderer,
    transport: TransportKind,
) -> Result<(), AppError> {
    let placement = fit_raster(artifact.width(), artifact.height(), viewport, 2)?;
    let image_number = std::process::id().max(1);
    let logical_image = match renderer {
        Renderer::Kgp => encode_png_display(
            artifact.png(),
            PngDisplayOptions { image_number, columns: placement.columns, rows: placement.rows },
        )?,
        Renderer::Sixel => {
            let resized = resize_rgba(artifact, placement.pixel_width, placement.pixel_height)?;
            encode_rgba_default(
                &resized,
                usize::try_from(placement.pixel_width).unwrap_or(usize::MAX),
                usize::try_from(placement.pixel_height).unwrap_or(usize::MAX),
            )
            .map_err(|error| AppError::SixelEncoding(error.to_string()))?
        },
    };
    let wire_image = transport_payload(transport, &logical_image);
    let image_delete = if renderer == Renderer::Kgp {
        transport_payload(transport, &delete_image_number(image_number))
    } else {
        Vec::new()
    };
    let tty = OpenOptions::new().read(true).write(true).open(&args.tty)?;
    let mut screen = ScreenGuard::enter(tty, image_delete)?;
    let left = viewport
        .columns
        .saturating_sub(placement.columns)
        .checked_div(2)
        .unwrap_or_default()
        .saturating_add(1);
    write!(screen.tty_mut(), "\x1b[1;{left}H")?;
    screen.tty_mut().write_all(&wire_image)?;
    let label = safe_label(path, viewport.columns.saturating_sub(20));
    let backend = match renderer {
        Renderer::Kgp => "KGP",
        Renderer::Sixel => "Sixel",
    };
    write!(
        screen.tty_mut(),
        "\x1b[{};1H\x1b[2K\x1b[2m {label}  {backend}  q/Esc: close\x1b[0m",
        viewport.rows
    )?;
    screen.tty_mut().flush()?;
    let deadline = close_deadline(args.exit_after_ms);
    while let Some(key) = read_key(screen.tty_mut(), deadline)? {
        if matches!(key, b'q' | 0x1b) {
            break;
        }
    }
    screen.restore()
}

pub fn run(args: &PreviewArgs) -> Result<(), AppError> {
    let (path, artifact) = load_artifact(&args.path)?;
    let transport = resolved_transport(args.transport);
    let viewport = observe_viewport(args, transport)?;
    match artifact {
        PreparedArtifact::Text(text) => show_text(args, &path, &text, viewport),
        PreparedArtifact::Raster(raster) => {
            let renderer = select_renderer(args, transport)?;
            show_raster(args, &path, &raster, viewport, renderer, transport)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_wrapping_is_deterministic_and_preserves_blank_lines() {
        assert_eq!(wrapped_text_lines("abcd\n\nef", 2), ["ab", "cd", "", "ef"]);
    }

    #[test]
    fn label_controls_cannot_reach_the_terminal() {
        assert_eq!(safe_label(Path::new("bad\n\x1bname"), 20), "bad��name");
    }

    #[test]
    fn automatic_transport_does_not_invent_tmux_without_a_hint() {
        if env::var_os("TMUX").is_none() {
            assert!(matches!(resolved_transport(PreviewTransport::Auto), TransportKind::Direct));
        }
    }

    #[test]
    fn accepts_kernel_pixels_for_the_exact_tty_surface() {
        assert_eq!(validated_tty_pixels(98, 30, 98, 30, 882, 660), (Some(882), Some(660)));
    }

    #[test]
    fn rejects_kernel_pixels_from_another_surface_or_without_pixel_extent() {
        assert_eq!(validated_tty_pixels(98, 30, 100, 32, 900, 704), (None, None));
        assert_eq!(validated_tty_pixels(98, 30, 98, 30, 0, 0), (None, None));
    }
}
