use crate::packet_io::{PacketIo, RecvDatagram};
use gtp_types::{Result, TransportError};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};

pub const DEFAULT_SOCKET_BUFFER_SIZE: usize = 2 * 1024 * 1024; // 2 MB OS buffers

/// High-performance UDP socket wrapper with socket2 configuration.
pub struct UdpSocketIo {
    socket: UdpSocket,
}

impl UdpSocketIo {
    pub fn bind(addr: SocketAddr) -> Result<Self> {
        let domain = match addr {
            SocketAddr::V4(_) => Domain::IPV4,
            SocketAddr::V6(_) => Domain::IPV6,
        };

        let sock = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| TransportError::Io(e.to_string()))?;

        sock.set_nonblocking(true)
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let _ = sock.set_recv_buffer_size(DEFAULT_SOCKET_BUFFER_SIZE);
        let _ = sock.set_send_buffer_size(DEFAULT_SOCKET_BUFFER_SIZE);

        let sock_addr = socket2::SockAddr::from(addr);
        sock.bind(&sock_addr)
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let std_socket: UdpSocket = sock.into();
        Ok(Self { socket: std_socket })
    }

    pub fn from_std(socket: UdpSocket) -> Result<Self> {
        socket
            .set_nonblocking(true)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(Self { socket })
    }
}

impl PacketIo for UdpSocketIo {
    fn send_batch(&self, packets: &[(&[u8], SocketAddr)]) -> Result<usize> {
        let mut sent_count = 0;
        for &(data, target) in packets {
            match self.socket.send_to(data, target) {
                Ok(_) => sent_count += 1,
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        Ok(sent_count)
    }

    fn recv_batch(&self, out: &mut [RecvDatagram]) -> Result<usize> {
        let mut count = 0;
        for item in out.iter_mut() {
            match self.socket.recv_from(&mut item.buf) {
                Ok((bytes, src)) => {
                    item.len = bytes;
                    item.src_addr = src;
                    item.ecn = 0;
                    count += 1;
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => return Err(TransportError::Io(e.to_string())),
            }
        }
        Ok(count)
    }

    fn local_addr(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| TransportError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_udp_socket_io_loopback_batch() {
        let server = UdpSocketIo::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.local_addr().unwrap();

        let client = UdpSocketIo::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let _client_addr = client.local_addr().unwrap();

        let msg1 = b"hello_gtp_1";
        let msg2 = b"hello_gtp_2";

        let sent = client
            .send_batch(&[(msg1, server_addr), (msg2, server_addr)])
            .unwrap();
        assert_eq!(sent, 2);

        // Sleep briefly to let OS socket receive datagrams
        std::thread::sleep(std::time::Duration::from_millis(10));

        let mut recv_batch = [RecvDatagram::default(), RecvDatagram::default()];
        let received = server.recv_batch(&mut recv_batch).unwrap();
        assert_eq!(received, 2);
        assert_eq!(&recv_batch[0].buf[..recv_batch[0].len], msg1);
        assert_eq!(&recv_batch[1].buf[..recv_batch[1].len], msg2);
    }
}
