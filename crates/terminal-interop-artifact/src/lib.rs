//! Consumer-neutral artifact decoding, sanitization, and viewport fitting.

use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ImageEncoder as _, ImageError, ImageReader, Limits, RgbaImage};
use std::io::Cursor;
use thiserror::Error;

/// Default maximum encoded input size for the version-one preview profile.
pub use terminal_interop_core::MAX_ARTIFACT_INPUT_BYTES_V1 as DEFAULT_MAX_INPUT_BYTES;
/// Default maximum decoded allocation: 256 MiB.
pub const DEFAULT_MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
/// Default maximum image dimension in either axis.
pub const DEFAULT_MAX_DIMENSION: u32 = 16_384;

/// Resource limits applied before an artifact can reach a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterLimits {
    /// Maximum encoded input bytes accepted by the decoder.
    pub max_input_bytes: usize,
    /// Maximum width and height accepted by the decoder.
    pub max_dimension: u32,
    /// Maximum best-effort decoder allocation.
    pub max_decoded_bytes: u64,
}

impl Default for RasterLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_dimension: DEFAULT_MAX_DIMENSION,
            max_decoded_bytes: DEFAULT_MAX_DECODED_BYTES,
        }
    }
}

/// Validated raster data shared by independent rendering backends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterArtifact {
    /// Image width in source pixels.
    width: u32,
    /// Image height in source pixels.
    height: u32,
    /// Canonical RGBA8 pixels.
    rgba: Vec<u8>,
    /// Canonical metadata-free PNG representation.
    png: Vec<u8>,
}

impl RasterArtifact {
    /// Image width in source pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in source pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Canonical RGBA8 pixels.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Canonical metadata-free PNG representation.
    #[must_use]
    pub fn png(&self) -> &[u8] {
        &self.png
    }
}

/// Terminal viewport available to a renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    /// Available terminal columns.
    pub columns: u16,
    /// Available terminal rows.
    pub rows: u16,
    /// Window width in pixels when directly observed.
    pub pixel_width: Option<u32>,
    /// Window height in pixels when directly observed.
    pub pixel_height: Option<u32>,
}

/// Aspect-preserving placement shared by KGP and Sixel renderers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterPlacement {
    /// Placement width in terminal cells.
    pub columns: u16,
    /// Placement height in terminal cells.
    pub rows: u16,
    /// Sixel raster width in pixels.
    pub pixel_width: u32,
    /// Sixel raster height in pixels.
    pub pixel_height: u32,
}

/// Artifact preparation failure before any terminal side effect.
#[derive(Debug, Error)]
pub enum ArtifactError {
    /// Encoded input exceeded the configured limit.
    #[error("artifact has {actual} bytes; limit is {limit}")]
    InputLimit { actual: usize, limit: usize },
    /// Raster decoding or canonical PNG encoding failed.
    #[error("image processing failed: {0}")]
    Image(#[from] ImageError),
    /// Terminal geometry cannot represent an image placement.
    #[error("terminal viewport must have at least one column and one row")]
    EmptyViewport,
    /// A renderer requested a zero-sized raster.
    #[error("raster target dimensions must be non-zero")]
    EmptyRasterTarget,
    /// An internal raster invariant was violated.
    #[error("validated raster state is inconsistent")]
    InvalidRasterState,
    /// Text contains NUL bytes and is treated as binary data.
    #[error("artifact is not supported text or a supported raster image")]
    UnsupportedArtifact,
}

/// Decode untrusted raster bytes under explicit resource limits.
///
/// # Errors
///
/// Returns [`ArtifactError`] when input, dimensions, allocation, or format are invalid.
pub fn decode_raster(input: &[u8], limits: RasterLimits) -> Result<RasterArtifact, ArtifactError> {
    if input.len() > limits.max_input_bytes {
        return Err(ArtifactError::InputLimit {
            actual: input.len(),
            limit: limits.max_input_bytes,
        });
    }

    let mut reader =
        ImageReader::new(Cursor::new(input)).with_guessed_format().map_err(ImageError::IoError)?;
    let mut decoder_limits = Limits::default();
    decoder_limits.max_image_width = Some(limits.max_dimension);
    decoder_limits.max_image_height = Some(limits.max_dimension);
    decoder_limits.max_alloc = Some(limits.max_decoded_bytes);
    reader.limits(decoder_limits);
    let rgba_image = reader.decode()?.into_rgba8();
    let (width, height) = rgba_image.dimensions();
    let rgba = rgba_image.into_raw();
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)?;
    Ok(RasterArtifact { width, height, rgba, png })
}

/// Fit an image into a viewport while preserving its pixel aspect ratio.
///
/// `reserved_rows` protects a footer or command hint from being covered.
///
/// # Errors
///
/// Returns [`ArtifactError::EmptyViewport`] when no drawable cell remains.
pub fn fit_raster(
    image_width: u32,
    image_height: u32,
    viewport: Viewport,
    reserved_rows: u16,
) -> Result<RasterPlacement, ArtifactError> {
    let available_rows = viewport.rows.saturating_sub(reserved_rows);
    if viewport.columns == 0 || available_rows == 0 || image_width == 0 || image_height == 0 {
        return Err(ArtifactError::EmptyViewport);
    }

    let observed_pixel_width = viewport.pixel_width.filter(|value| *value > 0).map(u64::from);
    let observed_pixel_height = viewport.pixel_height.filter(|value| *value > 0).map(u64::from);
    let available_pixel_width =
        observed_pixel_width.unwrap_or_else(|| u64::from(viewport.columns).saturating_mul(8));
    let full_pixel_height =
        observed_pixel_height.unwrap_or_else(|| u64::from(viewport.rows).saturating_mul(16));
    let available_pixel_height = full_pixel_height
        .saturating_mul(u64::from(available_rows))
        .checked_div(u64::from(viewport.rows))
        .unwrap_or_default()
        .max(1);

    let source_width = u64::from(image_width);
    let source_height = u64::from(image_height);
    let (pixel_width, pixel_height) = if source_width.saturating_mul(available_pixel_height)
        > source_height.saturating_mul(available_pixel_width)
    {
        let width = source_width.min(available_pixel_width);
        let height = source_height
            .saturating_mul(width)
            .checked_div(source_width)
            .unwrap_or_default()
            .max(1);
        (width, height)
    } else {
        let height = source_height.min(available_pixel_height);
        let width = source_width
            .saturating_mul(height)
            .checked_div(source_height)
            .unwrap_or_default()
            .max(1);
        (width, height)
    };

    let columns = pixel_width
        .saturating_mul(u64::from(viewport.columns))
        .saturating_add(available_pixel_width.saturating_sub(1))
        .checked_div(available_pixel_width)
        .unwrap_or_default()
        .clamp(1, u64::from(viewport.columns));
    let rows = pixel_height
        .saturating_mul(u64::from(viewport.rows))
        .saturating_add(full_pixel_height.saturating_sub(1))
        .checked_div(full_pixel_height)
        .unwrap_or_default()
        .clamp(1, u64::from(available_rows));

    Ok(RasterPlacement {
        columns: u16::try_from(columns).unwrap_or(viewport.columns),
        rows: u16::try_from(rows).unwrap_or(available_rows),
        pixel_width: u32::try_from(pixel_width).unwrap_or(u32::MAX),
        pixel_height: u32::try_from(pixel_height).unwrap_or(u32::MAX),
    })
}

/// Resize validated RGBA pixels for a pixel-addressed renderer.
///
/// # Errors
///
/// Returns [`ArtifactError`] when the target is empty or an internal invariant is broken.
pub fn resize_rgba(
    artifact: &RasterArtifact,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ArtifactError> {
    if width == 0 || height == 0 {
        return Err(ArtifactError::EmptyRasterTarget);
    }
    let source = RgbaImage::from_raw(artifact.width, artifact.height, artifact.rgba.clone())
        .ok_or(ArtifactError::InvalidRasterState)?;
    Ok(image::imageops::resize(&source, width, height, FilterType::Lanczos3).into_raw())
}

/// Convert text bytes into terminal-safe UTF-8 without interpreting control sequences.
///
/// # Errors
///
/// Returns [`ArtifactError::UnsupportedArtifact`] for NUL-containing binary input.
pub fn sanitize_text(input: &[u8], max_bytes: usize) -> Result<String, ArtifactError> {
    let bounded = input.get(..input.len().min(max_bytes)).unwrap_or_default();
    if bounded.contains(&0) {
        return Err(ArtifactError::UnsupportedArtifact);
    }
    let text = String::from_utf8_lossy(bounded);
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' | '\r' | '\t' => output.push(character),
            '\u{1b}' => output.push('␛'),
            value if value.is_control() => output.push('�'),
            value => output.push(value),
        }
    }
    if input.len() > max_bytes {
        output.push_str("\n… [preview truncated]\n");
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_wide_image_without_exceeding_viewport() {
        let placement = fit_raster(
            1920,
            1080,
            Viewport { columns: 120, rows: 40, pixel_width: Some(1200), pixel_height: Some(800) },
            2,
        )
        .expect("drawable viewport");
        assert!(placement.columns <= 120);
        assert!(placement.rows <= 38);
        assert_eq!(placement.pixel_width, 1200);
        assert_eq!(placement.pixel_height, 675);
    }

    #[test]
    fn sanitizes_escape_sequences_as_text() {
        let rendered = sanitize_text(b"safe\x1b[2Jtext", 100).expect("text");
        assert_eq!(rendered, "safe␛[2Jtext");
    }

    #[test]
    fn rejects_nul_containing_binary_text() {
        assert!(matches!(sanitize_text(b"a\0b", 100), Err(ArtifactError::UnsupportedArtifact)));
    }
}
