//! Versioned local callback intents for returning an external terminal hyperlink to its exact TUI.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

pub const OPEN_INTENT_SCHEMA_V1: &str = "urn:terminal-interop:open-intent:v1";
pub const INTENT_RECEIPT_SCHEMA_V1: &str = "urn:terminal-interop:intent-receipt:v1";
pub const INTENT_READY_SCHEMA_V1: &str = "urn:terminal-interop:intent-ready:v1";
pub const INTENT_URI_PREFIX_V1: &str = "terminal-interop-intent://v1/open/";

const ENDPOINT_HEX_BYTES: usize = 16;
const ENDPOINT_HEX_LEN: usize = ENDPOINT_HEX_BYTES * 2;
const MAX_TARGET_BYTES: usize = 16 * 1024;
const MAX_FRAME_BYTES: usize = 64 * 1024;
const SOCKET_PATH_SOFT_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct EndpointId(String);

impl EndpointId {
    /// Generate an unguessable 128-bit local endpoint identity.
    ///
    /// # Errors
    ///
    /// Returns [`IntentError::Random`] when the operating system random source is unavailable.
    pub fn generate() -> Result<Self, IntentError> {
        let mut bytes = [0u8; ENDPOINT_HEX_BYTES];
        getrandom::fill(&mut bytes).map_err(|error| IntentError::Random(error.to_string()))?;
        let mut value = String::with_capacity(ENDPOINT_HEX_LEN);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Ok(Self(value))
    }

    /// Validate one canonical lowercase endpoint representation.
    ///
    /// # Errors
    ///
    /// Returns [`IntentError::InvalidEndpoint`] for any non-canonical representation.
    pub fn parse(value: &str) -> Result<Self, IntentError> {
        if value.len() != ENDPOINT_HEX_LEN
            || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(IntentError::InvalidEndpoint);
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntentAction {
    OpenArtifact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathEncoding {
    UnixBytesBase64urlV1,
    Utf8Base64urlV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EncodedPathV1 {
    pub encoding: PathEncoding,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenIntentV1 {
    pub schema: String,
    pub endpoint: EndpointId,
    pub action: IntentAction,
    pub target: EncodedPathV1,
}

impl OpenIntentV1 {
    /// Construct an open intent from an exact absolute path.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is relative, empty, NUL-containing, or over the v1 bound.
    pub fn from_path(endpoint: EndpointId, path: &Path) -> Result<Self, IntentError> {
        if !path.is_absolute() {
            return Err(IntentError::TargetNotAbsolute);
        }
        let (encoding, bytes) = path_bytes(path);
        validate_target_bytes(bytes)?;
        Ok(Self {
            schema: OPEN_INTENT_SCHEMA_V1.to_owned(),
            endpoint,
            action: IntentAction::OpenArtifact,
            target: EncodedPathV1 { encoding, value: URL_SAFE_NO_PAD.encode(bytes) },
        })
    }

    /// Decode and validate the exact path carried by this intent.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported schemas or encodings and invalid or relative paths.
    pub fn path(&self) -> Result<PathBuf, IntentError> {
        if self.schema != OPEN_INTENT_SCHEMA_V1 {
            return Err(IntentError::UnsupportedSchema(self.schema.clone()));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.target.value)
            .map_err(|_| IntentError::InvalidTargetEncoding)?;
        validate_target_bytes(&bytes)?;
        let path = path_from_bytes(self.target.encoding, bytes)?;
        if !path.is_absolute() {
            return Err(IntentError::TargetNotAbsolute);
        }
        Ok(path)
    }

    /// Encode this intent as its canonical callback URI.
    ///
    /// # Errors
    ///
    /// Returns an error when the embedded intent path is invalid.
    pub fn uri(&self) -> Result<String, IntentError> {
        let path = self.path()?;
        let (_, bytes) = path_bytes(&path);
        Ok(format!(
            "{INTENT_URI_PREFIX_V1}{}/{}",
            self.endpoint.as_str(),
            URL_SAFE_NO_PAD.encode(bytes)
        ))
    }

    /// Parse and validate one canonical callback URI.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed endpoints, components, encodings, or target paths.
    pub fn parse_uri(uri: &str) -> Result<Self, IntentError> {
        if uri.contains(['?', '#']) {
            return Err(IntentError::UnsupportedUriComponent);
        }
        let suffix = uri.strip_prefix(INTENT_URI_PREFIX_V1).ok_or(IntentError::InvalidUri)?;
        let mut segments = suffix.split('/');
        let endpoint = EndpointId::parse(segments.next().ok_or(IntentError::InvalidUri)?)?;
        let encoded = segments.next().ok_or(IntentError::InvalidUri)?;
        if encoded.is_empty() || segments.next().is_some() {
            return Err(IntentError::InvalidUri);
        }
        let encoding = if cfg!(unix) {
            PathEncoding::UnixBytesBase64urlV1
        } else {
            PathEncoding::Utf8Base64urlV1
        };
        let intent = Self {
            schema: OPEN_INTENT_SCHEMA_V1.to_owned(),
            endpoint,
            action: IntentAction::OpenArtifact,
            target: EncodedPathV1 { encoding, value: encoded.to_owned() },
        };
        intent.path()?;
        Ok(intent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IntentDeliveryState {
    Forwarded,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentReceiptV1 {
    pub schema: String,
    pub endpoint: EndpointId,
    pub state: IntentDeliveryState,
    pub detail: String,
}

impl IntentReceiptV1 {
    #[must_use]
    pub fn forwarded(endpoint: EndpointId) -> Self {
        Self {
            schema: INTENT_RECEIPT_SCHEMA_V1.to_owned(),
            endpoint,
            state: IntentDeliveryState::Forwarded,
            detail: "intent was forwarded to the bound local consumer".to_owned(),
        }
    }

    #[must_use]
    pub fn rejected(endpoint: EndpointId, detail: impl Into<String>) -> Self {
        Self {
            schema: INTENT_RECEIPT_SCHEMA_V1.to_owned(),
            endpoint,
            state: IntentDeliveryState::Rejected,
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentReadyV1 {
    pub schema: String,
    pub endpoint: EndpointId,
    pub uri_prefix: String,
}

impl IntentReadyV1 {
    #[must_use]
    pub fn new(endpoint: EndpointId) -> Self {
        Self {
            schema: INTENT_READY_SCHEMA_V1.to_owned(),
            uri_prefix: format!("{INTENT_URI_PREFIX_V1}{}/", endpoint.as_str()),
            endpoint,
        }
    }
}

#[derive(Debug, Error)]
pub enum IntentError {
    #[error("cannot obtain secure random bytes: {0}")]
    Random(String),
    #[error("intent endpoint must be 32 lowercase hexadecimal characters")]
    InvalidEndpoint,
    #[error("intent URI is invalid")]
    InvalidUri,
    #[error("intent URI query and fragment components are not supported")]
    UnsupportedUriComponent,
    #[error("intent target encoding is invalid")]
    InvalidTargetEncoding,
    #[error("intent target is empty, contains NUL, or exceeds the {MAX_TARGET_BYTES}-byte bound")]
    InvalidTargetBytes,
    #[error("intent target must be an absolute path")]
    TargetNotAbsolute,
    #[error("path encoding is unsupported on this platform")]
    UnsupportedPathEncoding,
    #[error("unsupported intent schema: {0}")]
    UnsupportedSchema(String),
    #[error("XDG_RUNTIME_DIR is missing or is not absolute")]
    RuntimeDirectoryUnavailable,
    #[error("intent runtime directory is not private: {0}")]
    RuntimeDirectoryNotPrivate(PathBuf),
    #[error("intent socket path is too long: {0}")]
    SocketPathTooLong(PathBuf),
    #[error("intent endpoint is already bound: {0}")]
    EndpointInUse(PathBuf),
    #[error("intent frame exceeds the {MAX_FRAME_BYTES}-byte bound")]
    FrameTooLarge,
    #[error("intent peer closed before a complete frame was received")]
    TruncatedFrame,
    #[error("intent response endpoint does not match the request")]
    EndpointMismatch,
    #[error("intent transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("intent JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

fn validate_target_bytes(bytes: &[u8]) -> Result<(), IntentError> {
    if bytes.is_empty() || bytes.len() > MAX_TARGET_BYTES || bytes.contains(&0) {
        return Err(IntentError::InvalidTargetBytes);
    }
    Ok(())
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> (PathEncoding, &[u8]) {
    (PathEncoding::UnixBytesBase64urlV1, path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> (PathEncoding, &[u8]) {
    (PathEncoding::Utf8Base64urlV1, path.as_os_str().as_encoded_bytes())
}

#[cfg(unix)]
fn path_from_bytes(encoding: PathEncoding, bytes: Vec<u8>) -> Result<PathBuf, IntentError> {
    if encoding != PathEncoding::UnixBytesBase64urlV1 {
        return Err(IntentError::UnsupportedPathEncoding);
    }
    Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn path_from_bytes(encoding: PathEncoding, bytes: Vec<u8>) -> Result<PathBuf, IntentError> {
    if encoding != PathEncoding::Utf8Base64urlV1 {
        return Err(IntentError::UnsupportedPathEncoding);
    }
    let value = String::from_utf8(bytes).map_err(|_| IntentError::InvalidTargetEncoding)?;
    Ok(PathBuf::from(value))
}

/// Discover the private per-user runtime root for v1 intent sockets.
///
/// # Errors
///
/// Returns [`IntentError::RuntimeDirectoryUnavailable`] when `XDG_RUNTIME_DIR` is absent or
/// relative.
pub fn runtime_root() -> Result<PathBuf, IntentError> {
    let root = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(IntentError::RuntimeDirectoryUnavailable)?;
    Ok(root.join("terminal-interop").join("intent-v1"))
}

/// Derive the bounded socket path for one endpoint under an explicit runtime root.
///
/// # Errors
///
/// Returns [`IntentError::SocketPathTooLong`] when the resulting Unix socket path is unsafe.
pub fn socket_path(root: &Path, endpoint: &EndpointId) -> Result<PathBuf, IntentError> {
    let path = root.join(format!("{}.sock", endpoint.as_str()));
    if path.as_os_str().as_encoded_bytes().len() > SOCKET_PATH_SOFT_LIMIT {
        return Err(IntentError::SocketPathTooLong(path));
    }
    Ok(path)
}

#[cfg(unix)]
fn prepare_runtime_root(root: &Path) -> Result<(), IntentError> {
    fs::create_dir_all(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(IntentError::RuntimeDirectoryNotPrivate(root.to_path_buf()));
    }
    Ok(())
}

#[cfg(unix)]
pub struct IntentListener {
    listener: UnixListener,
    socket_path: PathBuf,
    endpoint: EndpointId,
}

#[cfg(unix)]
impl IntentListener {
    /// Bind a private local endpoint without replacing an existing socket.
    ///
    /// # Errors
    ///
    /// Returns an error when the runtime root cannot be made private, the endpoint already exists,
    /// or the Unix listener cannot be bound.
    pub fn bind(root: &Path, endpoint: EndpointId) -> Result<Self, IntentError> {
        prepare_runtime_root(root)?;
        let socket_path = socket_path(root, &endpoint)?;
        if socket_path.exists() {
            return Err(IntentError::EndpointInUse(socket_path));
        }
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        Ok(Self { listener, socket_path, endpoint })
    }

    /// Accept and validate one bounded intent request.
    ///
    /// # Errors
    ///
    /// Returns an error for transport failures, malformed frames, endpoint mismatch, or invalid
    /// intent data.
    pub fn accept(&self) -> Result<IncomingIntent, IntentError> {
        let (mut stream, _) = self.listener.accept()?;
        let intent: OpenIntentV1 = read_frame(&mut stream)?;
        if intent.endpoint != self.endpoint {
            let receipt = IntentReceiptV1::rejected(
                self.endpoint.clone(),
                "request endpoint does not match the bound consumer",
            );
            let _ = write_frame(&mut stream, &receipt);
            return Err(IntentError::EndpointMismatch);
        }
        intent.path()?;
        Ok(IncomingIntent { intent, stream })
    }

    #[must_use]
    pub const fn endpoint(&self) -> &EndpointId {
        &self.endpoint
    }
}

#[cfg(unix)]
impl Drop for IntentListener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
pub struct IncomingIntent {
    pub intent: OpenIntentV1,
    stream: UnixStream,
}

#[cfg(unix)]
impl IncomingIntent {
    /// Return one typed forwarding or rejection receipt to the dispatcher.
    ///
    /// # Errors
    ///
    /// Returns an error when the bounded response frame cannot be serialized or written.
    pub fn respond(mut self, receipt: &IntentReceiptV1) -> Result<(), IntentError> {
        write_frame(&mut self.stream, receipt)
    }
}

#[cfg(unix)]
/// Deliver one validated intent and wait for its listener receipt.
///
/// # Errors
///
/// Returns an error when the endpoint is unavailable, the bounded exchange times out, the peer
/// returns malformed data, or the receipt names another endpoint.
pub fn dispatch(
    root: &Path,
    intent: &OpenIntentV1,
    timeout: Duration,
) -> Result<IntentReceiptV1, IntentError> {
    let socket = socket_path(root, &intent.endpoint)?;
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_frame(&mut stream, intent)?;
    let receipt: IntentReceiptV1 = read_frame(&mut stream)?;
    if receipt.endpoint != intent.endpoint {
        return Err(IntentError::EndpointMismatch);
    }
    Ok(receipt)
}

#[cfg(unix)]
fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), IntentError> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IntentError::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| IntentError::FrameTooLarge)?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(&payload)?;
    stream.flush()?;
    Ok(())
}

#[cfg(unix)]
fn read_frame<T: for<'de> Deserialize<'de>>(stream: &mut UnixStream) -> Result<T, IntentError> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).map_err(map_frame_read_error)?;
    let length = usize::try_from(u32::from_be_bytes(header)).unwrap_or(usize::MAX);
    if length > MAX_FRAME_BYTES {
        return Err(IntentError::FrameTooLarge);
    }
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).map_err(map_frame_read_error)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(unix)]
fn map_frame_read_error(error: std::io::Error) -> IntentError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        IntentError::TruncatedFrame
    } else {
        IntentError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_uri_round_trip_preserves_the_exact_absolute_path() {
        let endpoint = EndpointId::parse("0123456789abcdef0123456789abcdef")
            .expect("fixture endpoint should be valid");
        let intent = OpenIntentV1::from_path(endpoint, Path::new("/tmp/agent artifact#1.png"))
            .expect("absolute fixture path should encode");
        let uri = intent.uri().expect("fixture intent should produce a URI");
        let parsed = OpenIntentV1::parse_uri(&uri).expect("generated URI should parse");
        assert_eq!(parsed, intent);
        assert_eq!(
            parsed.path().expect("parsed fixture path should decode"),
            Path::new("/tmp/agent artifact#1.png")
        );
    }

    #[test]
    fn uri_rejects_relative_nul_and_extra_components() {
        let endpoint = EndpointId::parse("0123456789abcdef0123456789abcdef")
            .expect("fixture endpoint should be valid");
        assert!(OpenIntentV1::from_path(endpoint, Path::new("relative.png")).is_err());
        assert!(
            OpenIntentV1::parse_uri(
                "terminal-interop-intent://v1/open/0123456789abcdef0123456789abcdef/L3RtcC9h?x=1"
            )
            .is_err()
        );
        assert!(
            OpenIntentV1::parse_uri(
                "terminal-interop-intent://v1/open/0123456789abcdef0123456789abcdef/L3RtcC8AYQ"
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_transport_returns_a_typed_forwarding_receipt() {
        let temporary = tempfile::tempdir().expect("temporary runtime should be created");
        let root = temporary.path().join("intent-v1");
        let endpoint = EndpointId::parse("fedcba9876543210fedcba9876543210")
            .expect("fixture endpoint should be valid");
        let listener =
            IntentListener::bind(&root, endpoint.clone()).expect("fixture listener should bind");
        let intent = OpenIntentV1::from_path(endpoint.clone(), Path::new("/tmp/image.png"))
            .expect("absolute fixture path should encode");
        let server = std::thread::spawn(move || {
            let incoming = listener.accept().expect("fixture request should arrive");
            assert_eq!(
                incoming.intent.path().expect("fixture path should decode"),
                Path::new("/tmp/image.png")
            );
            incoming
                .respond(&IntentReceiptV1::forwarded(endpoint))
                .expect("fixture receipt should be sent");
        });
        let receipt = dispatch(&root, &intent, Duration::from_secs(1))
            .expect("fixture callback should be forwarded");
        assert_eq!(receipt.state, IntentDeliveryState::Forwarded);
        server.join().expect("fixture listener thread should finish");
    }
}
