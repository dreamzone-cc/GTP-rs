use crate::aead::GtpAeadProtector;
use gtp_types::{ConnectionId, PacketNumber, Result};

#[cfg(any(test, feature = "insecure-plaintext"))]
use crate::plaintext::PlaintextProtector;

/// Pluggable cryptographic boundary for packet payload sealing and opening.
pub trait PacketProtector: Send + Sync {
    /// Encrypts in-place and appends authentication tag.
    fn seal(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        payload_len: usize,
    ) -> Result<usize>;

    /// Authenticates and decrypts payload in-place.
    ///
    /// # Postcondition (hard contract, R-8)
    /// On error, `payload` MUST be left byte-identical: the receive path's
    /// key-phase fallback retries `open` on the SAME buffer after a failure.
    /// Implementations must verify the authentication tag before applying
    /// any keystream to the buffer.
    fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize>;

    /// Authentication tag overhead in bytes.
    fn tag_len(&self) -> usize;
}

/// Static-dispatch packet protector eliminating heap allocations and vtables on hot path.
#[derive(Clone, Debug)]
pub enum Protector {
    Aead(GtpAeadProtector),
    /// Null cipher — accepts any tampering. Only compiled for tests or when
    /// the `insecure-plaintext` cargo feature is explicitly enabled, so a
    /// mis-wired config can never silently disable confidentiality and
    /// integrity in a production build.
    #[cfg(any(test, feature = "insecure-plaintext"))]
    Plaintext(PlaintextProtector),
}

impl Protector {
    #[inline(always)]
    pub fn seal(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        payload_len: usize,
    ) -> Result<usize> {
        match self {
            Protector::Aead(p) => p.seal(packet_number, connection_id, aad, payload, payload_len),
            #[cfg(any(test, feature = "insecure-plaintext"))]
            Protector::Plaintext(p) => {
                p.seal(packet_number, connection_id, aad, payload, payload_len)
            }
        }
    }

    #[inline(always)]
    pub fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize> {
        match self {
            Protector::Aead(p) => {
                p.open(packet_number, connection_id, aad, payload, ciphertext_len)
            }
            #[cfg(any(test, feature = "insecure-plaintext"))]
            Protector::Plaintext(p) => {
                p.open(packet_number, connection_id, aad, payload, ciphertext_len)
            }
        }
    }

    #[inline(always)]
    pub fn tag_len(&self) -> usize {
        match self {
            Protector::Aead(p) => p.tag_len(),
            #[cfg(any(test, feature = "insecure-plaintext"))]
            Protector::Plaintext(p) => p.tag_len(),
        }
    }
}

impl PacketProtector for Protector {
    #[inline(always)]
    fn seal(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        payload_len: usize,
    ) -> Result<usize> {
        self.seal(packet_number, connection_id, aad, payload, payload_len)
    }

    #[inline(always)]
    fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize> {
        self.open(packet_number, connection_id, aad, payload, ciphertext_len)
    }

    #[inline(always)]
    fn tag_len(&self) -> usize {
        self.tag_len()
    }
}
