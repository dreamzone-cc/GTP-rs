# E-6 Design Options — Server Authentication (SEC-5 / DEF-1)

> **Status:** design opened 2026-09-05 (G1), per ARDP v1.1 §10 item 5 — the
> longest-lead item and the only one blocking the product rather than a phase.
> **Not implemented.** This note frames the decision; the ADR (§12.3) records
> the choice once made.
> **Blocking relationship:** G7 / any deployment carrying real user traffic.
> Does not block G1–G3 (measurement and shadow mode on hosts we control).

## 1. The problem

The handshake is **anonymous X25519** (`ClientHello`/`ServerHello`/
`HandshakeFinish`, gtp-crypto `handshake.rs`). The key exchange is
contributory-checked and the derived keys are confirmed by the client proof,
but **nothing binds the exchange to an identity**: an active on-path attacker
can complete a handshake with the client as if it were the entry point, and
with the entry point as if it were the client (classic MITM), terminating the
tunnel on both sides.

For the adaptive-routing product this is not theoretical: the system's whole
premise is steering players' traffic through third-party VPS hops. A
route-selection layer that switches between exit points **it cannot
authenticate is choosing which unauthenticated party to trust** (ARDP §4).

What exists today and must be preserved:

- `StatelessTokenManager` — HMAC-SHA256 address-validation cookies, wired and
  strict in `HandshakeFinish` (`endpoint.rs:89,398,496`). This validates the
  *address* (return routability), not the *identity*.
- `PathValidator` — two-way path challenge (reachability of a new path), not
  identity.
- The v1.1 verification round retracted the claim that the cookie manager is
  dead code; it is live. Identity is the gap, not address validation.

## 2. Option A — Pre-Shared Key (PSK)

The client and operator share a symmetric secret (per user, or per
deployment); the handshake mixes it in.

- **Mechanism:** HKDF-include the PSK into the key schedule
  (`derive_directional_session_keys` input), and add a PSK binder (HMAC over
  the transcript) to `ClientHello`; the server rejects unbindable hellos.
- **Pros:** ~30 lines in `gtp-crypto` + endpoint plumbing; no new dependencies
  (HMAC-SHA256 already in-tree); no public-key infrastructure; constant-time
  comparison already available (`subtle`);quantum-hybrid-ish posture for the
  symmetric part.
- **Cons:** secret distribution is an operational problem (one leak breaks all
  sessions using that key); no identity *names* — only membership; rotation is
  manual; per-user keys need a provisioning channel the platform does not have
  yet (S1's session management would carry it).

## 3. Option B — Static server signature (Ed25519)

The server holds a long-term Ed25519 key; the client holds its public key in
a pinned trust store. `ServerHello` carries the static public key **and a
signature over the transcript-so-far** (client ephemeral ‖ server ephemeral ‖
cookie); the client verifies before sending `HandshakeFinish`.

- **Mechanism:** new wire fields in `ServerHello` (varint pubkey +
  signature); `ed25519-dalek` dependency; trust store = file of pinned keys
  (config path); key-id byte for rotation.
- **Pros:** real identity with names; key rotation without re-provisioning
  clients (ship two key-ids); scales to many exits signed by one operator
  key; auditable; matches how QUIC/TLS think about server auth.
- **Cons:** new dependency; wire-format change to a handshake frame (versioned
  — GTP frames are TLV-typed, so an extended `ServerHello` is additive if a
  new frame type or a v2 hello is used; both ends must upgrade together);
  ~150–250 lines including tests; private keys on every exit VPS (mitigate:
  per-exit keys signed by an operator key — a mini-chain, still no external
  CA).

## 4. Recommendation

**Option B (Ed25519 static keys, operator-signed)** as the target, with
**Option A as an interim** only if an unauthenticated deployment has to ship
before B lands (it should not: B *is* the G7 gate work).

Rationale:

1. The threat model is *on-path attacker impersonating infrastructure* —
   exactly what signatures solve and PSKs only mitigate.
2. Exit-point fleets (C-3/C-4) rotate and grow; PSK re-distribution does not
   scale to that, per-operator signing does.
3. The `ed25519` dependency cost is bounded and isolated to `gtp-crypto`;
   the AAD/nonce invariants (INV-9/INV-10) are untouched — the signature
   rides inside the existing authenticated transcript, not beside it.
4. Client-side pinning (no external CA) keeps the trust model identical to
   the PSK case operationally: the operator still owns the root of trust.

## 5. Required decisions before implementation (ADR-006)

| # | Decision | Default proposal |
| :-- | :-- | :-- |
| 1 | Wire shape: extend `ServerHello` vs new `ServerHelloV2` frame | new frame type (additive; old endpoints keep working) |
| 2 | Key hierarchy: per-exit keys only vs operator-signed per-exit | operator-signed (rotation without client updates) |
| 3 | Trust store format + config path | JSON file of `{key_id, pubkey}` under a config dir; CLI flag to print fingerprints |
| 4 | Client identity (reverse auth) | out of scope for G7 — server auth first; client auth rides S1 sessions |
| 5 | Downgrade resistance | hello includes a version list; signature covers it |

## 6. Test plan sketch (when implemented)

- MITM: attacker completes both half-handshakes with different ephemerals ⟹
  signature verification fails on the client (adversarial, mirrors
  `test_active_mitm_key_tamper_rejected`).
- Rotation: two pinned key-ids; server rotates; old sessions complete, new
  hellos use the new key without a client update.
- Negative: tampered signature, wrong pinned key, missing key-id ⟹ typed
  error, no session state created (INV-3 discipline).
