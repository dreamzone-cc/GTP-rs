# GTP-SEC-01: Security Boundary, AEAD & Replay Defense

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Packet Protection Model
1. **AEAD Cipher:** Standard ChaCha20-Poly1305 (RFC 8439).
2. **Nonce Derivation:**
   $$\text{Nonce} = \text{IV} \oplus (\text{ConnectionID} \mathbin{\Vert} \text{PacketNumber})$$
   Guarantees 96-bit unique nonce for every packet transmitted under a given connection key.
3. **Additional Authenticated Data (AAD):**
   Packet header (Flags, Version, Header Length, Connection ID, Packet Number) is fed as AAD to prevent tampering with routing and numbering fields.

## 2. Replay Protection
- Receiver maintains a sliding bitmap window (128 packets).
- Packets with $\text{PacketNumber} < \text{Largest} - 128$ are rejected.
- Packets within window are accepted if their corresponding bit is unset, and bit is set upon successful AEAD authentication.

## 3. Plaintext Lab Mode
- `PlaintextProtector` is available for zero-overhead local simulation, benchmarking, and unit testing.
