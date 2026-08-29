# ورقة تقنية شاملة
## إضافة GTP-rs إلى zgalaxy-rs مع Direct-First وRelay Fallback

**الإصدار:** 3.0  
**التاريخ:** 2026-08-28  
**النطاق:** `dreamzone-cc/zgalaxy-rs` + `dreamzone-cc/GTP-rs` + التحقق من دور `dreamzone-cc/ZGALAXY`  
**الهدف:** إضافة GTP كـ Mesh Transport إضافي، مع إبقاء QUIC، واعتماد الاتصال المباشر أولاً ثم Relay عند تعذر المسار المباشر.

---

# 1. الملخص التنفيذي

الهدف ليس استبدال QUIC، وليس تحويل `ZGALAXY` الخارجي إلى Relay، وليس وضع Relay داخل GTP.

الهدف الصحيح هو بناء طبقة **Path/Connection Management** مستقلة داخل `zgalaxy-rs` تجعل قرار المسار منفصلاً عن البروتوكول:

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

القاعدة:

```text
1. اكتشف Peer
2. حاول Direct Mesh
3. استخدم QUIC أو GTP وفق policy/capability
4. إذا فشل Direct → Relay
5. استمر في اختبار Direct
6. عند نجاح Direct مرة أخرى → migrate back
```

هذا يجعل:

```text
Path selection
```

مستقلاً عن:

```text
Transport selection
```

وهذه هي النقطة المعمارية الأساسية في الإصدار الجديد.

---

# 2. تصحيح المصطلحات

يجب عدم الخلط بين:

## 2.1 `ZGALAXY`

المستودع:

```text
dreamzone-cc/ZGALAXY
```

مشروع مستقل.

لا ينبغي افتراض أنه هو نفسه الـ Controller الموجود داخل `zgalaxy-rs`.

---

## 2.2 `zgalaxy-rs`

المستودع:

```text
dreamzone-cc/zgalaxy-rs
```

وهو الـ Client/Agent، ويحتوي داخله على:

```text
Client
EmbeddedController
PeerManager
NAT
TUN
Mesh/QUIC transport
```

إضافة GTP المطلوبة هي إلى هذا المشروع.

---

## 2.3 EmbeddedController

داخل:

```text
zgalaxy-rs/src/controller.rs
```

وهو Controller اختياري داخل نفس الـ binary.

وظائفه تشمل:

```text
network configuration
membership
authorization
IP assignment
member records
join handling
```

ولا ينبغي أن نعتبره Relay Data Plane تلقائياً.

---

## 2.4 Relay

Relay هو مسار بيانات احتياطي:

```text
Peer A → Relay → Peer B
```

ويجب تصميمه كـ component مستقل.

---

# 3. الاستنتاج المعماري الرئيسي

نحتاج إلى فصل قرارين:

```text
Question 1:
كيف أصل إلى Peer؟

Answer:
Direct أو Relay

Question 2:
ما transport المستخدم داخل المسار؟

Answer:
QUIC أو GTP
```

لذلك:

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

والتركيبات الممكنة:

```text
Direct + QUIC
Direct + GTP

Relay + QUIC
Relay + GTP
```

لكن الـ policy الافتراضية:

```text
Direct first
Relay fallback
```

---

# 4. الشكل النهائي المستهدف

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

# 5. ما الذي يثبت من الكود الحالي؟

المراجعة السابقة للملفات الأساسية تشير إلى أن:

- `src/quic.rs` يحتوي transport وevents وcontrol semantics.
- `src/main.rs` يتعامل مع أحداث QUIC وControl messages.
- `src/controller.rs` يحتوي EmbeddedController.
- `src/nat.rs` يحتوي coupling مع transport/QUIC.
- `src/peer.rs` يدير peer/path state.
- `src/transport.rs` يمثل UDP wire transport مختلفاً عن QUIC.
- `GTP-rs` يوفر transport capabilities مثل reliability modes، loss recovery، congestion control، AEAD، path validation/migration وTokio integration.

المراجع المباشرة مدرجة في القسم الأخير.

---

# 6. مشكلة architecture الحالية

المشكلة ليست أن QUIC سيئ.

المشكلة أن بعض ZGalaxy semantics مرتبطة مباشرة بـ QUIC.

الشكل الحالي المفاهيمي:

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

هذا يجعل إضافة GTP صعبة.

الشكل المطلوب:

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

# 7. فصل Control Plane عن Transport

يجب نقل:

```text
ControlMessage
```

من:

```text
src/quic.rs
```

إلى شيء مثل:

```text
src/control.rs
```

لأن الرسائل ليست QUIC-specific.

مثل:

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

هذه ZGalaxy semantics.

---

# 8. MeshTransport

يجب تعريف abstraction مبنية على احتياجات `zgalaxy-rs` وليس على شكل QUIC.

مثال:

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

لكن يجب تثبيت API النهائي بعد مراجعة GTP-rs الحالي أثناء التنفيذ.

---

# 9. TransportEvent

المقترح:

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

وبذلك:

```text
QUIC → TransportEvent
GTP  → TransportEvent
```

ولا يحتاج core إلى معرفة المصدر.

---

# 10. PathManager

هذه الطبقة هي أهم إضافة مع GTP/Relay.

المقترح:

```rust
pub struct PathManager {
    direct: DirectPathManager,
    relay: RelayPathManager,
    policy: PathPolicy,
}
```

مسؤولياتها:

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

# 11. State machine

يجب ألا تكون عملية fallback عبارة عن:

```rust
if !connected {
    relay();
}
```

بل state machine واضحة:

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

# 12. Direct-first policy

الـ default:

```rust
PathPolicy {
    prefer_direct: true,
    relay_on_failure: true,
    retry_direct: true,
}
```

ولا يجب تشغيل Relay قبل إعطاء direct فرصة مناسبة.

---

# 13. كيف نحدد فشل Direct؟

لا يكفي:

```text
TCP-like connection failed
```

نحتاج:

```text
candidate timeout
handshake timeout
path challenge failure
no packets received
repeated loss
NAT mapping failure
```

ويجب تحديد:

```text
initial timeout
retry count
backoff
relay threshold
```

بشكل configurable.

---

# 14. Relay لا يلغي Direct

عند الانتقال إلى Relay:

```text
Direct = failed/currently unavailable
Relay = active
```

لكن يبقى:

```text
Direct probing = enabled
```

مثلاً:

```text
كل 10-30 ثانية
```

أو adaptive probing.

---

# 15. العودة إلى Direct

عندما يصبح direct متاحاً:

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

ويجب تجنب packet loss قدر الإمكان.

---

# 16. Relay architecture

Relay يجب أن يكون server-side component.

الشكل:

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

Relay لا يفك تشفير ZGalaxy payload.

---

# 17. Relay routing

يجب أن يكون لدى Relay:

```text
PeerID → Session
```

مثلاً:

```rust
HashMap<PeerId, RelaySession>
```

وكل session تعرف:

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

لا يجب أن يكون Relay مفتوحاً:

```text
UDP packet → forward
```

بل:

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

ويجب أن تكون authorization مرتبطة بـ Controller/network membership.

---

# 19. العلاقة بين EmbeddedController وRelay

الأفضل:

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

لكن لا يجب أن يصبح:

```text
EmbeddedController
      =
Relay
```

لأن lifecycle والـ responsibilities مختلفة.

---

# 20. إذا كان Relay داخل نفس zgalaxy-rs binary

يمكن دعم:

```text
zgalaxy-rs --controller
```

ليحتوي:

```text
EmbeddedController
RelayService
Controller API
```

لكنها تبقى modules منفصلة:

```text
controller.rs
relay.rs
```

---

# 21. External ZGALAXY

يجب عدم تعديل `ZGALAXY` فقط لأننا أضفنا GTP.

لكن إذا كان المطلوب أن يكون **Relay service مملوكاً ومُداراً بواسطة منظومة ZGALAXY الخارجية**، فهناك حاجة لفحص واجهاتها الحالية بدقة:

```text
controller API
authentication
node registration
network membership
relay discovery
```

والمرحلة الأولى يجب أن تعتبر Relay endpoint خدمة مستقلة، إلى أن يتم إثبات وجود Relay protocol في `ZGALAXY`.

---

# 22. GTP integration

GTP يجب أن يكون transport backend:

```text
MeshTransport
     │
     ├── QuicTransport
     └── GtpTransport
```

ولا يجب أن يكون:

```text
GTP
 ├── Controller
 ├── NAT
 └── Relay
```

---

# 23. GTP Data Plane

التوصية:

```text
ZGalaxy frame
      ↓
GTP Unreliable
      ↓
Peer
```

لأن data plane الحالي المبني على QUIC يستخدم datagram semantics.

لا نريد تحويل كل L2/L3 traffic إلى reliable ordered traffic.

---

# 24. GTP Control Plane

التوصية:

```text
ControlMessage
      ↓
GTP ReliableOrdered
```

لـ:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
```

أما Ping/Pong فيمكن أن تكون control messages ذات priority مرتفعة.

---

# 25. GTP Relay

عند Direct:

```text
Peer A
   │
   │ GTP
   ▼
Peer B
```

عند Relay:

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

Relay لا يحتاج إلى تغيير GTP semantics.

---

# 26. خيار مهم: Relay عبر GTP

يفضل أن يكون Relay مجرد forwarding endpoint:

```text
GTP connection A
       │
       ▼
Relay routing
       │
       ▼
GTP connection B
```

ويظل end-to-end encryption بين peers.

---

# 27. لا نستخدم GTP لتشفير Peer Identity بدلاً من ZGalaxy identity

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

تبقى منفصلة.

---

# 28. Identity handshake فوق Direct وRelay

نفس protocol:

```text
Direct:
GTP → NodeAnnounce → Challenge → Proof

Relay:
GTP → Relay → NodeAnnounce → Challenge → Proof
```

أي أن Relay لا يغير identity semantics.

---

# 29. Controller mode فوق Relay

هذا يجب اختباره.

مثال:

```text
Controller Node
controller_enabled=true
        │
        │ Relay
        ▼
Client
```

يجب أن يعمل:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
```

بنفس الطريقة التي يعمل بها direct.

---

# 30. NAT architecture

NAT layer يجب أن يكون:

```text
NAT
 │
 ├── candidate discovery
 ├── direct probe
 └── path status
```

وليس:

```text
NAT → QUIC only
```

---

# 31. GTP path capabilities

يجب الاستفادة من قدرات GTP-rs المتعلقة بـ:

```text
path validation
NAT rebinding
path migration
PMTU
loss recovery
congestion control
RTT
```

بدلاً من إعادة تنفيذها في zgalaxy-rs.

لكن يجب فصل:

```text
GTP path state
```

عن:

```text
ZGalaxy Peer path state
```

---

# 32. PeerManager

`PeerManager` يجب أن يحتفظ بمفهوم:

```text
peer
paths
latency
endpoint
status
```

ولا يجب أن يصبح مسؤولاً عن:

```text
GTP implementation
QUIC implementation
Relay implementation
```

هذه مسؤولية PathManager.

---

# 33. Path object

المقترح:

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

وهذا يسمح بتمثيل:

```text
Peer A
 ├── Direct/QUIC
 ├── Direct/GTP
 └── Relay/GTP
```

---

# 34. Transport capabilities

كل transport يجب أن يقدم:

```rust
pub struct TransportCapabilities {
    pub unreliable: bool,
    pub reliable_ordered: bool,
    pub path_migration: bool,
    pub max_payload: usize,
}
```

GTP وQUIC يمكن أن يختلفا.

---

# 35. MTU

لا تستخدم قيمة QUIC الحالية كـ global constant.

بدلاً من:

```text
if quic:
    1186
```

يجب أن يصبح:

```text
TransportCapabilities.max_payload
```

لأن GTP لديه overhead مختلف وPMTU مختلف.

---

# 36. Relay MTU

Relay يضيف overhead إضافياً.

لذلك:

```text
Peer MTU
    ↓
Transport MTU
    ↓
Relay overhead
    ↓
Maximum payload
```

ويجب أن تكون fragmentation/segmentation semantics واضحة.

---

# 37. Reliability mapping

المقترح:

| ZGalaxy traffic | Direct QUIC | Direct GTP | Relay QUIC | Relay GTP |
|---|---|---|---|---|
| L2/L3 frame | Datagram | Unreliable | Datagram | Unreliable |
| Identity control | Stream | ReliableOrdered | Stream | ReliableOrdered |
| Network config | Stream | ReliableOrdered | Stream | ReliableOrdered |
| Ping/Pong | Control | High priority | Control | High priority |

---

# 38. Relay transport selection

لا ينبغي أن يكون:

```text
if relay:
    use QUIC
```

بل:

```text
select path
select transport
```

مثلاً:

```rust
PathSelection {
    mode: Relay,
    transport: Gtp,
}
```

---

# 39. Configuration

المقترح:

```toml
[transport]
mode = "quic"
```

القيم:

```text
quic
gtp
```

ثم:

```toml
[path]
prefer_direct = true
relay_enabled = true
direct_retry = true
```

ثم:

```toml
[relay]
enabled = true
endpoint = "..."
```

لا يجب تثبيت endpoint في code.

---

# 40. Auto mode مستقبلاً

بعد MVP:

```toml
[transport]
mode = "auto"
```

ثم:

```text
Peer capabilities
      ↓
Direct transport selection
      ↓
GTP preferred
      ↓
QUIC fallback
```

لكن هذا لا يجب أن يسبق نجاح GTP/QUIC basic operation.

---

# 41. Per-peer transport مستقبلاً

يمكن دعم:

```text
Peer A → GTP
Peer B → QUIC
Peer C → GTP
```

ثم:

```text
Peer A → Direct GTP
Peer B → Relay QUIC
Peer C → Direct QUIC
```

وهذا سبب إضافي لفصل PathManager عن Transport.

---

# 42. Path scoring

يمكن بناء score:

```text
direct + low latency = high score
direct + high loss = lower score
relay + stable = fallback score
```

مثلاً:

```text
Direct healthy
    score = 100

Direct degraded
    score = 50

Relay
    score = 20
```

ولا يستخدم Relay إلا إذا لم يكن direct صالحاً.

---

# 43. Hysteresis

يجب منع:

```text
Direct
Relay
Direct
Relay
...
```

بسبب jitter.

استخدم:

```text
failure threshold
success threshold
cooldown
```

مثلاً:

```text
3 consecutive direct failures
→ relay

5 successful direct probes
→ direct
```

الأرقام النهائية تحتاج benchmark.

---

# 44. Relay session lifecycle

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

هناك عدة خيارات:

### A. Controller يعطي relay endpoint

```text
NetworkConfigResponse
      +
RelayEndpoint
```

### B. Client لديه relay endpoint ثابت

```text
relay.endpoint
```

### C. External ZGALAXY يوفر relay discovery API

وهذا يحتاج فحصاً وتنفيذاً منفصلاً.

الأنسب للـ MVP:

```text
configured relay endpoint
```

ثم لاحقاً Controller-managed relay discovery.

---

# 46. Relay authorization

يجب أن يستطيع Relay معرفة:

```text
PeerID
NetworkID
membership
token/credential
```

لكن لا يحتاج إلى قراءة data payload.

---

# 47. E2E security

الهدف:

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

Relay يرى فقط metadata الضرورية:

```text
source session
destination session
packet size
timing
```

وليس payload plaintext.

---

# 48. DoS protection

Relay يجب أن يفرض:

```text
max connections
authentication rate limit
per-peer bandwidth
session timeout
packet rate limit
max payload
```

ولا يسمح:

```text
unauthenticated arbitrary forwarding
```

---

# 49. Relay observability

يجب تسجيل:

```text
relay sessions
active peers
bytes in/out
packets
drops
reason for fallback
direct recovery
```

لكن لا تسجل plaintext packets.

---

# 50. أهم metrics في Client

يجب إضافة:

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

# 51. API/debug endpoint

يفضل إضافة حالة:

```text
/peer
```

أو endpoint داخلي يعرض:

```json
{
  "peer": "...",
  "path": "direct",
  "transport": "gtp",
  "healthy": true,
  "latency_ms": 12
}
```

وعند Relay:

```json
{
  "peer": "...",
  "path": "relay",
  "transport": "gtp",
  "healthy": true
}
```

مع الحفاظ على backward compatibility للـ API الحالي.

---

# 52. Main.rs refactoring

الهدف النهائي:

```rust
let transport = build_transport(config).await?;
let path_manager = PathManager::new(...);
let control_engine = ControlEngine::new(...);
```

ثم event loop عام:

```rust
while let Some(event) = transport.next_event().await {
    path_manager.handle(event).await?;
}
```

ولا يحتوي `main.rs` على QUIC-specific controller handling.

---

# 53. الملفات المقترحة

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

# 54. لا تحذف `src/transport.rs`

يجب التمييز بين:

```text
src/transport.rs
```

و:

```text
src/transport/gtp.rs
```

الأول legacy/native UDP wire transport.

الثاني GTP Mesh Transport.

يمكن لاحقاً إعادة تسمية الملفات لتقليل الالتباس، لكن لا ينبغي تنفيذ rename كبير بالتزامن مع GTP.

---

# 55. مراحل التنفيذ

## Phase 0 — Repository audit

افحص:

```text
ZGALAXY
zgalaxy-rs
GTP-rs
```

مع توثيق:

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

انقل:

```text
ControlMessage
```

إلى module مستقل.

---

## Phase 2 — Transport abstraction

أنشئ:

```text
MeshTransport
TransportEvent
TransportCapabilities
```

---

## Phase 3 — QUIC adapter

اجعل QUIC يعمل عبر abstraction بدون تغيير behavior.

هذه مرحلة إلزامية قبل GTP.

---

## Phase 4 — ControlEngine

انقل:

```text
NodeAnnounce
NodeChallenge
AnnounceProof
NetworkConfigRequest
NetworkConfigResponse
Ping/Pong
```

خارج `main.rs`.

---

## Phase 5 — PathManager

أضف:

```text
DirectPath
RelayPath
PathState
PathPolicy
```

لكن يمكن في البداية تنفيذ Relay mock/in-process للاختبار.

---

## Phase 6 — NAT decoupling

اجعل NAT يتعامل مع:

```text
PathManager
```

وليس QUIC مباشرة.

---

## Phase 7 — GTP-rs

أضف dependency واختبر:

```text
runtime
endpoint
connection
unreliable
reliable
path APIs
```

لا تعتمد على API مفترض.

---

## Phase 8 — Direct GTP

نفذ:

```text
Peer A → GTP → Peer B
```

بدون Relay أولاً.

يجب إثبات:

```text
identity
control
TUN
peer state
NAT
```

---

## Phase 9 — GTP Controller mode

اختبر:

```text
GTP Client
     ↓
GTP
     ↓
zgalaxy-rs EmbeddedController
```

مع:

```text
NetworkConfigRequest
```

---

## Phase 10 — Relay server

أضف:

```text
RelayService
```

إما داخل `zgalaxy-rs` controller mode أو binary مستقل، حسب deployment requirements.

---

## Phase 11 — Relay over QUIC

أثبت:

```text
Peer A → QUIC Relay → Peer B
```

---

## Phase 12 — Relay over GTP

ثم:

```text
Peer A → GTP Relay → Peer B
```

---

## Phase 13 — Automatic fallback

نفذ:

```text
Direct first
→ timeout
→ Relay
```

---

## Phase 14 — Recovery

نفذ:

```text
Relay
→ direct probe
→ direct recovered
→ migrate
→ close relay
```

---

# 56. الاختبارات الأساسية

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

اختبر:

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

قارن:

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

اختبر:

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

النتيجة:

```text
reject
no panic
no unbounded memory
no authorization bypass
```

---

# 60. Migration strategy

لا تجمع كل التغييرات في commit واحد.

المقترح:

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

# 61. قرار مهم: Relay ليس Transport ثالثاً

لا نريد:

```text
QUIC
GTP
Relay
```

كأنها ثلاثة transports متساوية.

التصميم الصحيح:

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

هذا يزيل الالتباس بالكامل.

---

# 62. قرار مهم: Controller ليس Relay

التصميم:

```text
EmbeddedController
    =
control/management

RelayService
    =
data forwarding
```

يمكن تشغيلهما في نفس process، لكنهما modules منفصلة.

---

# 63. قرار مهم: GTP ليس مسؤولاً عن fallback

لا:

```text
GtpTransport
   ↓
if failed
   ↓
Relay
```

بل:

```text
PathManager
   ↓
Direct GTP failed
   ↓
choose Relay
   ↓
Relay GTP
```

وبالمثل:

```text
Direct QUIC failed
   ↓
Relay QUIC
```

---

# 64. قرار مهم: Direct هو الحالة الطبيعية

الـ relay يجب أن يكون:

```text
fallback
```

وليس:

```text
default topology
```

لأسباب:

```text
latency
bandwidth
server cost
scalability
privacy
```

---

# 65. قرار مهم: Relay لا يفك التشفير

الهدف:

```text
E2E:
Peer A ===================== Peer B
             encrypted
                ↓
             Relay
             opaque
```

وهذا يقلل trust requirements على Relay.

---

# 66. GTP وRelay: أفضل صيغة

الهدف النهائي:

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

وبذلك يمكن مستقبلاً إضافة:

```text
WebSocket relay
TCP relay
another transport
```

من دون إعادة تصميم الـ Client.

---

# 67. هل نحتاج تعديل ZGALAXY؟

ليس كشرط لإضافة GTP إلى `zgalaxy-rs`.

لكن إذا كان المطلوب:

```text
ZGALAXY external service
    ↓
Relay discovery
    ↓
Relay allocation
```

فهذا مشروع integration منفصل.

يجب أولاً فحص API والبنية الفعلية لـ `ZGALAXY` وإثبات وجود/غياب:

```text
relay allocation
relay registry
peer rendezvous
relay authentication
```

ولا ينبغي افتراض وجودها من مجرد اسم المشروع.

---

# 68. القرار المعماري النهائي

التصميم الذي ينبغي اعتماده:

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

# 69. الخلاصة التنفيذية

المشروع المطلوب ليس:

```text
"إضافة GTP إلى QUIC"
```

ولا:

```text
"استبدال QUIC بـ GTP"
```

بل:

```text
إعادة فصل architecture الخاصة بـ zgalaxy-rs
بحيث يصبح transport/path مستقلاً عن ZGalaxy semantics.
```

ثم:

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

وبالتالي يصبح السيناريو النهائي:

### الحالة الطبيعية

```text
Peer A
  │
  │ Direct GTP
  ▼
Peer B
```

### إذا فشل GTP direct

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

### إذا كان GTP غير متاح لكن QUIC متاح

```text
Peer A
  │
  │ Direct QUIC
  ▼
Peer B
```

### إذا فشل Direct بالكامل

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

### وإذا عاد Direct

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

**هذه هي البنية التي أوصي باعتمادها كهدف معماري رسمي للمشروع.**

---

# 70. المصادر

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

المراجع المستخدمة لفهم delivery modes، loss recovery، congestion control، priority scheduling، AEAD، path validation، NAT rebinding/path migration وTokio integration.

---

## `ZGALAXY`

- Repository:  
  https://github.com/dreamzone-cc/ZGALAXY

هذا المستودع منفصل عن `zgalaxy-rs`. يجب عدم افتراض أن وجود `ZGALAXY` يعني وجود Relay Data Plane فيه إلا بعد إثبات ذلك من الكود/API الفعلي.

---

## مراجع تصميمية إضافية

استخدمت أيضاً نماذج Relay/P2P الحديثة للتحقق من مبدأ:

```text
Direct preferred → Relay fallback
```

ومنها NetBird الذي يعرّف Relay client لإدارة connections إلى peers عبر relay، ومشاريع P2P التي تستخدم direct-first ثم relay fallback. هذه المراجع **ليست مصدراً لخصائص ZGALAXY نفسها**، وإنما مرجع لتصميم نمط الـ fallback. 
