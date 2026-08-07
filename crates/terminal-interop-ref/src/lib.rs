//! Short, non-path artifact references with identity-preserving local resolution.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use terminal_interop_core::MAX_ARTIFACT_INPUT_BYTES_V1;
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

/// Stable schema identity for local artifact references.
pub const ARTIFACT_REF_SCHEMA_V1: &str = "urn:terminal-interop:artifact-ref:v1";
/// URI prefix suitable for hyperlinks and desktop dispatch adapters.
pub const ARTIFACT_REF_URI_PREFIX: &str = "terminal-interop://artifact/";
const TOKEN_BYTES: usize = 8;
const TOKEN_LENGTH: usize = 13;
const TOKEN_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const MAX_ENTRY_BYTES: u64 = 64 * 1024;

/// File identity captured when an agent offers an artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    /// Encoded file length.
    pub size: u64,
    /// Last modification time in nanoseconds since the Unix epoch, when available.
    pub modified_unix_nanos: Option<u128>,
    /// Platform device identity, when available.
    pub device: Option<u64>,
    /// Platform inode identity, when available.
    pub inode: Option<u64>,
    /// SHA-256 of exact file contents encoded with standard Base64.
    pub content_sha256_base64: String,
}

/// Portable local registry entry. Paths are data, never shell source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRefV1 {
    /// Stable schema identifier.
    pub schema: String,
    /// Short opaque reference token.
    #[schemars(regex(pattern = "^[0-9A-HJKMNP-TV-Z]{13}$"))]
    pub token: String,
    /// Encoding profile for `path_base64`.
    pub path_encoding: String,
    /// Exact platform path bytes encoded with standard Base64.
    pub path_base64: String,
    /// File identity observed at registration.
    pub identity: FileIdentity,
    /// Registration time in milliseconds since the Unix epoch.
    pub registered_at_unix_ms: u64,
}

/// Portable structural failure in an artifact reference record.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ArtifactRefValidationError {
    /// The record does not use the v1 schema identity.
    #[error("unsupported artifact reference schema: {0:?}")]
    UnsupportedSchema(String),
    /// The token is not the canonical ambiguity-reduced Base32 representation.
    #[error("artifact reference token is not canonical")]
    InvalidToken,
    /// The path encoding profile is not defined by v1.
    #[error("unsupported artifact path encoding: {0:?}")]
    UnsupportedPathEncoding(String),
    /// The encoded path is malformed, empty, relative where detectable, or contains NUL.
    #[error("artifact reference path is invalid")]
    InvalidPath,
    /// The content digest is not one standard-Base64 SHA-256 value.
    #[error("artifact reference digest is not a SHA-256 value in standard Base64")]
    InvalidDigest,
    /// The declared source size exceeds the shared v1 preview profile.
    #[error("artifact reference size {actual} exceeds the {limit}-byte v1 limit")]
    ArtifactLimit {
        /// Declared source size.
        actual: u64,
        /// Shared v1 input limit.
        limit: usize,
    },
}

impl ArtifactRefV1 {
    /// Shell-friendly short reference.
    #[must_use]
    pub fn short_ref(&self) -> String {
        format!("@{}", self.token)
    }

    /// URI that does not expose or duplicate the original path.
    #[must_use]
    pub fn uri(&self) -> String {
        format!("{ARTIFACT_REF_URI_PREFIX}{}", self.token)
    }

    /// Validate the portable record without resolving or opening its local file.
    ///
    /// This establishes schema identity, canonical token syntax, path encoding structure, digest
    /// shape, and the shared resource bound. [`Registry::resolve`] additionally proves that the
    /// current local file still has the recorded identity.
    ///
    /// # Errors
    ///
    /// Returns an error when any portable v1 invariant is violated.
    pub fn validate(&self) -> Result<(), ArtifactRefValidationError> {
        if self.schema != ARTIFACT_REF_SCHEMA_V1 {
            return Err(ArtifactRefValidationError::UnsupportedSchema(self.schema.clone()));
        }
        parse_token(&self.token).map_err(|_| ArtifactRefValidationError::InvalidToken)?;

        let path = BASE64_STANDARD
            .decode(&self.path_base64)
            .map_err(|_| ArtifactRefValidationError::InvalidPath)?;
        if path.is_empty() || path.contains(&0) {
            return Err(ArtifactRefValidationError::InvalidPath);
        }
        match self.path_encoding.as_str() {
            "unix-bytes-v1" => {
                if path.first() != Some(&b'/') {
                    return Err(ArtifactRefValidationError::InvalidPath);
                }
            },
            "utf8-v1" => {
                let value = std::str::from_utf8(&path)
                    .map_err(|_| ArtifactRefValidationError::InvalidPath)?;
                if !is_windows_absolute(value) {
                    return Err(ArtifactRefValidationError::InvalidPath);
                }
            },
            other => {
                return Err(ArtifactRefValidationError::UnsupportedPathEncoding(other.to_owned()));
            },
        }

        let digest = BASE64_STANDARD
            .decode(&self.identity.content_sha256_base64)
            .map_err(|_| ArtifactRefValidationError::InvalidDigest)?;
        if digest.len() != 32 {
            return Err(ArtifactRefValidationError::InvalidDigest);
        }
        if self.identity.size > u64::try_from(MAX_ARTIFACT_INPUT_BYTES_V1).unwrap_or(u64::MAX) {
            return Err(ArtifactRefValidationError::ArtifactLimit {
                actual: self.identity.size,
                limit: MAX_ARTIFACT_INPUT_BYTES_V1,
            });
        }
        Ok(())
    }
}

/// Local artifact registry rooted in an explicit state directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    root: PathBuf,
}

/// Artifact reference or registry failure.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// No safe state root could be discovered.
    #[error("cannot discover state directory; set TERM_INTEROP_STATE_DIR or HOME")]
    StateDirectoryUnavailable,
    /// Filesystem operation failed.
    #[error("registry I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// Entry serialization or decoding failed.
    #[error("registry JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Cryptographic operating-system randomness was unavailable.
    #[error("cannot generate artifact reference token")]
    Random,
    /// Reference syntax was invalid.
    #[error("invalid artifact reference")]
    InvalidReference,
    /// Reference was syntactically valid but absent.
    #[error("artifact reference does not exist: @{0}")]
    NotFound(String),
    /// The referenced path is not a regular file.
    #[error("artifact is not a regular file: {0}")]
    NotRegularFile(PathBuf),
    /// The file changed identity after it was offered.
    #[error("artifact changed after registration: @{0}")]
    IdentityChanged(String),
    /// Stored path encoding is unsupported on this platform.
    #[error("unsupported path encoding: {0}")]
    UnsupportedPathEncoding(String),
    /// The system clock cannot produce a portable timestamp.
    #[error("system clock is before the Unix epoch")]
    ClockBeforeEpoch,
    /// Repeated random token collisions prevented registration.
    #[error("cannot allocate a unique artifact reference token")]
    TokenExhausted,
    /// A registry entry exceeded its structural resource bound.
    #[error("artifact reference entry exceeds the 65536-byte limit")]
    EntryLimit,
    /// The offered artifact exceeded the shared preview resource profile.
    #[error("artifact has {actual} bytes; reference profile limit is {limit}")]
    ArtifactLimit { actual: u64, limit: usize },
}

impl Registry {
    /// Discover the state root from explicit or standard environment owners.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::StateDirectoryUnavailable`] when no root exists.
    pub fn discover() -> Result<Self, RegistryError> {
        if let Some(root) = env::var_os("TERM_INTEROP_STATE_DIR") {
            return Ok(Self::new(PathBuf::from(root)));
        }
        if let Some(root) = env::var_os("XDG_STATE_HOME") {
            return Ok(Self::new(PathBuf::from(root).join("terminal-interop")));
        }
        env::var_os("HOME")
            .map(|home| Self::new(PathBuf::from(home).join(".local/state/terminal-interop")))
            .ok_or(RegistryError::StateDirectoryUnavailable)
    }

    /// Construct a registry at an explicit root.
    #[must_use]
    pub const fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Registry root used by this instance.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn refs_dir(&self) -> PathBuf {
        self.root.join("refs")
    }

    fn entry_path(&self, token: &str) -> PathBuf {
        self.refs_dir().join(format!("{token}.json"))
    }

    fn ensure_dirs(&self) -> Result<(), RegistryError> {
        fs::create_dir_all(self.refs_dir())?;
        #[cfg(unix)]
        {
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(self.refs_dir(), fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// Register one exact regular file and update the session-neutral latest pointer.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] when validation, persistence, or token creation fails.
    pub fn register(&self, path: &Path) -> Result<ArtifactRefV1, RegistryError> {
        self.ensure_dirs()?;
        let canonical = fs::canonicalize(path)?;
        let mut source = File::open(&canonical)?;
        let metadata = source.metadata()?;
        if !metadata.is_file() {
            return Err(RegistryError::NotRegularFile(canonical));
        }
        enforce_artifact_limit(metadata.len())?;
        let identity = file_identity(&metadata, hash_file(&mut source)?);
        let (path_encoding, path_bytes) = encode_path(&canonical);
        let registered_at_unix_ms = unix_time_ms()?;

        for _ in 0..16 {
            let token = random_token()?;
            let entry = ArtifactRefV1 {
                schema: ARTIFACT_REF_SCHEMA_V1.to_string(),
                token: token.clone(),
                path_encoding: path_encoding.to_string(),
                path_base64: BASE64_STANDARD.encode(&path_bytes),
                identity: identity.clone(),
                registered_at_unix_ms,
            };
            let bytes = serde_json::to_vec(&entry)?;
            let mut temporary = NamedTempFile::new_in(self.refs_dir())?;
            temporary.write_all(&bytes)?;
            temporary.flush()?;
            temporary.as_file().sync_all()?;
            match temporary.persist_noclobber(self.entry_path(&token)) {
                Ok(_) => {
                    self.write_latest(&token)?;
                    return Ok(entry);
                },
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(error) => return Err(RegistryError::Io(error.error)),
            }
        }
        Err(RegistryError::TokenExhausted)
    }

    /// Resolve and revalidate one short reference, URI, or bare token.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] when syntax, state, identity, or path decoding fails.
    pub fn resolve(&self, reference: &str) -> Result<PathBuf, RegistryError> {
        let token = parse_token(reference)?;
        let path = self.entry_path(token);
        let file = File::open(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RegistryError::NotFound(token.to_string())
            } else {
                RegistryError::Io(error)
            }
        })?;
        if file.metadata()?.len() > MAX_ENTRY_BYTES {
            return Err(RegistryError::EntryLimit);
        }
        let mut bytes = Vec::new();
        file.take(MAX_ENTRY_BYTES.saturating_add(1)).read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_ENTRY_BYTES {
            return Err(RegistryError::EntryLimit);
        }
        let entry: ArtifactRefV1 = serde_json::from_slice(&bytes)?;
        entry.validate().map_err(|error| match error {
            ArtifactRefValidationError::UnsupportedPathEncoding(encoding) => {
                RegistryError::UnsupportedPathEncoding(encoding)
            },
            _ => RegistryError::InvalidReference,
        })?;
        if entry.token != token {
            return Err(RegistryError::InvalidReference);
        }
        let path_bytes = BASE64_STANDARD
            .decode(entry.path_base64)
            .map_err(|_| RegistryError::InvalidReference)?;
        let decoded = decode_path(&entry.path_encoding, path_bytes)?;
        let mut source = File::open(&decoded).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                RegistryError::IdentityChanged(token.to_string())
            } else {
                RegistryError::Io(error)
            }
        })?;
        let metadata = source.metadata()?;
        enforce_artifact_limit(metadata.len())?;
        let identity = file_identity(&metadata, hash_file(&mut source)?);
        if !metadata.is_file() || identity != entry.identity {
            return Err(RegistryError::IdentityChanged(token.to_string()));
        }
        Ok(decoded)
    }

    /// Resolve the most recently registered artifact.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] when the pointer or referenced entry is invalid.
    pub fn resolve_latest(&self) -> Result<PathBuf, RegistryError> {
        let latest = fs::read_to_string(self.root.join("latest"))?;
        self.resolve(latest.trim())
    }

    fn write_latest(&self, token: &str) -> Result<(), RegistryError> {
        let mut temporary = NamedTempFile::new_in(&self.root)?;
        temporary.write_all(format!("@{token}\n").as_bytes())?;
        temporary.flush()?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(self.root.join("latest"))
            .map_err(|error| RegistryError::Io(error.error))?;
        Ok(())
    }
}

fn unix_time_ms() -> Result<u64, RegistryError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RegistryError::ClockBeforeEpoch)?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn modified_unix_nanos(metadata: &fs::Metadata) -> Option<u128> {
    metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|value| value.as_nanos())
}

fn file_identity(metadata: &fs::Metadata, content_sha256_base64: String) -> FileIdentity {
    #[cfg(unix)]
    {
        FileIdentity {
            size: metadata.len(),
            modified_unix_nanos: modified_unix_nanos(metadata),
            device: Some(metadata.dev()),
            inode: Some(metadata.ino()),
            content_sha256_base64,
        }
    }
    #[cfg(not(unix))]
    {
        FileIdentity {
            size: metadata.len(),
            modified_unix_nanos: modified_unix_nanos(metadata),
            device: None,
            inode: None,
            content_sha256_base64,
        }
    }
}

fn hash_file(file: &mut File) -> Result<String, RegistryError> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(buffer.get(..read).unwrap_or_default());
    }
    Ok(BASE64_STANDARD.encode(digest.finalize()))
}

fn enforce_artifact_limit(actual: u64) -> Result<(), RegistryError> {
    let limit = u64::try_from(MAX_ARTIFACT_INPUT_BYTES_V1).unwrap_or(u64::MAX);
    if actual > limit {
        return Err(RegistryError::ArtifactLimit { actual, limit: MAX_ARTIFACT_INPUT_BYTES_V1 });
    }
    Ok(())
}

fn is_windows_absolute(value: &str) -> bool {
    match value.as_bytes() {
        [drive, b':', separator, ..]
            if drive.is_ascii_alphabetic() && matches!(separator, b'/' | b'\\') =>
        {
            true
        },
        [first, second, ..] if matches!(first, b'/' | b'\\') && matches!(second, b'/' | b'\\') => {
            true
        },
        _ => false,
    }
}

#[cfg(unix)]
fn encode_path(path: &Path) -> (&'static str, Vec<u8>) {
    ("unix-bytes-v1", path.as_os_str().as_bytes().to_vec())
}

#[cfg(not(unix))]
fn encode_path(path: &Path) -> (&'static str, Vec<u8>) {
    ("utf8-v1", path.to_string_lossy().into_owned().into_bytes())
}

#[cfg(unix)]
fn decode_path(encoding: &str, bytes: Vec<u8>) -> Result<PathBuf, RegistryError> {
    if encoding != "unix-bytes-v1" {
        return Err(RegistryError::UnsupportedPathEncoding(encoding.to_string()));
    }
    Ok(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(not(unix))]
fn decode_path(encoding: &str, bytes: Vec<u8>) -> Result<PathBuf, RegistryError> {
    if encoding != "utf8-v1" {
        return Err(RegistryError::UnsupportedPathEncoding(encoding.to_string()));
    }
    let value = String::from_utf8(bytes).map_err(|_| RegistryError::InvalidReference)?;
    Ok(PathBuf::from(value))
}

fn random_token() -> Result<String, RegistryError> {
    let mut random = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut random).map_err(|_| RegistryError::Random)?;
    let mut accumulator = 0u128;
    for byte in random {
        accumulator = (accumulator << 8) | u128::from(byte);
    }
    let mut token = [b'0'; TOKEN_LENGTH];
    for character in token.iter_mut().rev() {
        let index = usize::try_from(accumulator & 31).unwrap_or_default();
        *character = TOKEN_ALPHABET.get(index).copied().ok_or(RegistryError::Random)?;
        accumulator >>= 5;
    }
    Ok(token.into_iter().map(char::from).collect())
}

/// Extract and validate a token from `@TOKEN`, a bare token, or the typed URI.
///
/// # Errors
///
/// Returns [`RegistryError::InvalidReference`] for non-canonical input.
pub fn parse_token(reference: &str) -> Result<&str, RegistryError> {
    let token = reference
        .strip_prefix(ARTIFACT_REF_URI_PREFIX)
        .or_else(|| reference.strip_prefix('@'))
        .unwrap_or(reference);
    if token.len() != TOKEN_LENGTH
        || !token.as_bytes().iter().all(|byte| TOKEN_ALPHABET.contains(byte))
    {
        return Err(RegistryError::InvalidReference);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_resolve_and_latest_preserve_file_identity() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let artifact = directory.path().join("artifact.txt");
        fs::write(&artifact, "hello").expect("write artifact");
        let registry = Registry::new(directory.path().join("state"));
        let entry = registry.register(&artifact).expect("register");
        entry.validate().expect("registered entry should be structurally valid");
        assert_eq!(entry.short_ref().len(), TOKEN_LENGTH + 1);
        assert_eq!(registry.resolve(&entry.short_ref()).expect("resolve"), artifact);
        assert_eq!(registry.resolve(&entry.uri()).expect("resolve URI"), artifact);
        assert_eq!(registry.resolve_latest().expect("resolve latest"), artifact);
    }

    #[test]
    fn portable_validation_rejects_non_sha256_digest() {
        let entry = ArtifactRefV1 {
            schema: ARTIFACT_REF_SCHEMA_V1.to_owned(),
            token: "0123456789ABC".to_owned(),
            path_encoding: "unix-bytes-v1".to_owned(),
            path_base64: BASE64_STANDARD.encode(b"/tmp/example.txt"),
            identity: FileIdentity {
                size: 0,
                modified_unix_nanos: None,
                device: None,
                inode: None,
                content_sha256_base64: BASE64_STANDARD.encode([0_u8; 31]),
            },
            registered_at_unix_ms: 0,
        };

        assert_eq!(entry.validate(), Err(ArtifactRefValidationError::InvalidDigest));
    }

    #[test]
    fn modified_artifact_does_not_silently_rebind() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let artifact = directory.path().join("artifact.txt");
        fs::write(&artifact, "first").expect("write artifact");
        let registry = Registry::new(directory.path().join("state"));
        let entry = registry.register(&artifact).expect("register");
        fs::write(&artifact, "different size").expect("modify artifact");
        assert!(matches!(
            registry.resolve(&entry.short_ref()),
            Err(RegistryError::IdentityChanged(_))
        ));
    }

    #[test]
    fn rejects_paths_and_ambiguous_tokens_as_references() {
        assert!(parse_token("/tmp/file").is_err());
        assert!(parse_token("@OOOOOOOOOOOOO").is_err());
        assert!(parse_token("@123").is_err());
    }

    #[test]
    fn rejects_oversized_artifact_before_hashing_or_persisting() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let artifact = directory.path().join("oversized.bin");
        let file = File::create(&artifact).expect("create sparse artifact");
        file.set_len(
            u64::try_from(MAX_ARTIFACT_INPUT_BYTES_V1)
                .expect("profile limit fits u64")
                .saturating_add(1),
        )
        .expect("size sparse artifact");
        let registry = Registry::new(directory.path().join("state"));

        assert!(matches!(registry.register(&artifact), Err(RegistryError::ArtifactLimit { .. })));
        assert!(!registry.root().join("latest").exists());
    }
}
