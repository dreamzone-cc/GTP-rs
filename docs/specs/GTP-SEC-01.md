# GTP-SEC-01: Security Boundary, AEAD & Replay Defense

**Status:** Normative Sub-Specification
**Version:** 1.2 (X-2/X-3 documentation alignment — the implementation at `gtp-core/src/connection.rs` is and was the normative behavior)

---

## 1. Packet Protection Model
1. **AEAD Cipher:** Standard ChaCha20-Poly1305 (RFC 8439).
2. **Nonce Derivation (X-3):**
   $$\text{Nonce} = \text{IV} \oplus (\text{ConnectionID}_{64} \mathbin{\Vert} \text{PacketNumber}_{64})$$
   The full 64-bit Connection ID and the full 64-bit Packet Number are mixed into the
   96-bit nonce (`gtp-crypto/src/aead.rs`, pinned by `nonce_uses_full_packet_number`),
   guaranteeing a unique nonce per packet under a given connection key. Uniqueness rests
   on one invariant: **one traffic key belongs to exactly one connection and one
   direction** (INV-9) — direction separation comes from the `c2s`/`s2c` HKDF labels
   (SEC-1), so a client packet N and a server packet N never share a (key, nonce) pair.
3. **Additional Authenticated Data (AAD) (X-2):**
   The **entire packet header as encoded on the wire** — Flags, Version, Header Length,
   Connection ID, Packet Number, Timestamp, and Payload Length — is fed as AAD
   (`gtp-core/src/connection.rs`, both the RX open and TX seal paths). Every
   behavior-affecting header byte is integrity-protected (INV-10); flipping any header
   bit fails authentication.

## 2. Replay Protection
- Receiver maintains a sliding bitmap window (128 packets).
- Packets with $\text{PacketNumber} < \text{Largest} - 128$ are rejected.
- Packets within window are accepted if their corresponding bit is unset, and bit is set upon successful AEAD authentication.

## 3. Plaintext Lab Mode
- `PlaintextProtector` is available for zero-overhead local simulation, benchmarking, and unit testing.
