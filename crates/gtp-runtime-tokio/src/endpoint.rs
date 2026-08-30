use crate::async_connection::AsyncGtpConnection;
use gtp_core::{GtpConfig, GtpConnection, ReceivedMessage};
use gtp_crypto::{derive_directional_handshake_session_keys, EphemeralKeyPair};
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
type PendingClientHandshakeMap =
    Arc<RwLock<FxHashMap<ConnectionId, oneshot::Sender<([u8; 32], [u8; 32], [u8; 32])>>>>;
type PendingServerHandshakeMap = Arc<
    RwLock<
        FxHashMap<
            ConnectionId,
            (
                EphemeralKeyPair,
                [u8; 32],
                [u8; 32],
                SocketAddr,
                MonotonicTime,
            ),
        >,
    >,
>;

/// Async GTP Endpoint running on top of Tokio with automated X25519 Handshake and Anti-Amplification defense.
pub struct GtpEndpoint {
    socket: Arc<UdpSocket>,
    connections: ConnectionMap,
    stateless_tokens: Arc<StatelessTokenManager>,
    pending_client_handshakes: PendingClientHandshakeMap,
    pending_server_handshakes: PendingServerHandshakeMap,
    hello_rate_limiter: Arc<Mutex<FxHashMap<IpAddr, (u32, MonotonicTime)>>>,
    incoming_connections_tx: mpsc::Sender<AsyncGtpConnection>,
    incoming_connections_rx: Arc<Mutex<mpsc::Receiver<AsyncGtpConnection>>>,
}

impl GtpEndpoint {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let mut token_secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut token_secret);

        let (incoming_tx, incoming_rx) = mpsc::channel(1024);

        let endpoint = Self {
            socket: Arc::new(socket),
            connections: Arc::new(RwLock::new(FxHashMap::default())),
            stateless_tokens: Arc::new(StatelessTokenManager::new(token_secret)),
            pending_client_handshakes: Arc::new(RwLock::new(FxHashMap::default())),
            pending_server_handshakes: Arc::new(RwLock::new(FxHashMap::default())),
            hello_rate_limiter: Arc::new(Mutex::new(FxHashMap::default())),
            incoming_connections_tx: incoming_tx,
            incoming_connections_rx: Arc::new(Mutex::new(incoming_rx)),
        };

        endpoint.start_rx_loop();
        Ok(endpoint)
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    /// Accepts the next incoming client connection established via dynamic X25519 handshake.
    pub async fn accept(&self) -> Option<AsyncGtpConnection> {
        let mut rx = self.incoming_connections_rx.lock().await;
        rx.recv().await
    }

    /// Establishes a GTP connection to a remote server with automated X25519 Diffie-Hellman ephemeral key exchange.
    pub async fn connect(
        &self,
        cid: ConnectionId,
        peer_addr: SocketAddr,
        secure: bool,
    ) -> Result<AsyncGtpConnection> {
        if !secure {
            #[allow(deprecated)]
            let conn = GtpConnection::new(cid, peer_addr, false);
            return Ok(self.register_connection(cid, conn).await);
        }

        // 1. Perform X25519 Ephemeral Handshake
        let client_pair = EphemeralKeyPair::generate();
        let client_pk = client_pair.public_key;
        let client_nonce = client_pair.nonce;

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut pending = self.pending_client_handshakes.write().await;
            pending.insert(cid, resp_tx);
        }

        // 2. Build and send ClientHello datagram
        let mut hello_buf = [0u8; 128];
        let header = PacketHeader::new_long(1, cid, PacketNumber(0), 0, 0);
        let header_len = header
            .encode(&mut hello_buf)
            .map_err(|_| TransportError::BufferOverflow)?;

        let hello_frame = Frame::ClientHello {
            client_public_key: client_pk,
            client_nonce,
            version: 1,
        };
        let frame_len = hello_frame
            .encode(&mut hello_buf[header_len..])
            .map_err(|_| TransportError::BufferOverflow)?;
        let total_len = header_len + frame_len;

        self.socket
            .send_to(&hello_buf[..total_len], peer_addr)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        // 3. Await ServerHello with automatic 400ms retransmission (NO silent fallback to static secret!)
        let mut attempts = 0;
        let mut rx = resp_rx;

        let (server_pk, server_nonce, stateless_cookie) = loop {
            match tokio::time::timeout(std::time::Duration::from_millis(400), &mut rx).await {
                Ok(Ok(data)) => break data,
                Ok(Err(_)) => {
                    let mut pending = self.pending_client_handshakes.write().await;
                    pending.remove(&cid);
                    return Err(TransportError::HandshakeFailed("Internal channel dropped"));
                }
                Err(_) => {
                    attempts += 1;
                    if attempts >= 8 {
                        let mut pending = self.pending_client_handshakes.write().await;
                        pending.remove(&cid);
                        return Err(TransportError::HandshakeTimeout);
                    }
                    // Retransmit ClientHello upon packet loss or transient throttling
                    let _ = self
                        .socket
                        .send_to(&hello_buf[..total_len], peer_addr)
                        .await;
                }
            }
        };

        let shared = client_pair
            .compute_shared_secret(&server_pk)
            .map_err(|_| TransportError::HandshakeFailed("non-contributory server key"))?;
        let keys =
            derive_directional_handshake_session_keys(&shared, &client_nonce, &server_nonce, cid);

        // 4. Send HandshakeFinish confirmation with Key Confirmation Proof
        let client_proof =
            gtp_crypto::compute_client_proof(&keys.client_tx_key, &client_pk, &server_pk);
        let mut fin_buf = [0u8; 128];
        let fin_hdr = PacketHeader::new_long(1, cid, PacketNumber(1), 0, 0);
        let fin_hdr_len = fin_hdr
            .encode(&mut fin_buf)
            .map_err(|_| TransportError::BufferOverflow)?;
        let fin_frame = Frame::HandshakeFinish {
            cookie_echo: stateless_cookie,
            client_proof,
        };
        let fin_frame_len = fin_frame
            .encode(&mut fin_buf[fin_hdr_len..])
            .map_err(|_| TransportError::BufferOverflow)?;
        let _ = self
            .socket
            .send_to(&fin_buf[..fin_hdr_len + fin_frame_len], peer_addr)
            .await;

        // SEC-1: the client seals with the client->server direction.
        let conn = GtpConnection::new_with_directional_keys(
            cid,
            peer_addr,
            &keys,
            true,
            true,
            GtpConfig::competitive_fps(),
        );

        Ok(self.register_connection(cid, conn).await)
    }

    /// Registers a connection built from externally supplied directional keys.
    pub async fn connect_with_directional_keys(
        &self,
        cid: ConnectionId,
        peer_addr: SocketAddr,
        keys: &gtp_crypto::DirectionalKeys,
        as_client: bool,
        pre_validated: bool,
    ) -> AsyncGtpConnection {
        let conn = GtpConnection::new_with_directional_keys(
            cid,
            peer_addr,
            keys,
            as_client,
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

        Self::spawn_tx_loop(Arc::clone(&self.socket), Arc::clone(&conn_arc));

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
        let pending_client_handshakes = Arc::clone(&self.pending_client_handshakes);
        let pending_server_handshakes = Arc::clone(&self.pending_server_handshakes);
        let rate_limiter = Arc::clone(&self.hello_rate_limiter);
        let incoming_tx = self.incoming_connections_tx.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            // P2-1: the RX loop must survive transient socket errors. On UDP sockets,
            // an ICMP port-unreachable surfaces as ConnectionReset/ConnectionRefused —
            // a single datagram from a remote host must never kill the endpoint.
            loop {
                let recv = socket.recv_from(&mut buf).await;
                let (bytes, src) = match recv {
                    Ok(res) => res,
                    Err(e) => match e.kind() {
                        std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted => continue,
                        _ => {
                            eprintln!("gtp endpoint: RX socket error: {e}");
                            continue;
                        }
                    },
                };
                let now = MonotonicTime::now();
                let datagram = &buf[..bytes];

                if datagram.is_empty() {
                    continue;
                }

                // Decode packet header
                let (header, header_len) = match PacketHeader::decode(datagram) {
                    Ok(res) => res,
                    Err(_) => continue,
                };

                let cid = header.connection_id;

                // 1. Check for Handshake Frames in unauthenticated / handshake packets
                if header_len < datagram.len() {
                    let frame_payload = &datagram[header_len..];
                    if !frame_payload.is_empty() {
                        let frame_type = frame_payload[0];

                        // Case A: Incoming ClientHello on Server (Stateless)
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
                                if now.duration_since(entry.1) >= gtp_types::Duration::from_secs(1)
                                {
                                    *entry = (1, now);
                                } else {
                                    entry.0 += 1;
                                }

                                if src.ip().is_loopback() || entry.0 <= 20 {
                                    let (server_pk, server_nonce, cookie) = {
                                        let mut psh = pending_server_handshakes.write().await;
                                        // Prune expired handshakes older than 3 seconds
                                        psh.retain(|_, (_, _, _, _, start_time)| {
                                            now.duration_since(*start_time)
                                                <= gtp_types::Duration::from_secs(3)
                                        });

                                        let cookie = stateless_tokens.generate_cookie(src, now);
                                        if let Some((existing_pair, _, _, _, _)) = psh.get(&cid) {
                                            (existing_pair.public_key, existing_pair.nonce, cookie)
                                        } else {
                                            let server_pair = EphemeralKeyPair::generate();
                                            let server_pk = server_pair.public_key;
                                            let server_nonce = server_pair.nonce;
                                            psh.insert(
                                                cid,
                                                (
                                                    server_pair,
                                                    client_public_key,
                                                    client_nonce,
                                                    src,
                                                    now,
                                                ),
                                            );
                                            (server_pk, server_nonce, cookie)
                                        }
                                    };

                                    let mut resp_buf = [0u8; 256];
                                    let s_hdr =
                                        PacketHeader::new_long(1, cid, PacketNumber(0), 0, 0);
                                    let s_hdr_len = s_hdr.encode(&mut resp_buf).unwrap_or(32);

                                    let s_frame = Frame::ServerHello {
                                        server_public_key: server_pk,
                                        server_nonce,
                                        stateless_cookie: cookie,
                                        assigned_cid: cid,
                                    };
                                    if let Ok(s_len) = s_frame.encode(&mut resp_buf[s_hdr_len..]) {
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
                                let mut pending = pending_client_handshakes.write().await;
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

                        // Case C: Incoming HandshakeFinish on Server (Address & Key Verified!)
                        if frame_type == FRAME_TYPE_HANDSHAKE_FINISH {
                            if let Ok((
                                Frame::HandshakeFinish {
                                    cookie_echo,
                                    client_proof,
                                },
                                _,
                            )) = Frame::decode(frame_payload)
                            {
                                // 1. Strict Cookie Verification (Address Ownership Verified!)
                                if stateless_tokens.verify_cookie(src, &cookie_echo, now) {
                                    let pending_entry = {
                                        let mut psh = pending_server_handshakes.write().await;
                                        psh.remove(&cid)
                                    };

                                    if let Some((
                                        server_pair,
                                        client_pk,
                                        client_nonce,
                                        _initial_src,
                                        _,
                                    )) = pending_entry
                                    {
                                        let server_nonce = server_pair.nonce;
                                        let server_pk = server_pair.public_key;
                                        let Ok(shared) =
                                            server_pair.compute_shared_secret(&client_pk)
                                        else {
                                            continue;
                                        };
                                        let keys = derive_directional_handshake_session_keys(
                                            &shared,
                                            &client_nonce,
                                            &server_nonce,
                                            cid,
                                        );

                                        // 2. Cryptographic Key Confirmation: Client & Server derived identical keys
                                        if gtp_crypto::verify_client_proof(
                                            &keys.client_tx_key,
                                            &client_pk,
                                            &server_pk,
                                            &client_proof,
                                        ) {
                                            // Instantiate verified connection (pre_validated: true).
                                            // SEC-1: the server seals with the server->client direction.
                                            let conn = GtpConnection::new_with_directional_keys(
                                                cid,
                                                src,
                                                &keys,
                                                false,
                                                true,
                                                GtpConfig::competitive_fps(),
                                            );

                                            let (tx, rx) = mpsc::channel(1024);
                                            let conn_arc = Arc::new(Mutex::new(conn));

                                            {
                                                let mut conns = connections.write().await;
                                                conns.insert(cid, (Arc::clone(&conn_arc), tx));
                                            }

                                            Self::spawn_tx_loop(
                                                Arc::clone(&socket),
                                                Arc::clone(&conn_arc),
                                            );

                                            let async_conn = AsyncGtpConnection {
                                                cid,
                                                conn: conn_arc,
                                                rx_channel: rx,
                                            };

                                            let _ = incoming_tx.send(async_conn).await;
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                    }
                }

                // 2. Regular Game Data Datagram Handling: Strictly routed by ConnectionId
                let mut datagram_copy = datagram.to_vec();
                let conns = connections.read().await;
                if let Some((conn_arc, tx)) = conns.get(&cid) {
                    let mut guard = conn_arc.lock().await;
                    if let Ok(msgs) = guard.handle_incoming_datagram(src, &mut datagram_copy, now) {
                        for msg in msgs {
                            let _ = tx.send(msg).await;
                        }
                    }
                    // CORE-4: evict closed connections so routing state does not leak
                    if guard.hot.state.is_closed() {
                        drop(guard);
                        drop(conns);
                        connections.write().await.remove(&cid);
                        continue;
                    }
                }
            }
        });
    }

    fn spawn_tx_loop(socket: Arc<UdpSocket>, conn_arc: Arc<Mutex<GtpConnection>>) {
        tokio::spawn(async move {
            let mut out_buf = [0u8; 1500];
            let mut interval = tokio::time::interval(std::time::Duration::from_micros(500));

            loop {
                interval.tick().await;
                let now = MonotonicTime::now();
                let mut guard = conn_arc.lock().await;

                // Core-C1: only a fully Closed connection ends the TX loop. A Draining
                // connection must keep transmitting so its CLOSE frame goes out.
                if guard.hot.state.is_closed() {
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

        let cid = ConnectionId(0x554433221100AABB);

        // Spawn client connect task
        let client_task =
            tokio::spawn(async move { client_ep.connect(cid, server_addr, true).await.unwrap() });

        // Server accepts incoming connection dynamically
        let mut server_conn =
            tokio::time::timeout(std::time::Duration::from_secs(2), server_ep.accept())
                .await
                .expect("Server accept timed out")
                .expect("Accept channel closed");

        let client_conn = client_task.await.unwrap();

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
