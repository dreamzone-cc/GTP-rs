use gtp_core::{ControlEvent, DetailedMetrics, GtpConnection, NetworkFeedback, ReceivedMessage};
use gtp_types::{
    ConnectionId, GenerationId, MessageId, MonotonicTime, OrderedGroupId, PriorityTier, Result,
    StateKey, StateSequence,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// High-level asynchronous GTP Connection handle for tokio applications.
pub struct AsyncGtpConnection {
    pub cid: ConnectionId,
    pub(crate) conn: Arc<Mutex<GtpConnection>>,
    pub(crate) rx_channel: mpsc::Receiver<ReceivedMessage>,
}

impl AsyncGtpConnection {
    pub fn new(conn: GtpConnection, rx_channel: mpsc::Receiver<ReceivedMessage>) -> Self {
        let cid = conn.connection_id();
        Self {
            cid,
            conn: Arc::new(Mutex::new(conn)),
            rx_channel,
        }
    }

    pub fn connection_id(&self) -> ConnectionId {
        self.cid
    }

    pub async fn peer_addr(&self) -> SocketAddr {
        self.conn.lock().await.peer_addr()
    }

    pub async fn send_unreliable(
        &self,
        payload: Vec<u8>,
        priority: PriorityTier,
    ) -> Result<MessageId> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.send_unreliable(payload, priority, None, now)
    }

    pub async fn send_sequenced(
        &self,
        state_key: StateKey,
        sequence: StateSequence,
        generation: GenerationId,
        payload: Vec<u8>,
    ) -> Result<MessageId> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.send_sequenced(state_key, sequence, generation, None, payload, now)
    }

    pub async fn send_reliable_unordered(
        &self,
        payload: Vec<u8>,
        priority: PriorityTier,
    ) -> Result<MessageId> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.send_reliable_unordered(payload, priority, None, now)
    }

    pub async fn send_reliable_ordered(
        &self,
        group_id: OrderedGroupId,
        payload: Vec<u8>,
        priority: PriorityTier,
    ) -> Result<MessageId> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.send_reliable_ordered(group_id, payload, priority, None, now)
    }

    pub async fn recv(&mut self) -> Option<ReceivedMessage> {
        self.rx_channel.recv().await
    }

    pub async fn feedback(&self) -> NetworkFeedback {
        let guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.feedback(now)
    }

    // ==========================================
    // Control API Async Extensions
    // ==========================================

    pub async fn set_ack_frequency(
        &self,
        ack_frequency_packets: u8,
        max_ack_delay_ms: u16,
        reorder_threshold: u8,
    ) -> Result<()> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.control().set_ack_frequency(
            ack_frequency_packets,
            max_ack_delay_ms,
            reorder_threshold,
            now,
        )
    }

    pub async fn trigger_path_challenge(&self, new_addr: SocketAddr, nonce: [u8; 8]) -> Result<()> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.control().trigger_path_challenge(new_addr, nonce, now)
    }

    pub async fn send_ping(&self, nonce: u64) -> Result<()> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.control().send_ping(nonce, now)
    }

    pub async fn query_metrics(&self) -> DetailedMetrics {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.control().query_metrics(now)
    }

    pub async fn graceful_close(&self, error_code: u16, reason: &'static str) -> Result<()> {
        let mut guard = self.conn.lock().await;
        let now = MonotonicTime::now();
        guard.control().graceful_close(error_code, reason, now)
    }

    pub async fn drain_events(&self) -> Vec<ControlEvent> {
        let mut guard = self.conn.lock().await;
        guard.drain_events()
    }
}
