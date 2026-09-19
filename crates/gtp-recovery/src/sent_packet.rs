use gtp_types::{FragmentId, MessageId, MonotonicTime, PacketNumber, TransmissionId};

/// Logical retransmission record for selective recovery of a reliable fragment.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct RetransmissionRecord {
    pub message_id: MessageId,
    pub fragment_id: FragmentId,
    pub transmission_id: TransmissionId,
    pub group_id: u16,
    pub order_seq: u32,
    /// F1: how many fragments the logical message has (1 = unfragmented) —
    /// a retransmitted fragment must carry its set's total on the wire or
    /// the receiver would treat it as a complete message.
    pub total_fragments: u16,
    pub payload: Vec<u8>,
}

/// Metadata stored per transmitted packet for ACK tracking and loss recovery.
#[derive(Clone, Debug)]
pub struct SentPacketRecord {
    pub packet_number: PacketNumber,
    pub send_time: MonotonicTime,
    pub bytes: usize,
    pub ack_eliciting: bool,
    pub in_flight: bool,
    pub retransmittable_frames: Vec<RetransmissionRecord>,
}
