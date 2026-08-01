//! Terminal pixel-geometry inquiry and bounded response parsing.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;

const ESC: u8 = 0x1b;

/// One exact response to the CSI 14 t window-area pixel inquiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPixels {
    /// Byte offset of the response in the input stream.
    pub offset: usize,
    /// Exclusive byte offset after the response.
    pub end_offset: usize,
    /// Reported text-area width in pixels.
    pub width: u32,
    /// Reported text-area height in pixels.
    pub height: u32,
    /// Exact response bytes encoded with standard Base64.
    pub raw_base64: String,
}

/// One exact response to the CSI 18 t text-area cell inquiry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowCells {
    /// Byte offset of the response in the input stream.
    pub offset: usize,
    /// Exclusive byte offset after the response.
    pub end_offset: usize,
    /// Reported text-area width in terminal cells.
    pub columns: u16,
    /// Reported text-area height in terminal cells.
    pub rows: u16,
    /// Exact response bytes encoded with standard Base64.
    pub raw_base64: String,
}

/// Build the standard window-area size query in pixels.
#[must_use]
pub fn build_window_pixels_query() -> Vec<u8> {
    b"\x1b[14t".to_vec()
}

/// Build paired pixel and cell inquiries for one coordinate-surface check.
#[must_use]
pub fn build_geometry_query() -> Vec<u8> {
    b"\x1b[14t\x1b[18t".to_vec()
}

/// Parse one complete `CSI 4 ; height ; width t` response at an exact offset.
#[must_use]
pub fn parse_window_pixels_at(input: &[u8], offset: usize) -> Option<WindowPixels> {
    let prefix_end = offset.checked_add(3)?;
    if input.get(offset..prefix_end)? != [ESC, b'[', b'4'] {
        return None;
    }

    let body_start = prefix_end.checked_add(1)?;
    if input.get(prefix_end) != Some(&b';') {
        return None;
    }
    let relative_end = input.get(body_start..)?.iter().position(|byte| *byte == b't')?;
    let end = body_start.checked_add(relative_end)?;
    let body = std::str::from_utf8(input.get(body_start..end)?).ok()?;
    let (height, width) = body.split_once(';')?;
    if height.is_empty() || width.is_empty() || width.contains(';') {
        return None;
    }
    let height = height.parse::<u32>().ok()?;
    let width = width.parse::<u32>().ok()?;
    if height == 0 || width == 0 {
        return None;
    }
    let end_offset = end.checked_add(1)?;
    let raw = input.get(offset..end_offset)?;
    Some(WindowPixels {
        offset,
        end_offset,
        width,
        height,
        raw_base64: BASE64_STANDARD.encode(raw),
    })
}

/// Parse all complete window-area pixel responses from an arbitrary byte stream.
#[must_use]
pub fn parse_window_pixels(input: &[u8]) -> Vec<WindowPixels> {
    let mut responses = Vec::new();
    let mut offset = 0usize;
    while offset < input.len() {
        if let Some(response) = parse_window_pixels_at(input, offset) {
            offset = response.end_offset;
            responses.push(response);
        } else {
            offset = offset.saturating_add(1);
        }
    }
    responses
}

/// Parse one complete `CSI 8 ; rows ; columns t` response at an exact offset.
#[must_use]
pub fn parse_window_cells_at(input: &[u8], offset: usize) -> Option<WindowCells> {
    let prefix_end = offset.checked_add(3)?;
    if input.get(offset..prefix_end)? != [ESC, b'[', b'8'] {
        return None;
    }

    let body_start = prefix_end.checked_add(1)?;
    if input.get(prefix_end) != Some(&b';') {
        return None;
    }
    let relative_end = input.get(body_start..)?.iter().position(|byte| *byte == b't')?;
    let end = body_start.checked_add(relative_end)?;
    let body = std::str::from_utf8(input.get(body_start..end)?).ok()?;
    let (rows, columns) = body.split_once(';')?;
    if rows.is_empty() || columns.is_empty() || columns.contains(';') {
        return None;
    }
    let rows = rows.parse::<u16>().ok()?;
    let columns = columns.parse::<u16>().ok()?;
    if rows == 0 || columns == 0 {
        return None;
    }
    let end_offset = end.checked_add(1)?;
    let raw = input.get(offset..end_offset)?;
    Some(WindowCells { offset, end_offset, columns, rows, raw_base64: BASE64_STANDARD.encode(raw) })
}

/// Parse all complete window-area cell responses from an arbitrary byte stream.
#[must_use]
pub fn parse_window_cells(input: &[u8]) -> Vec<WindowCells> {
    let mut responses = Vec::new();
    let mut offset = 0usize;
    while offset < input.len() {
        if let Some(response) = parse_window_cells_at(input, offset) {
            offset = response.end_offset;
            responses.push(response);
        } else {
            offset = offset.saturating_add(1);
        }
    }
    responses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_window_area_pixels_with_noise() {
        let responses = parse_window_pixels(b"noise\x1b[4;1080;1920tend");
        let response = responses.first().expect("one response");
        assert_eq!(response.width, 1920);
        assert_eq!(response.height, 1080);
        assert_eq!(response.offset, 5);
    }

    #[test]
    fn rejects_zero_incomplete_and_extra_fields() {
        assert!(parse_window_pixels(b"\x1b[4;0;1920t").is_empty());
        assert!(parse_window_pixels(b"\x1b[4;1080;1920").is_empty());
        assert!(parse_window_pixels(b"\x1b[4;1;2;3t").is_empty());
    }

    #[test]
    fn parses_window_area_cells() {
        let responses = parse_window_cells(b"\x1b[8;40;120t");
        let response = responses.first().expect("one response");
        assert_eq!(response.columns, 120);
        assert_eq!(response.rows, 40);
    }
}
