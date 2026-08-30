# Comprehensive Technical Paper
## Adding GTP-rs to zgalaxy-rs with Direct-First and Relay Fallback

**Version:** 3.0
**Date:** 2026-08-28
**Scope:** `dreamzone-cc/zgalaxy-rs` + `dreamzone-cc/GTP-rs` + verifying the role of `dreamzone-cc/ZGALAXY`
**Goal:** add GTP as an additional mesh transport, keep QUIC, and adopt a direct-connection-first strategy with relay fallback when the direct path is unavailable.

---

# 1. Executive summary

The goal is not to replace QUIC, not to turn the external `ZGALAXY` into a relay, and not to put a relay inside GTP.

The correct goal is to build an independent **Path/Connection Management** layer inside `zgalaxy-rs` that makes the path decision separate from the protocol:

```text
                         zgalaxy-rs
                              │
                    ┌─────────┴─────────┐
                    │                   │
              Control Plane        Data Plane
                    │                   │
          EmbeddedController      PathManager
                    │                   │
                    │          ┌────────┴────────┐
                    │          │                 │
                    │       Direct             Relay
                    │          │                 │
                    │      ┌───┴───┐         ┌───┴───┐
                    │      │       │         │       │
                    │     QUIC    GTP       QUIC    GTP
                    │
                    └────────────────────────────────
```

The rule:

```text
1. Discover a peer
2. Try the Direct mesh
3. Use QUIC or GTP per policy/capability
4. If Direct fails → Relay
5. Keep probing Direct
6. When Direct succeeds again → migrate back
```

This makes:

```text
Path selection
```

independent of:

```text
Transport selection
```

— and that is the core architectural point of the new version.

---

# 2. Terminology corrections

The following must not be conflated:

## 2.1 `ZGALAXY`

The repository:

```text
dreamzone-cc/ZGALAXY
```

An independent project.

It must not be assumed to be the same as the Controller inside `zgalaxy-rs`.

---

## 2.2 `zgalaxy-rs`

The repository:

```text
dreamzone-cc/zgalaxy-rs
```

It is the client/agent, containing:

```text
Client
EmbeddedController
PeerManager
NAT
TUN
Mesh/QUIC transport
```

The requested GTP addition belongs to this project.

---

## 2.3 EmbeddedController

Lives in:

```text
zgalaxy-rs/src/controller.rs
```

An optional controller inside the same binary.

Its responsibilities include:

```text
network configuration
membership
authorization
IP assignment
member records
join handling
```

It must not automatically be treated as the relay data plane.

---

## 2.4 Relay

A relay is a fallback data path:

```text
Peer A → Relay → Peer B
```

and must be designed as an independent component.

---

# 3. The main architectural conclusion

We need to separate two decisions:

```text
Question 1:
How do I reach a peer?

Answer:
Direct or Relay

Question 2:
Which transport is used inside the path?

Answer:
QUIC or GTP
```

Therefore:

```rust
enum PathMode {
    Direct,
    Relay,
}

enum TransportKind {
    Quic,
    Gtp,
}
```

And the possible combinations:

```text
Direct + QUIC
Direct + GTP

Relay + QUIC
Relay + GTP
```

But the default policy:

```text
Direct first
Relay fallback
```

---

# 4. The target end-state shape

```text
                         ┌─────────────────────┐
                         │ External ZGALAXY    │
                         │ (independent repo)  │
                         └──────────┬──────────┘
                                    │
                              Control/API
                                    │
                                    ▼
┌──────────────────────────────────────────────────────────────────┐
│                         zgalaxy-rs Client                        │
│                                                                  │
│  ┌──────────────────────┐       ┌────────────────────────────┐  │
│  │ EmbeddedController   │       │ PathManager                │  │
│  │                      │       │                            │  │
│  │ membership           │       │ Direct discovery           │  │
│  │ authorization        │       │ NAT traversal              │  │
│  │ network config       │       │ path probing               │  │
│  │ IP assignment        │       │ relay fallback             │  │
│  └──────────┬───────────┘       │ migration back             │  │
│             │                   └────────────┬───────────────┘  │
│             │                                │                  │
│             └──────────────┬─────────────────┘                  │
│                            ▼                                    │
│                     MeshTransport                              │
│                            │                                    │
│                 ┌──────────┴──────────┐                         │
│                 │                     │                         │
│               QUIC                   GTP                       │
│                 │                     │                         │
│                 └──────────┬──────────┘                         │
│                            │                                    │
│                       UDP network                               │
└────────────────────────────┼────────────────────────────────────┘
                             │
                  Direct or Relay path
```

---

# 5. What does the current code prove?

The previous review of the core files indicates that:

- `src/quic.rs` contains the transport, events, and control semantics.
- `src/main.rs` handles QUIC events and control messages.
- `src/controller.rs` contains the EmbeddedController.
- `src/nat.rs` contains coupling with the transport/QUIC.
- `src/peer.rs` manages peer/path state.
- `src/transport.rs` is a distinct UDP wire transport, separate from QUIC.
- `GTP-rs` provides transport capabilities such as reliability modes, loss recovery, congestion control, AEAD, path validation/migration, and Tokio integration.

Direct references are listed in the final section.

---

# 6. The problem with the current architecture

The problem is not that QUIC is bad.

The problem is that some ZGalaxy semantics are directly coupled to QUIC.

The current conceptual shape:

```text
QuicEvent
   │
   ▼
main.rs
   │
   ├── identity
   ├── controller
   ├── network
   ├── peer
   ├── ping/pong
   └── data
```

This makes adding GTP difficult.

The required shape:

```text
QuicEvent ──┐
            │
GtpEvent ───┤
            ▼
    TransportEvent
            │
            ▼
      Mesh/Path Engine
            │
      ┌─────┴─────┐
      │           │
     Data       Control
      │           │
     TUN      ControlEngine
                  │
           EmbeddedController
```

---

# 7. Separating the control plane from the transport

We must move:

```text
ControlMessage
```

from:

```text
src/quic.rs
```

to something like:

```text
src/control.rs
```

because the messages are not QUIC-specific.

For example:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
AnnounceAccepted
NetworkConfigRequest
NetworkConfigResponse
Ping
Pong
```

These are ZGalaxy semantics.

---

# 8. MeshTransport

An abstraction must be defined based on the needs of `zgalaxy-rs`, not on the shape of QUIC.

Example:

```rust
#[async_trait::async_trait]
pub trait MeshTransport: Send + Sync {
    async fn start(&self) -> anyhow::Result<()>;

    async fn connect(
        &self,
        peer: PeerId,
        endpoint: SocketAddr,
    ) -> anyhow::Result<()>;

    async fn send_frame(
        &self,
        peer: PeerId,
        frame: Bytes,
    ) -> anyhow::Result<()>;

    async fn send_control(
        &self,
        peer: PeerId,
        message: ControlMessage,
    ) -> anyhow::Result<()>;

    async fn close(
        &self,
        peer: PeerId,
    ) -> anyhow::Result<()>;
}
```

But the final API must be pinned down after reviewing the current GTP-rs during implementation.

---

# 9. TransportEvent

The proposal:

```rust
pub enum TransportEvent {
    Connected {
        peer: PeerId,
    },

    Disconnected {
        peer: PeerId,
    },

    Frame {
        peer: PeerId,
        data: Bytes,
    },

    Control {
        peer: PeerId,
        message: ControlMessage,
    },

    PathChanged {
        peer: PeerId,
        path: PathInfo,
    },
}
```

With that:

```text
QUIC → TransportEvent
GTP  → TransportEvent
```

and the core does not need to know the source.

---

# 10. PathManager

This layer is the most important addition together with GTP/Relay.

The proposal:

```rust
pub struct PathManager {
    direct: DirectPathManager,
    relay: RelayPathManager,
    policy: PathPolicy,
}
```

Its responsibilities:

```text
peer discovery
candidate management
NAT traversal
direct connection attempts
direct health
relay selection
relay session
fallback
recovery
migration
```

---

# 11. The state machine

The fallback process must not be:

```rust
if !connected {
    relay();
}
```

but a clear state machine:

```text
             ┌──────────────┐
             │   Discovered │
             └──────┬───────┘
                    │
                    ▼
             ┌──────────────┐
             │ DirectProbe  │
             └──────┬───────┘
                    │
          ┌─────────┴─────────┐
          │                   │
       success              timeout
          │                   │
          ▼                   ▼
      ┌────────┐         ┌──────────┐
      │ Direct │         │  Relay   │
      └───┬────┘         └────┬─────┘
          │                   │
          │ periodic probe    │
          └─────────┬─────────┘
                    ▼
             Direct available
                    │
                    ▼
               migrate back
```

---

# 12. The direct-first policy

The default:

```rust
PathPolicy {
    prefer_direct: true,
    relay_on_failure: true,
    retry_direct: true,
}
```

Relay must not be engaged before direct has been given a fair chance.

---

# 13. How do we determine direct failure?

It is not enough that:

```text
a TCP-like connection failed
```

We need:

```text
candidate timeout
handshake timeout
path challenge failure
no packets received
repeated loss
NAT mapping failure
```

And the following must be defined, configurably:

```text
initial timeout
retry count
backoff
relay threshold
```

---

# 14. Relay does not cancel Direct

When switching to relay:

```text
Direct = failed/currently unavailable
Relay = active
```

but the following remains on:

```text
Direct probing = enabled
```

For example:

```text
every 10-30 seconds
```

or adaptive probing.

---

# 15. Returning to Direct

When direct becomes available:

```text
Relay
  │
  │ direct probe success
  ▼
Direct validation
  │
  ▼
Switch traffic
  │
  ▼
Drain relay
  │
  ▼
Close relay
```

Packet loss must be avoided as much as possible.

---

# 16. Relay architecture

The relay must be a server-side component.

The shape:

```text
Peer A
  │
  │ encrypted transport
  ▼
┌─────────────────┐
│ Relay Server    │
│                 │
│ Session table   │
│ Peer routing    │
│ Authentication  │
└────────┬────────┘
         │
         │ encrypted transport
         ▼
      Peer B
```

The relay does not decrypt the ZGalaxy payload.

---

# 17. Relay routing

The relay must have:

```text
PeerID → Session
```

For example:

```rust
HashMap<PeerId, RelaySession>
```

And each session knows:

```text
peer identity
authenticated state
transport kind
endpoint
last_seen
bytes
```

---

# 18. Relay authentication

The relay must not be open:

```text
UDP packet → forward
```

but:

```text
connect
  ↓
authenticate
  ↓
authorize
  ↓
bind PeerID
  ↓
allow relay
```

And authorization must be tied to the Controller/network membership.

---

# 19. The relationship between the EmbeddedController and the relay

The best shape:

```text
EmbeddedController
       │
       ├── authenticates Peer
       ├── knows membership
       └── provides network configuration
                    │
                    ▼
              RelayService
                    │
                    └── forwards traffic
```

But it must not become:

```text
EmbeddedController
      =
Relay
```

because the lifecycle and responsibilities differ.

---

# 20. If the relay lives inside the same zgalaxy-rs binary

One can support:

```text
zgalaxy-rs --controller
```

containing:

```text
EmbeddedController
RelayService
Controller API
```

but they remain separate modules:

```text
controller.rs
relay.rs
```

---

# 21. External ZGALAXY

`ZGALAXY` must not be modified just because we added GTP.

However, if the requirement is for the **relay service to be owned and operated by the external ZGALAXY system**, its current interfaces must be examined precisely:

```text
controller API
authentication
node registration
network membership
relay discovery
```

And the first phase must treat the relay endpoint as an independent service, until the existence of a relay protocol in `ZGALAXY` is proven.

---

# 22. GTP integration

GTP must be a transport backend:

```text
MeshTransport
     │
     ├── QuicTransport
     └── GtpTransport
```

And it must not be:

```text
GTP
 ├── Controller
 ├── NAT
 └── Relay
```

---

# 23. The GTP data plane

The recommendation:

```text
ZGalaxy frame
      ↓
GTP Unreliable
      ↓
Peer
```

because the current QUIC-based data plane uses datagram semantics.

We do not want to turn all L2/L3 traffic into reliable ordered traffic.

---

# 24. The GTP control plane

The recommendation:

```text
ControlMessage
      ↓
GTP ReliableOrdered
```

for:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
```

Ping/Pong can be high-priority control messages.

---

# 25. GTP relay

When direct:

```text
Peer A
   │
   │ GTP
   ▼
Peer B
```

When relaying:

```text
Peer A
   │
   │ GTP
   ▼
Relay
   │
   │ GTP
   ▼
Peer B
```

The relay does not need to change GTP semantics.

---

# 26. An important option: relay over GTP

The relay should preferably be a plain forwarding endpoint:

```text
GTP connection A
       │
       ▼
Relay routing
       │
       ▼
GTP connection B
```

End-to-end encryption between the peers is preserved.

---

# 27. We do not use GTP encryption for peer identity in place of ZGalaxy identity

GTP security:

```text
transport security
```

ZGalaxy identity:

```text
Ed25519 node identity
NodeAnnounce
NodeChallenge
AnnounceProof
```

They remain separate.

---

# 28. The identity handshake over Direct and Relay

The same protocol:

```text
Direct:
GTP → NodeAnnounce → Challenge → Proof

Relay:
GTP → Relay → NodeAnnounce → Challenge → Proof
```

I.e., the relay does not change identity semantics.

---

# 29. Controller mode over the relay

This must be tested.

Example:

```text
Controller Node
controller_enabled=true
        │
        │ Relay
        ▼
Client
```

The following must work:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
```

in the same way as direct.

---

# 30. NAT architecture

The NAT layer must be:

```text
NAT
 │
 ├── candidate discovery
 ├── direct probe
 └── path status
```

and not:

```text
NAT → QUIC only
```

---

# 31. GTP path capabilities

The GTP-rs capabilities related to:

```text
path validation
NAT rebinding
path migration
PMTU
loss recovery
congestion control
RTT
```

must be leveraged instead of reimplemented in zgalaxy-rs.

But the following must be kept separate:

```text
GTP path state
```

from:

```text
ZGalaxy Peer path state
```

---

# 32. PeerManager

`PeerManager` must retain the concepts of:

```text
peer
paths
latency
endpoint
status
```

and must not become responsible for:

```text
GTP implementation
QUIC implementation
Relay implementation
```

That is the PathManager's responsibility.

---

# 33. The Path object

The proposal:

```rust
pub struct PathInfo {
    pub peer: PeerId,
    pub mode: PathMode,
    pub transport: TransportKind,
    pub endpoint: SocketAddr,
    pub latency: Duration,
    pub healthy: bool,
    pub last_seen: Instant,
}
```

This allows representing:

```text
Peer A
 ├── Direct/QUIC
 ├── Direct/GTP
 └── Relay/GTP
```

---

# 34. Transport capabilities

Each transport must offer:

```rust
pub struct TransportCapabilities {
    pub unreliable: bool,
    pub reliable_ordered: bool,
    pub path_migration: bool,
    pub max_payload: usize,
}
```

GTP and QUIC can differ.

---

# 35. MTU

Do not use the current QUIC value as a global constant.

Instead of:

```text
if quic:
    1186
```

it must become:

```text
TransportCapabilities.max_payload
```

because GTP has a different overhead and a different PMTU.

---

# 36. Relay MTU

The relay adds extra overhead.

Therefore:

```text
Peer MTU
    ↓
Transport MTU
    ↓
Relay overhead
    ↓
Maximum payload
```

And the fragmentation/segmentation semantics must be clear.

---

# 37. Reliability mapping

The proposal:

| ZGalaxy traffic | Direct QUIC | Direct GTP | Relay QUIC | Relay GTP |
|---|---|---|---|---|
| L2/L3 frame | Datagram | Unreliable | Datagram | Unreliable |
| Identity control | Stream | ReliableOrdered | Stream | ReliableOrdered |
| Network config | Stream | ReliableOrdered | Stream | ReliableOrdered |
| Ping/Pong | Control | High priority | Control | High priority |

---

# 38. Relay transport selection

It should not be:

```text
if relay:
    use QUIC
```

but:

```text
select path
select transport
```

For example:

```rust
PathSelection {
    mode: Relay,
    transport: Gtp,
}
```

---

# 39. Configuration

The proposal:

```toml
[transport]
mode = "quic"
```

Values:

```text
quic
gtp
```

Then:

```toml
[path]
prefer_direct = true
relay_enabled = true
direct_retry = true
```

Then:

```toml
[relay]
enabled = true
endpoint = "..."
```

The endpoint must not be hardcoded.

---

# 40. An auto mode in the future

After the MVP:

```toml
[transport]
mode = "auto"
```

then:

```text
Peer capabilities
      ↓
Direct transport selection
      ↓
GTP preferred
      ↓
QUIC fallback
```

But this must not precede the success of basic GTP/QUIC operation.

---

# 41. Per-peer transport in the future

One can support:

```text
Peer A → GTP
Peer B → QUIC
Peer C → GTP
```

and then:

```text
Peer A → Direct GTP
Peer B → Relay QUIC
Peer C → Direct QUIC
```

This is an additional reason to separate the PathManager from the transport.

---

# 42. Path scoring

A score can be built:

```text
direct + low latency = high score
direct + high loss = lower score
relay + stable = fallback score
```

For example:

```text
Direct healthy
    score = 100

Direct degraded
    score = 50

Relay
    score = 20
```

And the relay is used only when direct is not viable.

---

# 43. Hysteresis

The following must be prevented:

```text
Direct
Relay
Direct
Relay
...
```

due to jitter.

Use:

```text
failure threshold
success threshold
cooldown
```

For example:

```text
3 consecutive direct failures
→ relay

5 successful direct probes
→ direct
```

The final numbers need benchmarking.

---

# 44. The relay session lifecycle

```text
Create
  ↓
Authenticate
  ↓
Bind peer
  ↓
Discover peer relay session
  ↓
Establish relay path
  ↓
Forward
  ↓
Probe direct
  ↓
Direct recovered
  ↓
Drain
  ↓
Close
```

---

# 45. Relay discovery

There are several options:

### A. The controller provides the relay endpoint

```text
NetworkConfigResponse
      +
RelayEndpoint
```

### B. The client has a fixed relay endpoint

```text
relay.endpoint
```

### C. The external ZGALAXY offers a relay discovery API

This needs a separate examination and implementation.

The most suitable for the MVP:

```text
configured relay endpoint
```

then, later, controller-managed relay discovery.

---

# 46. Relay authorization

The relay must be able to know:

```text
PeerID
NetworkID
membership
token/credential
```

but it does not need to read the data payload.

---

# 47. E2E security

The goal:

```text
Peer A
  |
  | encrypted
  ▼
Relay
  |
  | same encrypted payload
  ▼
Peer B
```

The relay sees only the necessary metadata:

```text
source session
destination session
packet size
timing
```

and not the plaintext payload.

---

# 48. DoS protection

The relay must enforce:

```text
max connections
authentication rate limit
per-peer bandwidth
session timeout
packet rate limit
max payload
```

and must not allow:

```text
unauthenticated arbitrary forwarding
```

---

# 49. Relay observability

The following must be recorded:

```text
relay sessions
active peers
bytes in/out
packets
drops
reason for fallback
direct recovery
```

but plaintext packets must not be logged.

---

# 50. The most important client-side metrics

The following must be added:

```text
direct_attempts
direct_success
direct_failures
relay_activations
relay_duration
direct_recoveries
path_migrations
transport_failures
```

---

# 51. The API/debug endpoint

It is preferable to add a state for:

```text
/peer
```

or an internal endpoint showing:

```json
{
  "peer": "...",
  "path": "direct",
  "transport": "gtp",
  "healthy": true,
  "latency_ms": 12
}
```

And under relay:

```json
{
  "peer": "...",
  "path": "relay",
  "transport": "gtp",
  "healthy": true
}
```

while preserving backward compatibility with the current API.

---

# 52. The main.rs refactoring

The end goal:

```rust
let transport = build_transport(config).await?;
let path_manager = PathManager::new(...);
let control_engine = ControlEngine::new(...);
```

then a generic event loop:

```rust
while let Some(event) = transport.next_event().await {
    path_manager.handle(event).await?;
}
```

And `main.rs` contains no QUIC-specific controller handling.

---

# 53. The proposed files

```text
src/
├── controller.rs
├── controller_api.rs
├── control.rs
├── control_engine.rs
│
├── path/
│   ├── mod.rs
│   ├── manager.rs
│   ├── direct.rs
│   ├── relay.rs
│   ├── policy.rs
│   └── state.rs
│
├── transport/
│   ├── mod.rs
│   ├── traits.rs
│   ├── events.rs
│   ├── capabilities.rs
│   ├── quic.rs
│   └── gtp.rs
│
├── quic.rs              # transitional adapter
├── transport.rs         # legacy UDP wire
├── nat.rs
├── peer.rs
├── network.rs
├── identity.rs
├── crypto.rs
├── packet.rs
└── main.rs
```

---

# 54. Do not delete `src/transport.rs`

A distinction must be made between:

```text
src/transport.rs
```

and:

```text
src/transport/gtp.rs
```

The former is the legacy/native UDP wire transport.

The latter is the GTP mesh transport.

The files can later be renamed to reduce confusion, but a large rename should not be performed simultaneously with the GTP work.

---

# 55. Implementation phases

## Phase 0 — Repository audit

Examine:

```text
ZGALAXY
zgalaxy-rs
GTP-rs
```

documenting:

```text
control
identity
NAT
peer
transport
relay
API
```

---

## Phase 1 — Control extraction

Move:

```text
ControlMessage
```

into an independent module.

---

## Phase 2 — Transport abstraction

Create:

```text
MeshTransport
TransportEvent
TransportCapabilities
```

---

## Phase 3 — The QUIC adapter

Make QUIC work through the abstraction without a behavior change.

This is a mandatory phase before GTP.

---

## Phase 4 — ControlEngine

Move:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
Ping/Pong
```

out of `main.rs`.

---

## Phase 5 — PathManager

Add:

```text
DirectPath
RelayPath
PathState
PathPolicy
```

Initially a mock/in-process relay can be used for testing.

---

## Phase 6 — NAT decoupling

Make NAT deal with:

```text
PathManager
```

rather than QUIC directly.

---

## Phase 7 — GTP-rs

Add the dependency and test:

```text
runtime
endpoint
connection
unreliable
reliable
path APIs
```

Do not rely on an assumed API.

---

## Phase 8 — Direct GTP

Implement:

```text
Peer A → GTP → Peer B
```

without a relay first.

The following must be proven:

```text
identity
control
TUN
peer state
NAT
```

---

## Phase 9 — GTP controller mode

Test:

```text
GTP Client
     ↓
GTP
     ↓
zgalaxy-rs EmbeddedController
```

with:

```text
NetworkConfigRequest
```

---

## Phase 10 — The relay server

Add:

```text
RelayService
```

either inside `zgalaxy-rs` controller mode or as a standalone binary, per deployment requirements.

---

## Phase 11 — Relay over QUIC

Prove:

```text
Peer A → QUIC Relay → Peer B
```

---

## Phase 12 — Relay over GTP

Then:

```text
Peer A → GTP Relay → Peer B
```

---

## Phase 13 — Automatic fallback

Implement:

```text
Direct first
→ timeout
→ Relay
```

---

## Phase 14 — Recovery

Implement:

```text
Relay
→ direct probe
→ direct recovered
→ migrate
→ close relay
```

---

# 56. The core tests

## Test 1 — Direct QUIC

```text
A ───────── QUIC ───────── B
```

---

## Test 2 — Direct GTP

```text
A ───────── GTP ───────── B
```

---

## Test 3 — Relay QUIC

```text
A ─── QUIC ─── Relay ─── QUIC ─── B
```

---

## Test 4 — Relay GTP

```text
A ─── GTP ─── Relay ─── GTP ─── B
```

---

## Test 5 — Direct failure

```text
A ─── X ─── B

      ↓

A ─── Relay ─── B
```

---

## Test 6 — Direct recovery

```text
Relay active
    ↓
Direct becomes available
    ↓
switch
    ↓
relay closed
```

---

## Test 7 — Controller over Direct

```text
Client → Direct → EmbeddedController
```

---

## Test 8 — Controller over Relay

```text
Client → Relay → EmbeddedController
```

---

# 57. Network fault injection

Test:

```text
packet loss:
1%, 5%, 10%, 20%

latency:
20ms, 50ms, 100ms, 200ms

jitter

reordering

NAT mapping changes

endpoint changes

temporary firewall block
```

---

# 58. GTP benchmarks

Compare:

```text
QUIC Direct
GTP Direct
QUIC Relay
GTP Relay
```

Metrics:

```text
throughput
p50 latency
p95 latency
p99 latency
CPU
RAM
packets/sec
loss recovery
connection setup
fallback time
recovery time
```

---

# 59. Security tests

Test:

```text
invalid NodeAnnounce
invalid signature
wrong node address
replayed proof
unauthorized network
unauthorized relay
relay without authentication
oversized payload
malformed GTP
GTP replay
connection flood
```

The expected result:

```text
reject
no panic
no unbounded memory
no authorization bypass
```

---

# 60. The migration strategy

Do not bundle every change into one commit.

The proposal:

```text
1. ControlMessage extraction
2. TransportEvent
3. MeshTransport
4. QUIC adapter
5. ControlEngine
6. NAT decoupling
7. PathManager
8. Relay abstraction
9. GTP dependency
10. Direct GTP
11. GTP controller mode
12. Relay server
13. GTP relay
14. Automatic fallback
15. Recovery/migration
16. Tests/benchmarks
```

---

# 61. A key decision: the relay is not a third transport

We do not want:

```text
QUIC
GTP
Relay
```

as three equal transports.

The correct design:

```text
Path
├── Direct
│   └── Transport
│       ├── QUIC
│       └── GTP
│
└── Relay
    └── Transport
        ├── QUIC
        └── GTP
```

This removes the confusion entirely.

---

# 62. A key decision: the controller is not the relay

The design:

```text
EmbeddedController
    =
control/management

RelayService
    =
data forwarding
```

They can run in the same process, but they are separate modules.

---

# 63. A key decision: GTP is not responsible for fallback

Not:

```text
GtpTransport
   ↓
if failed
   ↓
Relay
```

but:

```text
PathManager
   ↓
Direct GTP failed
   ↓
choose Relay
   ↓
Relay GTP
```

and likewise:

```text
Direct QUIC failed
   ↓
Relay QUIC
```

---

# 64. A key decision: Direct is the normal state

The relay must be:

```text
fallback
```

and not:

```text
default topology
```

for the reasons:

```text
latency
bandwidth
server cost
scalability
privacy
```

---

# 65. A key decision: the relay does not decrypt

The goal:

```text
E2E:
Peer A ===================== Peer B
             encrypted
                ↓
             Relay
             opaque
```

This reduces the trust requirements on the relay.

---

# 66. GTP and the relay: the best shape

The end goal:

```text
                    PathManager
                         │
            ┌────────────┴────────────┐
            │                         │
          Direct                    Relay
            │                         │
       ┌────┴────┐              ┌─────┴─────┐
       │         │              │           │
      QUIC      GTP            QUIC        GTP
       │         │              │           │
       └────┬────┘              └─────┬─────┘
            │                         │
            ▼                         ▼
          Peer B                    Relay
```

With this, one can later add:

```text
WebSocket relay
TCP relay
another transport
```

without redesigning the client.

---

# 67. Do we need to modify ZGALAXY?

Not as a condition for adding GTP to `zgalaxy-rs`.

But if the requirement is:

```text
ZGALAXY external service
    ↓
Relay discovery
    ↓
Relay allocation
```

then that is a separate integration project.

The actual ZGALAXY API and structure must first be examined, proving the presence or absence of:

```text
relay allocation
relay registry
peer rendezvous
relay authentication
```

Their existence must not be assumed from the project name alone.

---

# 68. The final architectural decision

The design to adopt:

```text
                          ┌──────────────────┐
                          │  ZGALAXY         │
                          │  external repo   │
                          └────────┬─────────┘
                                   │
                              management
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────┐
│                         zgalaxy-rs                             │
│                                                                │
│  ┌──────────────────┐       ┌──────────────────────────────┐  │
│  │ EmbeddedController│       │ PathManager                  │  │
│  │                  │       │                              │  │
│  │ membership       │       │ Direct-first                 │  │
│  │ authorization    │       │ NAT traversal                │  │
│  │ network config   │       │ fallback                     │  │
│  └────────┬─────────┘       │ recovery                     │  │
│           │                 └──────────────┬───────────────┘  │
│           │                                │                  │
│           └───────────────┬────────────────┘                  │
│                           ▼                                   │
│                    MeshTransport                              │
│                           │                                   │
│              ┌────────────┴────────────┐                      │
│              │                         │                      │
│            QUIC                       GTP                     │
│              │                         │                      │
│              └────────────┬────────────┘                      │
│                           │                                   │
│                     Path selected                             │
│                           │                                   │
│                    ┌──────┴──────┐                            │
│                    │             │                            │
│                 Direct         Relay                          │
└────────────────────┼─────────────┼────────────────────────────┘
                     │             │
                     ▼             ▼
                   Peer B        Relay
```

---

# 69. The executive conclusion

The required project is not:

```text
"adding GTP to QUIC"
```

nor:

```text
"replacing QUIC with GTP"
```

but:

```text
Re-separating the zgalaxy-rs architecture
so that transport/path becomes independent of ZGalaxy semantics.
```

Then:

```text
QUIC ─────────┐
              ├── MeshTransport
GTP ──────────┘
                    │
                    ▼
               PathManager
                    │
          ┌─────────┴─────────┐
          │                   │
       Direct               Relay
       (first)             (fallback)
```

Thus the final scenarios become:

### The normal state

```text
Peer A
  │
  │ Direct GTP
  ▼
Peer B
```

### If direct GTP fails

```text
Peer A
  │
  │ GTP
  ▼
Relay
  │
  │ GTP
  ▼
Peer B
```

### If GTP is unavailable but QUIC is available

```text
Peer A
  │
  │ Direct QUIC
  ▼
Peer B
```

### If Direct fails entirely

```text
Peer A
  │
  │ QUIC/GTP
  ▼
Relay
  │
  │ QUIC/GTP
  ▼
Peer B
```

### And if Direct returns

```text
Relay
  │
  ▼
Direct probe succeeds
  │
  ▼
Traffic migrates to Direct
  │
  ▼
Relay session closes
```

**This is the structure I recommend adopting as the official architectural goal of the project.**

---

# 70. Sources

## `zgalaxy-rs`

- Repository:
  https://github.com/dreamzone-cc/zgalaxy-rs

- Architecture:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/ARCHITECTURE.md

- Embedded Controller:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/controller.rs

- Main daemon/event loop:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/main.rs

- QUIC implementation:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/quic.rs

- NAT:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/nat.rs

- Peer Manager:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/peer.rs

- Legacy UDP transport:
  https://raw.githubusercontent.com/dreamzone-cc/zgalaxy-rs/main/src/transport.rs

---

## `GTP-rs`

- Repository:
  https://github.com/dreamzone-cc/GTP-rs

- README:
  https://raw.githubusercontent.com/dreamzone-cc/GTP-rs/main/README.md

References used to understand delivery modes, loss recovery, congestion control, priority scheduling, AEAD, path validation, NAT rebinding/path migration, and Tokio integration.

---

## `ZGALAXY`

- Repository:
  https://github.com/dreamzone-cc/ZGALAXY

This repository is separate from `zgalaxy-rs`. The existence of `ZGALAXY` must not be taken to mean it contains a relay data plane until that is proven from the actual code/API.

---

## Additional design references

Modern relay/P2P models were also used to validate the principle:

```text
Direct preferred → Relay fallback
```

including NetBird, which defines a relay client managing connections to peers via a relay, and P2P projects that use direct-first then relay fallback. These references are **not a source for the characteristics of ZGALAXY itself**, but a reference for designing the fallback pattern.
