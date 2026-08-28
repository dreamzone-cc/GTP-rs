# GTP-REC-01: Loss Recovery, RTT & ACK Management

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. ACK Management
1. **ACK Ranges:** Receiver encodes non-contiguous acknowledged packet numbers using an array of `(gap, length)` pairs bounded by `MAX_ACK_RANGES = 32`.
2. **Adaptive ACK Frequency:**
   - Default: ACK every 2 ack-eliciting packets or after `max_ack_delay` (25ms).
   - Loss/Reorder Trigger: Immediate ACK upon detecting out-of-order packet.
   - Control Frame Trigger: Immediate ACK upon receiving PATH, PING, or HANDSHAKE frames.

## 2. RTT Estimation
- Samples taken from first transmission of ACK-eliciting packets:
  - `sample_rtt = now - send_time - ack_delay`
  - `smoothed_rtt = (7/8)*smoothed_rtt + (1/8)*sample_rtt`
  - `rttvar = (3/4)*rttvar + (1/4)*|smoothed_rtt - sample_rtt|`
  - `min_rtt = min(min_rtt, sample_rtt)`

## 3. Loss Detection Algorithms
1. **Packet Threshold:** A packet is declared lost if $k \ge 3$ newer packets have been acknowledged.
2. **Time Threshold:** A packet is declared lost if $\text{elapsed} \ge \frac{9}{8} \times \max(\text{smoothed\_rtt}, \text{latest\_rtt})$.
3. **Probe Timeout (PTO):** $\text{PTO} = \text{smoothed\_rtt} + 4 \times \text{rttvar} + \text{max\_ack\_delay}$. On expiry, send probe packet and double PTO.

## 4. Selective Recovery Policy
- When a packet containing multiple frames is lost:
  - Only `RELIABLE_DATA` frames that have not expired or been superseded are retransmitted.
  - Retransmissions are framed as `RETX` within a new packet with a new `PacketNumber` and incremented `TransmissionId`.
