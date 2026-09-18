use crate::async_connection::AsyncGtpConnection;
use gtp_core::{GtpConfig, GtpConnection, ReceivedMessage};
use gtp_crypto::{derive_directional_handshake_session_keys, EphemeralKeyPair};
use gtp_path::StatelessTokenManager;
use gtp_types::{ConnectionId, MessageClass, MonotonicTime, PacketNumber, Result, TransportError};
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
// Only the gated insecure branch and tests reference the sim secret.
#[cfg(any(test, feature = "insecure-plaintext"))]
use gtp_core::state::OFFLINE_SIM_MASTER_SECRET;

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

/// Close code reported to the peer when an application stops draining its own
/// receive channel and a reliable message would otherwise have to be dropped (N-2).
const RECEIVE_OVERFLOW_CLOSE_CODE: u16 = 8;

/// Close code reported to the peer when the application has dropped its handle for a
/// connection, so nothing is left to deliver to (N-2).
const APPLICATION_GONE_CLOSE_CODE: u16 = 9;

/// Decides whether a received datagram may be inspected as an unauthenticated
/// handshake packet (N-1).
///
/// Handshake frames are only ever emitted inside **long-header** packets
/// (`ClientHello`, `ServerHello` and `HandshakeFinish` are all built with
/// `PacketHeader::new_long`), while every established-connection datagram carries a
/// short header. Without that gate the routing loop read `payload[0]` of a short
/// header packet — which is the first byte of AEAD *ciphertext* — and compared it
/// against `FRAME_TYPE_CLIENT_HELLO` / `_SERVER_HELLO` / `_HANDSHAKE_FINISH`
/// (`0x0B`, `0x0C`, `0x0D`). Ciphertext bytes are uniformly distributed, so 3 of 256
/// datagrams entered a handshake branch, failed to decode as a handshake frame, and
/// were then dropped by the branch's unconditional `continue` — a silent, permanent
/// **1.172%** loss floor on every connection, applied before decryption and
/// therefore invisible to every layer below.
fn is_handshake_candidate(header: &PacketHeader, header_len: usize, datagram_len: usize) -> bool {
    header.flags.is_long_header() && header_len < datagram_len
}

/// Per-IP ClientHello rate limiter with self-pruning state.
///
/// The map of seen source addresses is itself bounded: every hello prunes
/// entries older than the accounting window first, so spoofed-source floods
/// cannot grow it without limit (each distinct IP used to insert an entry
/// that nothing ever removed).
#[derive(Default)]
struct HelloRateLimiter {
    seen: FxHashMap<IpAddr, (u32, MonotonicTime)>,
}

impl HelloRateLimiter {
    /// Accounts one hello from `ip` and returns whether it is within `max`
    /// per `window`. Loopback is always allowed (local tooling).
    fn allow(
        &mut self,
        ip: IpAddr,
        now: MonotonicTime,
        window: gtp_types::Duration,
        max: u32,
    ) -> bool {
        if ip.is_loopback() {
            return true;
        }
        self.seen
            .retain(|_, (_, t)| now.duration_since(*t) < window);
        let entry = self.seen.entry(ip).or_insert((0, now));
        if now.duration_since(entry.1) >= window {
            *entry = (1, now);
        } else {
            entry.0 += 1;
        }
        entry.0 <= max
    }

    /// Retained-entry count (test observability for the pruning bound).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.seen.len()
    }
}

/// Async GTP Endpoint running on top of Tokio with automated X25519 Handshake and Anti-Amplification defense.
pub struct GtpEndpoint {
    socket: Arc<UdpSocket>,
    connections: ConnectionMap,
    stateless_tokens: Arc<StatelessTokenManager>,
    pending_client_handshakes: PendingClientHandshakeMap,
    pending_server_handshakes: PendingServerHandshakeMap,
    hello_rate_limiter: Arc<Mutex<HelloRateLimiter>>,
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
            hello_rate_limiter: Arc::new(Mutex::new(HelloRateLimiter::default())),
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
            // Fail closed: plaintext connections exist only in test/sim builds
            // (the null-cipher protector is feature-gated off in production).
            #[cfg(any(test, feature = "insecure-plaintext"))]
            {
                #[allow(deprecated)]
                let conn = GtpConnection::new_with_role(
                    cid,
                    peer_addr,
                    false,
                    true,
                    OFFLINE_SIM_MASTER_SECRET,
                    GtpConfig::default(),
                );
                return Ok(self.register_connection(cid, conn).await);
            }
            #[cfg(not(any(test, feature = "insecure-plaintext")))]
            return Err(TransportError::HandshakeFailed(
                "insecure (plaintext) connections are disabled in production builds",
            ));
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

        if let Err(e) = self
            .socket
            .send_to(&hello_buf[..total_len], peer_addr)
            .await
        {
            // Do not strand the pending entry on a failed initial send —
            // nothing but a later same-CID handshake would ever clear it.
            let mut pending = self.pending_client_handshakes.write().await;
            pending.remove(&cid);
            return Err(TransportError::Io(e.to_string()));
        }

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

        // 4. Send HandshakeFinish confirmation with Key Confirmation Proof.
        // SEC-9: keyed by the dedicated confirmation key derived from the shared
        // secret (never an AEAD traffic key), bound to the full transcript.
        let transcript = gtp_crypto::HandshakeTranscript {
            client_pk,
            server_pk,
            client_nonce,
            server_nonce,
            connection_id: cid,
        };
        let client_proof = gtp_crypto::compute_client_proof(&shared, &transcript);
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

        let async_conn = self.register_connection(cid, conn).await;

        // 5. Establishment confirmation. The server accepts the session only after
        // it verifies this HandshakeFinish; a Finish that is lost — or that races
        // ahead of the server's connection registration — leaves the session
        // half-open, and the client would stream application data into a black hole
        // with no way to notice. So do not declare the connection established on
        // faith: enqueue an ack-eliciting Ping (which the tx loop sends), wait for
        // the server's ACK, and retransmit the HandshakeFinish until that ACK
        // arrives — mirroring the ClientHello retransmission above. An ACK can only
        // come from a peer that decrypted our traffic, which proves it accepted and
        // registered the connection.
        let fin_datagram_len = fin_hdr_len + fin_frame_len;
        let mut confirmed = false;
        'confirm: for _ in 0..8 {
            {
                let mut guard = async_conn.conn.lock().await;
                let now = MonotonicTime::now();
                let _ = guard.control().send_ping(0x00C0_FFEE, now);
            }
            // Poll for the server's ACK for up to ~400ms (≈ several RTTs).
            for _ in 0..8 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if async_conn
                    .conn
                    .lock()
                    .await
                    .hot
                    .loss_detector
                    .has_received_ack()
                {
                    confirmed = true;
                    break 'confirm;
                }
            }
            // No confirmation yet: the Finish may have been lost. Retransmit it.
            let _ = self
                .socket
                .send_to(&fin_buf[..fin_datagram_len], peer_addr)
                .await;
        }

        // If still unconfirmed after the retransmit budget, return the connection
        // anyway: a genuinely one-way or dead path should still hand the caller a
        // handle (the dead link is observable via telemetry — Total RX Packets stays
        // 0) rather than blocking connect() indefinitely.
        let _ = confirmed;

        Ok(async_conn)
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

                // 1. Check for Handshake Frames in unauthenticated / handshake packets.
                // N-1: gated on the long-header bit — a short header means an
                // established connection, whose payload is ciphertext, not frames.
                if is_handshake_candidate(&header, header_len, datagram.len()) {
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
                                // Rate limit per IP (max 20 hellos per second,
                                // self-pruning map).
                                let allowed = {
                                    let mut rl = rate_limiter.lock().await;
                                    rl.allow(src.ip(), now, gtp_types::Duration::from_secs(1), 20)
                                };

                                if allowed {
                                    let (server_pk, server_nonce, cookie) = {
                                        let mut psh = pending_server_handshakes.write().await;
                                        // Prune expired handshakes older than 3 seconds
                                        psh.retain(|_, (_, _, _, _, start_time)| {
                                            now.duration_since(*start_time)
                                                <= gtp_types::Duration::from_secs(3)
                                        });

                                        let cookie = stateless_tokens.generate_cookie(src, now);
                                        // Reuse the stored server ephemeral ONLY for a genuine
                                        // ClientHello retransmit — one carrying the SAME client
                                        // key material. A ClientHello for this CID that carries
                                        // DIFFERENT client material is a new handshake (a fresh
                                        // client that happens to reuse the connection id, or a
                                        // stale entry left by an earlier attempt): it must
                                        // supersede the stale entry, otherwise the ServerHello
                                        // advertises an ephemeral derived against the OLD client
                                        // key and the client_proof in the eventual
                                        // HandshakeFinish can never match — the session then
                                        // half-opens (server never accepts, client streams into a
                                        // black hole). Keying the pending state on (CID + client
                                        // ephemeral) instead of the CID alone closes that seam.
                                        let is_retransmit = psh.get(&cid).is_some_and(
                                            |(_, stored_pk, stored_nonce, _, _)| {
                                                *stored_pk == client_public_key
                                                    && *stored_nonce == client_nonce
                                            },
                                        );
                                        if let Some((existing_pair, _, _, _, _)) =
                                            psh.get(&cid).filter(|_| is_retransmit)
                                        {
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
                                        let transcript = gtp_crypto::HandshakeTranscript {
                                            client_pk,
                                            server_pk,
                                            client_nonce,
                                            server_nonce,
                                            connection_id: cid,
                                        };
                                        if gtp_crypto::verify_client_proof(
                                            &shared,
                                            &transcript,
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
                let mut evict = false;

                // N-2: this scope holds BOTH the connection map's read lock and this
                // connection's mutex, and it is entered by the single RX task that
                // serves every connection on this socket. Nothing inside it may await
                // on anything an application controls. Delivery therefore never awaits:
                // `try_send` decides immediately, and the class decides what a full
                // channel means. Awaiting here used to park the whole endpoint — no
                // other connection was served, no new client could be registered, and
                // the saturated connection's own application could not retake its lock,
                // which closed a circular wait between the RX loop and that application.
                {
                    let conns = connections.read().await;
                    if let Some((conn_arc, tx)) = conns.get(&cid) {
                        let mut guard = conn_arc.lock().await;
                        if let Ok(msgs) =
                            guard.handle_incoming_datagram(src, &mut datagram_copy, now)
                        {
                            let produced = msgs.len();
                            for (idx, msg) in msgs.into_iter().enumerate() {
                                match tx.try_send(msg) {
                                    Ok(()) => {}
                                    Err(mpsc::error::TrySendError::Full(msg)) => match msg.class {
                                        // Droppable by contract, and the receive-side
                                        // state table already applied supersession
                                        // upstream, so what is dropped here is surplus.
                                        MessageClass::Unreliable
                                        | MessageClass::UnreliableSequenced { .. } => {
                                            guard.cold.total_dropped_frames += 1;
                                        }
                                        // Reliable classes were already acknowledged to
                                        // the peer, so the sender will never retransmit
                                        // them: dropping is silent data loss, and for an
                                        // ordered group it breaks the ordering of
                                        // everything after it. Growing without bound
                                        // turns a slow consumer into memory exhaustion,
                                        // and waiting is the stall being fixed here. An
                                        // application that cannot keep up with its own
                                        // reliable stream cannot continue safely, so the
                                        // connection ends explicitly and the peer is
                                        // told why.
                                        MessageClass::ReliableUnordered
                                        | MessageClass::ReliableOrdered { .. } => {
                                            // This message and every one still behind it
                                            // in this datagram go undelivered, so the
                                            // counter has to account for the whole tail,
                                            // not just the message that tripped the
                                            // overflow.
                                            guard.cold.total_dropped_frames +=
                                                (produced - idx) as u64;
                                            if guard.hot.state.is_active() {
                                                let _ = guard.control().graceful_close(
                                                    RECEIVE_OVERFLOW_CLOSE_CODE,
                                                    "receive queue overflow",
                                                    now,
                                                );
                                            }
                                            evict = true;
                                            break;
                                        }
                                    },
                                    // The application dropped its handle: there is
                                    // nothing left to deliver to. Retiring the routing
                                    // entry is not enough on its own — `spawn_tx_loop`
                                    // exits only on `is_closed()`, so the connection
                                    // would keep a task alive, keep the GtpConnection
                                    // alive with it, and keep PTO-retransmitting to a
                                    // peer whose replies are no longer routed anywhere:
                                    // one leaked task, connection and traffic stream per
                                    // handle the application drops. Closing it lets the
                                    // TX loop finish and tells the peer why.
                                    Err(mpsc::error::TrySendError::Closed(_)) => {
                                        guard.cold.total_dropped_frames += (produced - idx) as u64;
                                        if guard.hot.state.is_active() {
                                            let _ = guard.control().graceful_close(
                                                APPLICATION_GONE_CLOSE_CODE,
                                                "application handle dropped",
                                                now,
                                            );
                                        }
                                        evict = true;
                                        break;
                                    }
                                }
                            }
                        }
                        // CORE-4: evict closed connections so routing state does not leak
                        if guard.hot.state.is_closed() {
                            evict = true;
                        }
                    }
                }
                // Both guards are released by the scope above, in that order, BEFORE
                // the map write below — taking the write lock while still holding the
                // read guard would deadlock this task against itself.
                if evict {
                    connections.write().await.remove(&cid);
                    continue;
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

    /// N-1 regression: a datagram whose first ciphertext byte collides with a
    /// handshake frame type must still be routed as ordinary traffic. Before the
    /// gate this exact datagram entered a handshake branch and was dropped by
    /// its `continue`.
    ///
    /// The colliding datagram is CRAFTED deterministically: the connection's
    /// legacy key material is static, so its keystream is a pure function of
    /// the packet number — sealing the same payload under successive PNs
    /// sweeps first-ciphertext bytes until one lands on a handshake type
    /// (pigeonhole: expected ~85 tries, bounded at 4096). Relying on the live
    /// TX loop instead would race the congestion window: with no peer ACKs the
    /// window fills and production stalls long before 3 random collisions are
    /// guaranteed.
    #[test]
    fn short_header_ciphertext_colliding_with_handshake_types_is_still_routed() {
        let cid = ConnectionId(0xA11C_E000_1234_5678);
        let peer: SocketAddr = "127.0.0.1:9000".parse().unwrap();
        let mut conn = GtpConnection::new_with_role(
            cid,
            peer,
            true,
            true,
            OFFLINE_SIM_MASTER_SECRET,
            GtpConfig::default(),
        );

        let payload = b"n1-collision-probe".to_vec();
        let aad = b"short_header_aad";
        let mut out = [0u8; 1500];
        let mut collisions = 0usize;

        for pn_offset in 1u64..=4096 {
            // Deterministic keystream sweep: seal under successive packet numbers.
            let pn = PacketNumber(pn_offset);
            out[..payload.len()].copy_from_slice(&payload);
            let sealed = gtp_crypto::PacketProtector::seal(
                &gtp_crypto::GtpAeadProtector::new(*conn.hot.tx_key, *conn.hot.tx_iv),
                pn,
                cid,
                aad,
                &mut out[..],
                payload.len(),
            )
            .expect("probe seals");
            let sealed_len = sealed + 24; // + short header

            // Build a real short-header datagram around the sealed payload.
            let header = PacketHeader::new_short(cid, pn, 1_000_000, 0);
            let header_len = header.encode(&mut out).expect("header encodes");
            assert_eq!(header_len, 24);
            let total_len = header_len + sealed;

            // Established traffic must always use the short header — that is
            // what makes the gate a sound discriminator in the first place.
            assert!(
                !header.flags.is_long_header(),
                "established-connection traffic must carry a short header"
            );

            let first_payload_byte = out[header_len];
            if matches!(
                first_payload_byte,
                FRAME_TYPE_CLIENT_HELLO | FRAME_TYPE_SERVER_HELLO | FRAME_TYPE_HANDSHAKE_FINISH
            ) {
                collisions += 1;
                assert!(
                    !is_handshake_candidate(&header, header_len, total_len),
                    "datagram whose ciphertext starts with 0x{first_payload_byte:02X} was \
                     routed into the handshake path and would be dropped (N-1)"
                );
            }
            if collisions >= 3 {
                return;
            }
        }

        panic!(
            "no ciphertext/handshake-type collision produced in 4096 sealed probes — \
             the probe is broken, not the gate"
        );
    }

    /// The gate must not cost the handshake anything: all three handshake frames are
    /// emitted in long-header packets and must still be inspected.
    /// The per-IP hello limiter must not grow without bound under
    /// spoofed-source floods: entries older than the accounting window are
    /// pruned by every allow() call.
    #[test]
    fn hello_rate_limiter_prunes_stale_entries() {
        use std::net::Ipv4Addr;
        let mut rl = HelloRateLimiter::default();
        let t0 = MonotonicTime::from_micros(1_000_000);
        let window = gtp_types::Duration::from_secs(1);

        // A flood of distinct (spoofed) sources at t0.
        for i in 0..1000u32 {
            let ip = IpAddr::V4(Ipv4Addr::new(10, (i >> 16) as u8, (i >> 8) as u8, i as u8));
            assert!(rl.allow(ip, t0, window, 20), "first hello is allowed");
        }
        assert_eq!(rl.len(), 1000);

        // One hello a window later: the stale flood is pruned first.
        let t1 = t0 + gtp_types::Duration::from_secs(2);
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        assert!(rl.allow(ip, t1, window, 20));
        assert!(
            rl.len() <= 2,
            "stale entries must be reaped, got {}",
            rl.len()
        );

        // And the limit itself: 20 hellos per window pass, the 21st does not.
        let t2 = t1 + gtp_types::Duration::from_secs(2);
        let flood_ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7));
        for _ in 0..20 {
            assert!(rl.allow(flood_ip, t2, window, 20));
        }
        assert!(!rl.allow(flood_ip, t2, window, 20));
        // Loopback bypasses the limit entirely.
        assert!(rl.allow(IpAddr::V4(Ipv4Addr::LOCALHOST), t2, window, 0));
    }

    #[test]
    fn long_header_handshake_packets_are_still_inspected() {
        let cid = ConnectionId(7);
        let header = PacketHeader::new_long(1, cid, PacketNumber(0), 0, 64);
        assert!(is_handshake_candidate(&header, 28, 128));

        // A long header with no payload carries nothing to inspect.
        assert!(!is_handshake_candidate(&header, 28, 28));
    }

    /// Saturates one connection's application channel while a second one stays healthy,
    /// and hands back both endpoints so a test can inspect the routing map directly.
    /// These two live inside the crate on purpose: asserting on eviction and on
    /// `total_dropped_frames` needs private state, and neither is worth widening the
    /// public API for.
    #[cfg(test)]
    async fn saturate_one_of_two_connections(
        flood: usize,
        reliable: bool,
    ) -> (
        GtpEndpoint,
        GtpEndpoint,
        GtpEndpoint,
        AsyncGtpConnection,
        AsyncGtpConnection,
        AsyncGtpConnection,
        AsyncGtpConnection,
        ConnectionId,
        ConnectionId,
    ) {
        let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let server_addr = server_ep.local_addr().unwrap();
        let cid_a = ConnectionId(0xEEEE_0000_0000_0001);
        let cid_b = ConnectionId(0xEEEE_0000_0000_0002);

        let ep_a = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let cli_a = ep_a.connect(cid_a, server_addr, true).await.unwrap();
        let srv_a = server_ep.accept().await.unwrap();

        let ep_b = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let cli_b = ep_b.connect(cid_b, server_addr, true).await.unwrap();
        let srv_b = server_ep.accept().await.unwrap();

        for i in 0..flood {
            let _ = if reliable {
                cli_a
                    .send_reliable_unordered(
                        format!("r-{i}").into_bytes(),
                        PriorityTier::P3ReliableGameplay,
                    )
                    .await
            } else {
                cli_a
                    .send_unreliable(format!("u-{i}").into_bytes(), PriorityTier::P1Input)
                    .await
            };
            if i % 400 == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // `srv_b` is handed back deliberately: dropping it would close B's channel and
        // the RX loop would retire B as a gone application, which is a different test.
        (
            server_ep, ep_a, ep_b, cli_a, cli_b, srv_a, srv_b, cid_a, cid_b,
        )
    }

    /// N-2: an application that drops its handle leaves a `Closed` channel behind. The
    /// routing entry must go, and so must the connection itself — `spawn_tx_loop` exits
    /// only on `is_closed()`, so an evicted-but-Established connection keeps a task
    /// alive, keeps the `GtpConnection` alive with it, and keeps PTO-retransmitting to a
    /// peer whose replies no longer reach it. This asserts the whole lifecycle, not just
    /// the map entry.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn dropping_the_application_handle_closes_the_connection_and_ends_the_tx_loop() {
        let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let server_addr = server_ep.local_addr().unwrap();
        let cid = ConnectionId(0xEEEE_0000_0000_0009);

        let ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let cli = ep.connect(cid, server_addr, true).await.unwrap();
        let srv = server_ep.accept().await.unwrap();

        // Take our own reference BEFORE the handle goes, so the connection stays
        // observable after it has been evicted from the routing map.
        let conn_arc = {
            let conns = server_ep.connections.read().await;
            let (arc, _) = conns.get(&cid).expect("routable while the app holds it");
            Arc::clone(arc)
        };

        // The application is gone; its Receiver goes with it.
        drop(srv);

        // 1. The routing entry is retired.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let _ = cli
                .send_unreliable(b"still-here".to_vec(), PriorityTier::P1Input)
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if !server_ep.connections.read().await.contains_key(&cid) {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the routing entry survived the application dropping its handle"
            );
        }

        // 2. The connection itself reaches Closed, which is what lets the TX loop stop.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !conn_arc.lock().await.hot.state.is_closed() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the connection stayed open after eviction, so the TX loop never exits"
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // 3. The TX task has actually ended: it held the only other strong reference
        //    once the map entry was removed, so the count falling to ours proves it.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while Arc::strong_count(&conn_arc) > 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the TX task is still holding the connection: strong_count = {}",
                Arc::strong_count(&conn_arc)
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // 4. And it is not still transmitting to a peer that can no longer be heard.
        let before = conn_arc.lock().await.cold.total_tx_packets;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let after = conn_arc.lock().await.cold.total_tx_packets;
        assert_eq!(
            before, after,
            "the connection kept transmitting after it was closed and evicted"
        );
    }

    /// N-2: drops must be counted, and counted for the whole tail a close discards —
    /// otherwise a silent policy is also an unmeasurable one.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn overflow_drops_are_counted_and_the_other_connection_survives() {
        let (server_ep, _ep_a, _ep_b, _cli_a, cli_b, _srv_a, _srv_b, cid_a, cid_b) =
            saturate_one_of_two_connections(1600, false).await;

        let dropped = {
            let conns = server_ep.connections.read().await;
            let (conn_arc, _) = conns.get(&cid_a).expect("A is still routable");
            let guard = conn_arc.lock().await;
            guard.cold.total_dropped_frames
        };
        assert!(
            dropped > 0,
            "a saturated unreliable stream must register drops, got {dropped}"
        );

        // B was never touched: still routable, and still delivering.
        assert!(
            server_ep.connections.read().await.contains_key(&cid_b),
            "B must be unaffected by A's overflow"
        );
        for i in 0..10 {
            let _ = cli_b
                .send_unreliable(format!("b-{i}").into_bytes(), PriorityTier::P1Input)
                .await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let b_rx = {
            let conns = server_ep.connections.read().await;
            let (conn_arc, _) = conns.get(&cid_b).unwrap();
            let guard = conn_arc.lock().await;
            guard.cold.total_rx_packets
        };
        assert!(b_rx > 0, "B stopped receiving while A was saturated");
    }

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
