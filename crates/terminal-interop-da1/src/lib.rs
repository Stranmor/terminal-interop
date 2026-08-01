//! ANSI/DEC primary device attributes (DA1) parsing and event mapping.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use std::collections::BTreeMap;
use terminal_interop_core::{ProtocolId, WireEvent, WireEventRole};

const ESC: u8 = 0x1b;

/// One parsed private DA1 response with exact wire offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Da1Response {
    /// Byte offset of the response in the input stream.
    pub offset: usize,
    /// Exclusive byte offset after the response.
    pub end_offset: usize,
    /// Device level followed by advertised extension identifiers.
    pub parameters: Vec<u16>,
    /// Exact parameter text, including the DEC private marker.
    pub parameter_text: String,
    /// Exact response bytes encoded with standard Base64.
    pub raw_base64: String,
}

impl Da1Response {
    /// The first DA1 parameter, representing the device level.
    #[must_use]
    pub fn device_level(&self) -> Option<u16> {
        self.parameters.first().copied()
    }

    /// Whether the response advertises one extension identifier.
    #[must_use]
    pub fn advertises(&self, extension: u16) -> bool {
        self.parameters.get(1..).is_some_and(|values| values.contains(&extension))
    }

    /// Convert this response into a protocol-neutral wire event.
    #[must_use]
    pub fn wire_event(&self, sequence: u32, role: WireEventRole) -> WireEvent {
        let mut fields = BTreeMap::new();
        fields.insert("parameters".to_string(), self.parameter_text.clone());
        if let Some(level) = self.device_level() {
            fields.insert("device_level".to_string(), level.to_string());
        }
        let extensions = self
            .parameters
            .get(1..)
            .unwrap_or_default()
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        fields.insert("extensions".to_string(), extensions);
        WireEvent {
            sequence,
            role,
            protocol: Some(protocol_id()),
            correlation: None,
            status: None,
            fields,
            raw_base64: self.raw_base64.clone(),
        }
    }
}

/// Protocol identity for the DA1 query and response exchange.
#[must_use]
pub fn protocol_id() -> ProtocolId {
    ProtocolId {
        namespace: "org.ecma".to_string(),
        name: "ansi-dec-primary-device-attributes".to_string(),
        revision: "csi-c-da1-v1".to_string(),
    }
}

/// Build a primary device attributes query.
#[must_use]
pub fn build_query() -> Vec<u8> {
    b"\x1b[c".to_vec()
}

/// Parse one complete private DA1 response at an exact byte offset.
#[must_use]
pub fn parse_response_at(input: &[u8], offset: usize) -> Option<Da1Response> {
    let prefix_end = offset.checked_add(3)?;
    if input.get(offset..prefix_end)? != [ESC, b'[', b'?'] {
        return None;
    }

    let mut end = prefix_end;
    while let Some(byte) = input.get(end) {
        match byte {
            b'0'..=b'9' | b';' => end = end.checked_add(1)?,
            b'c' => break,
            _ => return None,
        }
    }
    if input.get(end) != Some(&b'c') {
        return None;
    }

    let parameter_bytes = input.get(prefix_end..end)?;
    let parameter_body = std::str::from_utf8(parameter_bytes).ok()?;
    let parameters = parameter_body
        .split(';')
        .filter(|value| !value.is_empty())
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parameters.is_empty() {
        return None;
    }

    let end_offset = end.checked_add(1)?;
    let raw = input.get(offset..end_offset)?;
    Some(Da1Response {
        offset,
        end_offset,
        parameters,
        parameter_text: format!("?{parameter_body}"),
        raw_base64: BASE64_STANDARD.encode(raw),
    })
}

/// Parse all complete private DA1 responses from an arbitrary byte stream.
#[must_use]
pub fn parse_responses(input: &[u8]) -> Vec<Da1Response> {
    let mut responses = Vec::new();
    let mut offset = 0usize;
    while offset < input.len() {
        if let Some(response) = parse_response_at(input, offset) {
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
    fn parses_level_extensions_and_trailing_separator() {
        let responses = parse_responses(b"noise\x1b[?62;4;52;cafter");
        let response = responses.first().expect("one DA1 response");
        assert_eq!(response.device_level(), Some(62));
        assert!(response.advertises(4));
        assert!(response.advertises(52));
        assert_eq!(response.parameter_text, "?62;4;52;");
    }

    #[test]
    fn malformed_or_incomplete_sequences_are_not_evidence() {
        assert!(parse_responses(b"\x1b[?62;watc").is_empty());
        assert!(parse_responses(b"\x1b[?62;52").is_empty());
        assert!(parse_responses(b"\x1b[?c").is_empty());
    }

    #[test]
    fn preserves_multiple_response_offsets() {
        let responses = parse_responses(b"\x1b[?62;52cX\x1b[?64;4c");
        assert_eq!(responses.len(), 2);
        assert_eq!(responses.first().map(|response| response.offset), Some(0));
        assert_eq!(responses.get(1).map(|response| response.offset), Some(10));
    }
}
