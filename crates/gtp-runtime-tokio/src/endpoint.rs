use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex, RwLock};
use crate::async_connection::AsyncGtpConnection;
use gtp_core::{GtpConnection, ReceivedMessage};
use gtp_types::{ConnectionId, MonotonicTime, Result, TransportError};

/// Async GTP Endpoint running on top of Tokio.
pub struct GtpEndpoint {
    socket: Arc<UdpSocket>,
    connections: Arc<RwLock<HashMap<ConnectionId, (Arc<Mutex<GtpConnection>>, mpsc::Sender<ReceivedMessage>)>>>,
}

impl GtpEndpoint {
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;

        let endpoint = Self {
            socket: Arc::new(socket),
            connections: Arc::new(RwLock::new(HashMap::new())),
        };

        endpoint.start_rx_loop();
        Ok(endpoint)
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    pub async fn connect(&self, cid: ConnectionId, peer_addr: SocketAddr, secure: bool) -> AsyncGtpConnection {
        let (tx, rx) = mpsc::channel(1024);
        let conn = GtpConnection::new(cid, peer_addr, secure);
        let conn_arc = Arc::new(Mutex::new(conn));

        {
            let mut conns = self.connections.write().await;
            conns.insert(cid, (Arc::clone(&conn_arc), tx));
        }

        // Start background TX pump task for this connection
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

        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((bytes, src)) => {
                        let now = MonotonicTime::now();
                        let mut datagram_copy = buf[..bytes].to_vec();

                        let conns = connections.read().await;
                        for (conn_arc, tx) in conns.values() {
                            let mut guard = conn_arc.lock().await;
                            if guard.peer_addr() == src || true {
                                if let Ok(msgs) = guard.handle_incoming_datagram(src, &mut datagram_copy, now) {
                                    for msg in msgs {
                                        let _ = tx.send(msg).await;
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => break,
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

                while let Ok(Some((dest, len))) = guard.produce_outgoing_datagram(now, &mut out_buf) {
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
        let server_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server_ep.local_addr().unwrap();

        let client_ep = GtpEndpoint::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
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
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), server_conn.recv())
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
