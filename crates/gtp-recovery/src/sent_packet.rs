use gtp_types::{FragmentId, MessageId, MonotonicTime, PacketNumber, TransmissionId};

/// Logical retransmission record for selective recovery of a reliable fragment.
#[derive(Clone, Eq, PartialEq, Debug)]
pub struct RetransmissionRecord {
    pub message_id: MessageId,
    pub fragment_id: FragmentId,
    pub transmission_id: TransmissionId,
    pub group_id: u16,
    pub order_seq: u32,
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
