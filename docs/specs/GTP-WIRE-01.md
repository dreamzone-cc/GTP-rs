# GTP-WIRE-01: Packet Formats & Wire Framing

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Packet Layout
Each UDP datagram contains a GTP packet consisting of:
1. **Header** (Common, Long, or Short)
2. **Encrypted/Authenticated Payload** (Series of Frames)
3. **AEAD Authentication Tag** (16 bytes in secure profile)

### 1.1 Common Header Fields
- `flags` (u8):
  - Bit 7: Header Form (1 = Long Header, 0 = Short Header)
  - Bit 6: Key Phase
  - Bit 5: ACK Present
  - Bit 4-3: ECN Bits (00=Not-ECT, 01=ECT(1), 10=ECT(0), 11=CE)
  - Bit 2-0: Reserved
- `version` (u32, Long Header only): Protocol version (`0x00010001` for v1.1)
- `header_len` (u8): Length of header in bytes
- `connection_id` (u64): 64-bit opaque connection identifier
- `packet_number` (u64): Monotonically increasing packet sequence number
- `timestamp` (u32): Low 32 bits of monotonic timestamp in microseconds
- `payload_len` (u16): Length of encrypted payload

## 2. Frame Format
Payload consists of concatenated TLV frames:
- `0x01 ACK`: `largest_acked` (u64), `ack_delay_us` (u32), `range_count` (u8), ranges: `[(gap: u32, len: u32)]`, ECN counters (`ect0`, `ect1`, `ce`).
- `0x02 DATA`: `message_id` (u64), `state_key` (u48), `sequence` (u32), `generation` (u32), `deadline_ms` (u16), `payload_len` (u16), `payload`.
- `0x03 RELIABLE_DATA`: `message_id` (u64), `fragment_id` (u16), `total_fragments` (u16), `group_id` (u16), `order_seq` (u32), `payload_len` (u16), `payload`.
- `0x04 RETX`: `message_id` (u64), `fragment_id` (u16), `transmission_id` (u8), `payload_len` (u16), `payload`.
- `0x05 PING`: `nonce` (u64).
- `0x06 PATH_CHALLENGE`: `challenge_data` ([u8; 8]).
- `0x07 PATH_RESPONSE`: `response_data` ([u8; 8]).
- `0x08 MTU_PROBE`: `probe_id` (u32), padding.
- `0x09 CLOSE`: `error_code` (u16), `reason_len` (u8), `reason` (str).
- `0x0A ACK_FREQUENCY`: `ack_frequency_packets` (u8), `max_ack_delay_ms` (u16), `reorder_threshold` (u8).
- `0x0B HANDSHAKE_INIT`: `client_nonce` ([u8; 16]), `version` (u32).
- `0x0C HANDSHAKE_RESPONSE`: `server_nonce` ([u8; 16]), `stateless_cookie` ([u8; 32]), `assigned_cid` (u64).
- `0x0D HANDSHAKE_FINISH`: `cookie_echo` ([u8; 32]), `client_proof` ([u8; 16]).
- `0x0E PADDING`: Padding bytes.
