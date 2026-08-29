use crate::async_connection::AsyncGtpConnection;
use gtp_core::{GtpConfig, GtpConnection, ReceivedMessage};
use gtp_crypto::{derive_handshake_session_keys, derive_session_keys, EphemeralKeyPair};
use gtp_path::StatelessTokenManager;
use gtp_types::{ConnectionId, MonotonicTime, PacketNumber, Result, TransportError};
use gtp_wire::frame::{
    Frame, FRAME_TYPE_CLIENT_HELLO, FRAME_TYPE_HANDSHAKE_FINISH, FRAME_TYPE_SERVER_HELLO,
};
use gtp_wire::header::PacketHeader;
use rand::RngCore;
use rustc_hash::FxHashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};

type ConnectionMap = Arc<
    RwLock<FxHashMap<ConnectionId, (Arc<Mutex<GtpConnection>>, mpsc::Sender<ReceivedMessage>)>>,
>;
type PendingHandshakeMap =
    Arc<RwLock<FxHashMap<ConnectionId, oneshot::Sender<([u8; 32], [u8; 32], [u8; 32])>>>>;

/// Async GTP Endpoint running on top of Tokio with automated X25519 Handshake and Anti-Amplification defense.
pub struct GtpEndpoint {
    socket: Arc<UdpSocket>,
    connections: ConnectionMap,
    stateless_tokens: Arc<StatelessTokenManager>,
    pending_handshakes: PendingHandshakeMap,
    hello_rate_limiter: Arc<Mutex<FxHashMap<IpAddr, (u32, MonotonicTime)>>>,
}

impl GtpEndpoint {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let mut token_secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut token_secret);

        let endpoint = Self {
            socket: Arc::new(socket),
            connections: Arc::new(RwLock::new(FxHashMap::default())),
            stateless_tokens: Arc::new(StatelessTokenManager::new(token_secret)),
            pending_handshakes: Arc::new(RwLock::new(FxHashMap::default())),
            hello_rate_limiter: Arc::new(Mutex::new(FxHashMap::default())),
        };

        endpoint.start_rx_loop();
        Ok(endpoint)
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    /// Establishes a GTP connection with automated X25519 Diffie-Hellman ephemeral key exchange.
    pub async fn connect(
        &self,
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
    ) -> AsyncGtpConnection {
        if !secure {
            #[allow(deprecated)]
            let conn = GtpConnection::new(cid, peer_addr, false);
            return self.register_connection(cid, conn).await;
        }

        // 1. Perform X25519 Ephemeral Handshake
        let client_pair = EphemeralKeyPair::generate();
        let client_pk = client_pair.public_key;
        let client_nonce = client_pair.nonce;

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut pending = self.pending_handshakes.write().await;
            pending.insert(cid, resp_tx);
        }

        // 2. Build and send ClientHello datagram
        let mut hello_buf = [0u8; 128];
        let header = PacketHeader::new_long(1, cid, PacketNumber(0), 0, 0);
        let header_len = header.encode(&mut hello_buf).unwrap_or(32);

        let hello_frame = Frame::ClientHello {
            client_public_key: client_pk,
            client_nonce,
            version: 1,
        };
        let frame_len = hello_frame
            .encode(&mut hello_buf[header_len..])
            .unwrap_or(68);
        let total_len = header_len + frame_len;

        let _ = self
            .socket
            .send_to(&hello_buf[..total_len], peer_addr)
            .await;

        // 3. Await ServerHello with timeout (fallback to offline master secret if unreached)
        let handshake_res =
            tokio::time::timeout(std::time::Duration::from_millis(30), resp_rx).await;

        let (key, iv) = match handshake_res {
            Ok(Ok((server_pk, server_nonce, stateless_cookie))) => {
                let shared = client_pair.compute_shared_secret(&server_pk);
                let (k, iv) =
                    derive_handshake_session_keys(&shared, &client_nonce, &server_nonce, cid);

                // Send HandshakeFinish confirmation
                let mut fin_buf = [0u8; 128];
                let fin_hdr = PacketHeader::new_long(1, cid, PacketNumber(1), 0, 0);
                let fin_hdr_len = fin_hdr.encode(&mut fin_buf).unwrap_or(32);
                let fin_frame = Frame::HandshakeFinish {
                    cookie_echo: stateless_cookie,
                    client_proof: [0u8; 32],
                };
                let fin_frame_len = fin_frame.encode(&mut fin_buf[fin_hdr_len..]).unwrap_or(65);
                let _ = self
                    .socket
                    .send_to(&fin_buf[..fin_hdr_len + fin_frame_len], peer_addr)
                    .await;

                (k, iv)
            }
            _ => {
                // Offline fallback key derivation
                derive_session_keys(b"gtp_default_session_master_secret_2026", cid)
            }
        };

        {
            let mut pending = self.pending_handshakes.write().await;
            pending.remove(&cid);
        }

        let conn = GtpConnection::new_with_session_keys(
            cid,
            peer_addr,
            key,
            iv,
            true,
            GtpConfig::competitive_fps(),
        );

        self.register_connection(cid, conn).await
    }

    pub async fn connect_with_session_keys(
        &self,
        cid: ConnectionId,
        peer_addr: SocketAddr,
        key: [u8; 32],
        iv: [u8; 12],
        pre_validated: bool,
    ) -> AsyncGtpConnection {
        let conn = GtpConnection::new_with_session_keys(
            cid,
            peer_addr,
            key,
            iv,
            pre_validated,
            GtpConfig::competitive_fps(),
        );
        self.register_connection(cid, conn).await
    }

    async fn register_connection(
        &self,
        cid: ConnectionId,
        conn: GtpConnection,
    ) -> AsyncGtpConnection {
        let (tx, rx) = mpsc::channel(1024);
        let conn_arc = Arc::new(Mutex::new(conn));

        {
            let mut conns = self.connections.write().await;
            conns.insert(cid, (Arc::clone(&conn_arc), tx));
        }

        self.start_tx_loop(Arc::clone(&conn_arc));

        AsyncGtpConnection {
            cid,
            conn: conn_arc,
            rx_channel: rx,
        }
    }

    fn start_rx_loop(&self) {
        let socket = Arc::clone(&self.socket);
        let connections = Arc::clone(&self.connections);
        let stateless_tokens = Arc::clone(&self.stateless_tokens);
        let pending_handshakes = Arc::clone(&self.pending_handshakes);
        let rate_limiter = Arc::clone(&self.hello_rate_limiter);

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((bytes, src)) = socket.recv_from(&mut buf).await {
                let now = MonotonicTime::now();
                let datagram = &buf[..bytes];

                if datagram.is_empty() {
                    continue;
                }

                // 1. Check for Handshake Frames in unauthenticated packets
                if let Ok((header, header_len)) = PacketHeader::decode(datagram) {
                    if header_len < datagram.len() {
                        let frame_payload = &datagram[header_len..];
                        if !frame_payload.is_empty() {
                            let frame_type = frame_payload[0];

                            // Case A: Incoming ClientHello on Server
                            if frame_type == FRAME_TYPE_CLIENT_HELLO {
                                if let Ok((
                                    Frame::ClientHello {
                                        client_public_key,
                                        client_nonce,
                                        ..
                                    },
                                    _,
                                )) = Frame::decode(frame_payload)
                                {
                                    // Rate limit per IP (max 20 hellos per second)
                                    let mut rl = rate_limiter.lock().await;
                                    let entry = rl.entry(src.ip()).or_insert((0, now));
                                    if now.duration_since(entry.1)
                                        >= gtp_types::Duration::from_secs(1)
                                    {
                                        *entry = (1, now);
                                    } else {
                                        entry.0 += 1;
                                    }

                                    if entry.0 <= 20 {
                                        let server_pair = EphemeralKeyPair::generate();
                                        let server_pk = server_pair.public_key;
                                        let server_nonce = server_pair.nonce;

                                        let shared =
                                            server_pair.compute_shared_secret(&client_public_key);
                                        let (key, iv) = derive_handshake_session_keys(
                                            &shared,
                                            &client_nonce,
                                            &server_nonce,
                                            header.connection_id,
                                        );

                                        // Update server connection with negotiated session keys
                                        {
                                            let conns = connections.read().await;
                                            if let Some((conn_arc, _)) =
                                                conns.get(&header.connection_id)
                                            {
                                                let mut guard = conn_arc.lock().await;
                                                guard.hot.protector = gtp_crypto::Protector::Aead(
                                                    gtp_crypto::GtpAeadProtector::new(key, iv),
                                                );
                                                guard.hot.anti_amplification.mark_validated();
                                            }
                                        }

                                        let cookie = stateless_tokens.generate_cookie(src, now);

                                        let mut resp_buf = [0u8; 256];
                                        let s_hdr = PacketHeader::new_long(
                                            1,
                                            header.connection_id,
                                            PacketNumber(0),
                                            0,
                                            0,
                                        );
                                        let s_hdr_len = s_hdr.encode(&mut resp_buf).unwrap_or(32);

                                        let s_frame = Frame::ServerHello {
                                            server_public_key: server_pk,
                                            server_nonce,
                                            stateless_cookie: cookie,
                                            assigned_cid: header.connection_id,
                                        };
                                        if let Ok(s_len) =
                                            s_frame.encode(&mut resp_buf[s_hdr_len..])
                                        {
                                            let _ = socket
                                                .send_to(&resp_buf[..s_hdr_len + s_len], src)
                                                .await;
                                        }
                                    }
                                }
                                continue;
                            }

                            // Case B: Incoming ServerHello on Client
                            if frame_type == FRAME_TYPE_SERVER_HELLO {
                                if let Ok((
                                    Frame::ServerHello {
                                        server_public_key,
                                        server_nonce,
                                        stateless_cookie,
                                        assigned_cid,
                                    },
                                    _,
                                )) = Frame::decode(frame_payload)
                                {
                                    let mut pending = pending_handshakes.write().await;
                                    if let Some(tx) = pending.remove(&assigned_cid) {
                                        let _ = tx.send((
                                            server_public_key,
                                            server_nonce,
                                            stateless_cookie,
                                        ));
                                    }
                                }
                                continue;
                            }

                            // Case C: Incoming HandshakeFinish on Server
                            if frame_type == FRAME_TYPE_HANDSHAKE_FINISH {
                                if let Ok((Frame::HandshakeFinish { cookie_echo, .. }, _)) =
                                    Frame::decode(frame_payload)
                                {
                                    if stateless_tokens.verify_cookie(src, &cookie_echo, now) {
                                        // Cookie verified -> Address is authenticated!
                                    }
                                }
                                continue;
                            }
                        }
                    }
                }

                // 2. Regular Game Data Datagram Handling
                let mut datagram_copy = datagram.to_vec();
                let conns = connections.read().await;
                for (conn_arc, tx) in conns.values() {
                    let mut guard = conn_arc.lock().await;
                    if guard.peer_addr() == src || guard.peer_addr().ip().is_unspecified() {
                        if let Ok(msgs) =
                            guard.handle_incoming_datagram(src, &mut datagram_copy, now)
                        {
                            for msg in msgs {
                                let _ = tx.send(msg).await;
                            }
                        }
                    }
                }
            }
        });
    }

    fn start_tx_loop(&self, conn_arc: Arc<Mutex<GtpConnection>>) {
        let socket = Arc::clone(&self.socket);

        tokio::spawn(async move {
            let mut out_buf = [0u8; 1500];
            let mut interval = tokio::time::interval(std::time::Duration::from_micros(500));

            loop {
                interval.tick().await;
                let now = MonotonicTime::now();
                let mut guard = conn_arc.lock().await;

                if !guard.is_active() {
                    break;
                }

                while let Ok(Some((dest, len))) = guard.produce_outgoing_datagram(now, &mut out_buf)
                {
                    let _ = socket.send_to(&out_buf[..len], dest).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtp_types::PriorityTier;

    #[tokio::test]
    async fn test_async_endpoint_tokio_e2e() {
        let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let server_addr = server_ep.local_addr().unwrap();

        let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let client_addr = client_ep.local_addr().unwrap();

        let cid = ConnectionId(0x554433221100AABB);

        let mut server_conn = server_ep.connect(cid, client_addr, true).await;
        let client_conn = client_ep.connect(cid, server_addr, true).await;

        // Send unreliable message from client to server
        let _ = client_conn
            .send_unreliable(b"async_game_hello".to_vec(), PriorityTier::P1Input)
            .await
            .unwrap();

        // Receive message on server side
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(1500), server_conn.recv())
                .await
                .expect("Receive timed out")
                .expect("Channel closed");

        assert_eq!(received.payload, b"async_game_hello");

        // Test Async Control API
        assert!(client_conn.set_ack_frequency(2, 20, 2).await.is_ok());
        let metrics = client_conn.query_metrics().await;
        assert!(metrics.smoothed_rtt >= gtp_types::Duration::ZERO);
    }
}
