# GTP-PATH-01: Path Management, Handshake & Anti-Amplification

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Handshake State Machine
```text
[INITIAL]
   │ (Send HANDSHAKE_INIT)
   ▼
[HANDSHAKING]
   │ (Receive HANDSHAKE_RESPONSE with Cookie)
   ▼
[VALIDATED]
   │ (Send HANDSHAKE_FINISH & establish keys)
   ▼
[ESTABLISHED]
   │ (Receive CLOSE / Idle Timeout)
   ▼
[DRAINING]
   │ (Draining period expires)
   ▼
[CLOSED]
```

## 2. Stateless Validation & Anti-Amplification
1. **Stateless Cookie:** Server responds to unknown initial packets with an HMAC-signed token:
   $$\text{Token} = \text{HMAC}(\text{ServerSecret}, \text{ClientIP} \mathbin{\Vert} \text{ClientPort} \mathbin{\Vert} \text{Timestamp})$$
2. **Anti-Amplification Limit:** Server MUST NOT send more than $3 \times$ the number of bytes received from unvalidated client addresses.

## 3. Path Validation & NAT Rebinding
1. When packets arrive from a new 4-tuple with a valid established CID:
   - Server does NOT switch active path immediately.
   - Server sends `PATH_CHALLENGE` containing an 8-byte random nonce to the new address.
2. Upon receiving `PATH_RESPONSE` matching the challenge data:
   - Path is marked `Validated`.
   - Active path is safely switched to the new address.
