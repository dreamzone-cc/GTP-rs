use crate::packet_io::{PacketIo, RecvDatagram};
use gtp_types::{Result, TransportError};
use socket2::{Domain, Protocol, Socket, Type};
use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};

pub const DEFAULT_SOCKET_BUFFER_SIZE: usize = 2 * 1024 * 1024; // 2 MB OS buffers

/// ECN codepoint for ECT(0) (RFC 3168): marks outgoing traffic as
/// ECN-Capable so congested routers can CE-mark it instead of dropping it.
#[cfg(unix)]
pub const ECN_ECT0: u8 = 0b10;

/// Extract the 2-bit ECN codepoint from an IP TOS/Traffic-Class byte.
/// 00 = Not-ECT, 01 = ECT(1), 10 = ECT(0), 11 = CE (congestion experienced).
pub fn ecn_from_tos(tos: u8) -> u8 {
    tos & 0x03
}

/// High-performance UDP socket wrapper with socket2 configuration.
pub struct UdpSocketIo {
    socket: UdpSocket,
    /// Whether ancillary (cmsg) reception was enabled — only then does
    /// `recv_batch` pay the recvmsg cost.
    ecn_enabled: bool,
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
        Ok(Self {
            socket: std_socket,
            ecn_enabled: false,
        })
    }

    pub fn from_std(socket: UdpSocket) -> Result<Self> {
        socket
            .set_nonblocking(true)
            .map_err(|e| TransportError::Io(e.to_string()))?;
        Ok(Self {
            socket,
            ecn_enabled: false,
        })
    }

    /// Opts the socket into real ECN feedback (QUIC-style, app-level report):
    ///
    /// - outgoing datagrams are marked ECT(0) at the IP layer, so congested
    ///   routers may CE-mark them instead of dropping them;
    /// - incoming datagrams are received with ancillary data carrying the
    ///   IP TOS / Traffic Class, and [`RecvDatagram::ecn`] reports the 2-bit
    ///   ECN codepoint the network actually delivered (feed it into the
    ///   connection's ACK tracker; the peer reacts via its congestion
    ///   controller's `on_ecn`).
    ///
    /// The protocol header's own `ecn_bits` are inside the authenticated AAD
    /// and therefore cannot be marked en route — the IP layer is the only
    /// honest source of network ECN signal.
    pub fn enable_ecn(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            set_ecn_sockopts(self.socket.as_raw_fd())?;
            self.ecn_enabled = true;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(TransportError::Io(
                "ECN ancillary reception is not implemented on this platform".into(),
            ))
        }
    }
}

/// Transient receive conditions that simply end the current batch, mirroring
/// the runtime layer's P2-1 policy: an ICMP port-unreachable surfacing as
/// ConnectionReset on a UDP socket must not abort the whole batch.
fn is_transient_recv(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionRefused
            | ErrorKind::Interrupted
    )
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
            #[cfg(unix)]
            let received = if self.ecn_enabled {
                recv_one_with_ecn(&self.socket, &mut item.buf)
            } else {
                self.socket
                    .recv_from(&mut item.buf)
                    .map(|(bytes, src)| (bytes, src, 0u8))
            };
            #[cfg(not(unix))]
            let received = self
                .socket
                .recv_from(&mut item.buf)
                .map(|(bytes, src)| (bytes, src, 0u8));

            match received {
                Ok((bytes, src, ecn)) => {
                    item.len = bytes;
                    item.src_addr = src;
                    item.ecn = ecn;
                    count += 1;
                }
                Err(ref e) if is_transient_recv(e.kind()) => break,
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

// ---------------------------------------------------------------------------
// Unix ECN plumbing (recvmsg + cmsg). Safety rests on three invariants,
// each local to `recv_one_with_ecn`: the iovec points at the caller's buffer
// for the duration of the single recvmsg call; the control buffer is a
// u64-aligned array (cmsg alignment requirement); and cmsghdr iteration uses
// libc's own CMSG_FIRSTHDR/CMSG_NXTHDR/CMSG_DATA with lengths bounded by
// msg_controllen as filled by the kernel.
// ---------------------------------------------------------------------------
#[cfg(unix)]
use std::os::fd::AsRawFd;

#[cfg(unix)]
fn set_ecn_sockopts(fd: std::os::unix::io::RawFd) -> Result<()> {
    // SAFETY: plain setsockopt(2) with int-sized values on a valid fd.
    unsafe {
        let ect0: libc::c_int = ECN_ECT0 as libc::c_int;
        let v4 = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &ect0 as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let _v4 = v4 == 0;
        let recv_tos: libc::c_int = 1;
        let r4 = libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_RECVTOS,
            &recv_tos as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let _r4 = r4 == 0;
        let tclass: libc::c_int = (ECN_ECT0 as libc::c_int) << 20; // ECN sits in the TClass LSBs
        let v6 = libc::setsockopt(
            fd,
            libc::IPPROTO_IPV6,
            libc::IPV6_TCLASS,
            &tclass as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let _v6 = v6 == 0;
        let recv_tclass: libc::c_int = 1;
        let r6 = libc::setsockopt(
            fd,
            libc::IPPROTO_IPV6,
            libc::IPV6_RECVTCLASS,
            &recv_tclass as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
        let _r6 = r6 == 0;
    }
    // Failing to enable is best-effort (e.g. v6 opts on a v4 socket): the
    // receive path then simply reports Not-ECT instead of erroring.
    Ok(())
}

/// One recvmsg carrying the source address and the delivered ECN codepoint.
#[cfg(unix)]
fn recv_one_with_ecn(
    socket: &UdpSocket,
    buf: &mut [u8],
) -> std::io::Result<(usize, SocketAddr, u8)> {
    // SAFETY: see the module-level comment — iovec borrows `buf` for this
    // call only; `control` is a u64-aligned ancillary buffer; `name` is a
    // sockaddr_storage initialized to zero as required by recvmsg.
    unsafe {
        let mut name: libc::sockaddr_storage = std::mem::zeroed();
        let mut hdr: libc::msghdr = std::mem::zeroed();
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        // 64 bytes of kernel-written ancillary data, u64-aligned.
        let mut control = [0u64; 8];

        hdr.msg_name = (&mut name as *mut libc::sockaddr_storage).cast();
        hdr.msg_namelen = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        hdr.msg_iov = &mut iov;
        hdr.msg_iovlen = 1;
        hdr.msg_control = control.as_mut_ptr().cast();
        hdr.msg_controllen = control.len() * std::mem::size_of::<u64>();

        let n = libc::recvmsg(socket.as_raw_fd(), &mut hdr, 0);
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let ((), addr) = socket2::SockAddr::try_init(|storage, len| {
            let src_len = hdr.msg_namelen.min(*len);
            std::ptr::copy_nonoverlapping(
                (&name as *const libc::sockaddr_storage) as *const u8,
                storage as *mut u8,
                src_len as usize,
            );
            *len = src_len;
            Ok(())
        })?;
        let addr = addr.as_socket().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown family")
        })?;

        let mut ecn = 0u8;
        let mut cmsg = libc::CMSG_FIRSTHDR(&hdr);
        while !cmsg.is_null() {
            let level = (*cmsg).cmsg_level;
            let ctype = (*cmsg).cmsg_type;
            let is_tos = (level == libc::IPPROTO_IP && ctype == libc::IP_TOS)
                || (level == libc::IPPROTO_IPV6 && ctype == libc::IPV6_TCLASS);
            if is_tos {
                // cmsg_len is CMSG_LEN(payload) = header + payload; the
                // TOS/TClass payload is at least one byte.
                let data_len = ((*cmsg).cmsg_len as usize)
                    .saturating_sub(std::mem::size_of::<libc::cmsghdr>());
                if data_len >= 1 {
                    let byte = *(libc::CMSG_DATA(cmsg) as *const u8);
                    ecn = ecn_from_tos(byte);
                }
            }
            cmsg = libc::CMSG_NXTHDR(&hdr, cmsg);
        }

        Ok((n as usize, addr, ecn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_recv_kinds_end_the_batch_instead_of_erroring() {
        for kind in [
            ErrorKind::WouldBlock,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionRefused,
            ErrorKind::Interrupted,
        ] {
            assert!(is_transient_recv(kind), "{kind:?} must be transient");
        }
        assert!(!is_transient_recv(ErrorKind::AddrInUse));
        assert!(!is_transient_recv(ErrorKind::UnexpectedEof));
    }

    #[test]
    fn ecn_codepoint_mapping() {
        assert_eq!(ecn_from_tos(0x00), 0b00, "Not-ECT");
        assert_eq!(ecn_from_tos(0x01), 0b01, "ECT(1)");
        assert_eq!(ecn_from_tos(0x02), 0b10, "ECT(0)");
        assert_eq!(ecn_from_tos(0x03), 0b11, "CE");
        // DSCP bits must not leak into the codepoint.
        assert_eq!(ecn_from_tos(0b1010_10), 0b10);
        assert_eq!(ecn_from_tos(0b1100_11), 0b11);
    }

    /// End-to-end on loopback: a sender marked ECT(0) at the IP layer is
    /// observed as codepoint 0b10 by an ECN-enabled receiver.
    #[cfg(unix)]
    #[test]
    fn loopback_ecn_is_observed_by_the_receiver() {
        let mut server = UdpSocketIo::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let server_addr = server.local_addr().unwrap();
        let mut client = UdpSocketIo::bind("127.0.0.1:0".parse().unwrap()).unwrap();

        server.enable_ecn().unwrap();
        client.enable_ecn().unwrap(); // also marks the client's sends ECT(0)

        client
            .send_batch(&[(&b"ecn-probe"[..], server_addr)])
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));

        let mut batch = [RecvDatagram::default()];
        let n = server.recv_batch(&mut batch).unwrap();
        assert_eq!(n, 1);
        assert_eq!(&batch[0].buf[..batch[0].len], b"ecn-probe");
        assert_eq!(
            batch[0].ecn, 0b10,
            "loopback must preserve the ECT(0) marking to the receiver"
        );
    }

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
        assert_eq!(recv_batch[0].ecn, 0, "ECN disabled: codepoint reads 0");
    }
}
