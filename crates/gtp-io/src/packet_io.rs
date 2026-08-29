use gtp_types::Result;
use std::net::SocketAddr;

/// Datagram payload buffer for batch reception.
pub const MAX_DATAGRAM_SIZE: usize = 2048;

#[derive(Clone, Debug)]
pub struct RecvDatagram {
    pub buf: [u8; MAX_DATAGRAM_SIZE],
    pub len: usize,
    pub src_addr: SocketAddr,
    pub ecn: u8,
}

impl Default for RecvDatagram {
    fn default() -> Self {
        Self {
            buf: [0u8; MAX_DATAGRAM_SIZE],
            len: 0,
            src_addr: "0.0.0.0:0".parse().unwrap(),
            ecn: 0,
        }
    }
}

/// Abstract I/O boundary for sending and receiving datagram batches.
pub trait PacketIo: Send + Sync {
    /// Sends a batch of datagrams. Returns count of successfully submitted packets.
    fn send_batch(&self, packets: &[(&[u8], SocketAddr)]) -> Result<usize>;

    /// Receives a batch of available datagrams into destination buffers.
    fn recv_batch(&self, out: &mut [RecvDatagram]) -> Result<usize>;

    /// Local bound socket address.
    fn local_addr(&self) -> Result<SocketAddr>;
}
