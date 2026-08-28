use gtp_types::{ConnectionId, PacketNumber, Result};

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
    fn open(
        &self,
        packet_number: PacketNumber,
        connection_id: ConnectionId,
        aad: &[u8],
        payload: &mut [u8],
    ) -> Result<usize>;

    /// Authentication tag overhead in bytes.
    fn tag_len(&self) -> usize;
}
