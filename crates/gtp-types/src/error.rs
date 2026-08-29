use core::fmt;

/// Structured transport errors.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum TransportError {
    InvalidPacket(&'static str),
    BufferTooShort,
    BufferOverflow,
    AuthenticationFailed,
    CryptoFailure,
    TruncatedFrame { needed: usize, available: usize },
    MalformedFrame(&'static str),
    ReplayDetected,
    ProtocolViolation(&'static str),
    ResourceLimitExceeded(&'static str),
    PathValidationFailed,
    Timeout,
    HandshakeTimeout,
    HandshakeFailed(&'static str),
    ConnectionClosed(u16),
    MessageExpired,
    Io(String),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPacket(reason) => write!(f, "Invalid packet: {}", reason),
            Self::BufferTooShort => write!(f, "Buffer too short"),
            Self::BufferOverflow => write!(f, "Buffer overflow"),
            Self::AuthenticationFailed => write!(f, "AEAD authentication failed"),
            Self::CryptoFailure => write!(f, "Cryptographic operation failed"),
            Self::TruncatedFrame { needed, available } => {
                write!(
                    f,
                    "Truncated frame: needed {} bytes, available {}",
                    needed, available
                )
            }
            Self::MalformedFrame(reason) => write!(f, "Malformed frame: {}", reason),
            Self::ReplayDetected => write!(f, "Replayed packet rejected"),
            Self::ProtocolViolation(reason) => write!(f, "Protocol violation: {}", reason),
            Self::ResourceLimitExceeded(reason) => write!(f, "Resource limit exceeded: {}", reason),
            Self::PathValidationFailed => write!(f, "Path validation failed"),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::HandshakeTimeout => write!(f, "X25519 cryptographic handshake timed out"),
            Self::HandshakeFailed(reason) => write!(f, "Handshake failed: {}", reason),
            Self::ConnectionClosed(code) => write!(f, "Connection closed with code 0x{:04x}", code),
            Self::MessageExpired => write!(f, "Message deadline expired"),
            Self::Io(err) => write!(f, "I/O error: {}", err),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for TransportError {}

pub type Result<T> = core::result::Result<T, TransportError>;
