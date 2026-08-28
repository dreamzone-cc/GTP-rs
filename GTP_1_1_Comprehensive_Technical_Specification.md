# Game Transport Protocol v1.1 (GTP/1)
## المواصفة التقنية الشاملة لبروتوكول نقل ألعاب تنافسي منخفض الكمون فوق UDP

**الحالة:** Technical Architecture / Research Specification
**تاريخ الإصدار:** 28 أغسطس 2026
**اللغة المستهدفة للتنفيذ:** Rust
**المنصة المرجعية:** Linux / Internet / Data Center / LAN
**الملف:** GTP/1.1 Comprehensive Technical Specification

> **ملاحظة منهجية:** هذه الوثيقة توسع وتعيد ضبط ورقة GTP/1 السابقة. القرارات الأساسية الخاصة بدلالات الرسائل، Connection ID، ACK ranges، selective retransmission، deadlines، pacing، multi-core affinity، والـ I/O abstraction محفوظة، بينما أضيفت إليها متطلبات أحدث مرتبطة بالـ QUIC recovery، ACK Frequency، UDP GSO/GRO، io_uring، thread-per-core، وبنية Rust منخفضة الكلفة.

---

# 1. الملخص التنفيذي

يقترح هذا المستند **Game Transport Protocol v1 (GTP/1)** كطبقة نقل متخصصة للألعاب real-time والتنافسية فوق UDP، وليست بديلاً عامًا لـ TCP أو QUIC.

الهدف هو بناء Transport يعرف دلالة البيانات وعمرها، ويستطيع اتخاذ قرار النقل وفق أربعة أبعاد مستقلة:

1. هل يجب أن تصل الرسالة؟
2. هل يهم ترتيبها؟
3. هل ما زالت الرسالة صالحة عند وقت التسليم؟
4. ما مقدار الموارد التي يجوز إنفاقها عليها في ظل حالة الشبكة؟

يحتوي البروتوكول على أربع دلالات أساسية:

```text
UNRELIABLE
UNRELIABLE_SEQUENCED
RELIABLE_UNORDERED
RELIABLE_ORDERED
```

ويستخدم اتصالاً واحداً ومساراً منطقياً واحداً وCongestion Controller واحداً وPacing واحداً، مع Queue/Scheduler متعدد الدلالات.

التصميم يأخذ من:

```text
KCP
  selective ARQ
  tunability
  low overhead
  delivery telemetry

QUIC
  connection identifiers
  packet numbering
  ACK ranges
  RTT/loss recovery
  path validation
  migration concepts
  anti-amplification
  AEAD boundary

QUIC DATAGRAM
  unreliable congestion-controlled datagrams

Homa-inspired scheduling
  message-oriented scheduling
  receiver/deadline awareness concepts

Linux UDP
  UDP_SEGMENT / GSO
  UDP_GRO
  batched I/O
  io_uring

Rust
  ownership
  zero-copy views
  thread-local state
  compile-time invariants
```

لكن GTP لا ينسخ أي بروتوكول بصورة كاملة.

المبدأ النهائي هو:

```text
Game Semantics
      ↓
GTP Message Engine
      ↓
Freshness / Deadline / Priority Scheduler
      ↓
Congestion Budget + Pacing
      ↓
ACK / RTT / Loss Recovery
      ↓
Path Management
      ↓
AEAD / Integrity
      ↓
Packet Codec
      ↓
UDP I/O Backend
      ↓
Linux Fast Path / NIC
```

---

# 2. النطاق

## 2.1 داخل النطاق

GTP/1 مصمم لـ:

- ألعاب FPS التنافسية.
- Racing.
- Battle Royale.
- ألعاب الحركة real-time.
- ألعاب multiplayer واسعة النطاق.
- game state replication.
- player input.
- snapshots.
- gameplay events.
- RPC/events التي تتطلب ترتيباً جزئياً.
- Internet وNAT.
- LAN وData Center.
- خوادم متعددة الأنوية.

## 2.2 خارج النطاق

لا يكون GTP transport مناسباً أساساً لـ:

- HTTP/3 compatibility.
- browser-native WebTransport replacement.
- file transfer العام.
- byte-stream compatibility.
- mail/transaction systems.
- bulk transport غير المرتبط بالألعاب.

يمكن استخدام reliable mode في نقل بيانات كبيرة صغيرة/متوسطة داخل اللعبة، لكن ذلك ليس هدف البروتوكول الرئيسي.

---

# 3. أهداف التصميم

## 3.1 أهداف MUST

يجب أن:

- يعمل فوق UDP.
- يطبق congestion control على إجمالي traffic الخاص بالجلسة.
- يطبق pacing.
- يدعم loss detection سريعاً.
- يفرق بين packet identity وmessage identity.
- يدعم unreliable وreliable semantics.
- لا يعيد إرسال state أصبحت قديمة.
- يمنع HoL بين message classes المختلفة.
- يدعم Connection ID.
- يدعم packet number.
- يدعم ACK ranges.
- يدعم path validation.
- يدعم NAT rebinding.
- يدعم anti-amplification.
- يدعم authenticated/encrypted Internet mode.
- يمنع allocation لكل packet في steady state.
- يسمح بالـ batch I/O.
- يفصل protocol core عن runtime والـ kernel backend.

## 3.2 أهداف SHOULD

يفضل أن:

- يدعم ECN.
- يدعم adaptive ACK frequency.
- يدعم GSO/GRO على Linux.
- يدعم io_uring backend.
- يدعم thread-per-core.
- يدعم application-level redundancy.
- يسمح بإضافة FEC مستقبلاً.
- يدعم PMTU probing.
- يقدم observability شاملة دون packet logging دائم.

## 3.3 OPTIONAL

- multipath.
- 0-RTT.
- FEC.
- hardware crypto offload.
- AF_XDP.
- DPDK.
- compression.
- advanced ECN strategies.

## 3.4 EXPERIMENTAL

- GTP-BBR-like controller.
- receiver-assisted scheduling.
- adaptive redundancy.
- cross-packet state compression.
- per-path traffic steering.

---

# 4. المبادئ الأساسية

## 4.1 البيانات ليست متساوية

المعلومة التالية ليست مثل السابقة:

```text
Player position
Purchase confirmation
Weapon fired
Cosmetic effect
```

لذلك لا يجب دفعها كلها داخل ordered reliable stream واحد.

## 4.2 Packet != Message

Packet هو وحدة النقل على الشبكة.

Message هي وحدة معنى في اللعبة.

قد يحتوي packet واحد على عدة messages، وقد تحتاج message واحدة إلى عدة packets.

## 4.3 Freshness جزء من transport semantics

في لعبة real-time، الرسالة الصحيحة التي تصل متأخرة قد تصبح خاطئة عملياً.

لذلك يجب أن يستطيع النقل أن يقول:

```text
expired => DROP
```

بدلاً من تنفيذ retransmission أعمى.

## 4.4 Congestion control لا يجوز تجاوزه

Priority تعني الاختيار داخل budget فقط.

لا تعني:

```text
HIGH PRIORITY => ignore cwnd
```

---

# 5. نموذج الاتصال

يستخدم GTP مفهوم Session/Connection ID بدلاً من الاعتماد الكامل على 5-tuple.

```text
Connection ID = 64 bits minimum target
```

ويجب أن يكون opaque بالنسبة للطرف الآخر وألا يكشف:

- IP.
- Port.
- User ID.
- Shard ID الخام.
- CPU core.

يمكن للتنفيذ أن يستخدم mapping داخلياً:

```text
CID → worker/shard
```

من أجل scaling.

---

# 6. Packet Identity

يجب فصل:

```text
Packet Number
Message ID
Fragment ID
Transmission ID
State Sequence
Generation ID
```

### Packet Number

يمثل packet على مستوى transport.

### Message ID

يمثل logical game message.

### Fragment ID

يحدد جزء message مجزأة.

### Transmission ID

يحدد محاولة إرسال معينة للـ message/fragment.

### State Sequence

يحدد freshness ضمن state key.

### Generation ID

يحدد جيل snapshot/state الكامل.

---

# 7. دلالات الرسائل

## 7.1 UNRELIABLE

للبيانات التي يمكن تعويضها برسالة لاحقة:

- position.
- rotation.
- velocity.
- aim.
- snapshot fragments.

عند الفقد:

```text
DROP
```

## 7.2 UNRELIABLE_SEQUENCED

تستخدم عندما تكون أحدث نسخة فقط ذات قيمة.

مثال:

```text
seq 100 → accept
seq 102 → accept
seq 101 → drop
```

## 7.3 RELIABLE_UNORDERED

معلومات يجب ألا تضيع، لكن لا يجب أن تنتظر رسائل أخرى:

```text
achievement unlocked
item discovered
damage event
combat trigger
```

## 7.4 RELIABLE_ORDERED

يجب أن تستخدم فقط عندما يكون الترتيب جزءاً من الدلالة:

```text
JOIN
SETUP
START
END
```

ولا تكون default path.

---

# 8. Message Descriptor

كل message يمكن أن ترتبط داخلياً بهذا النموذج:

```text
MessageDescriptor
    class
    message_id
    state_key?
    sequence?
    generation?
    created_at
    deadline?
    priority
    reliability
    ordering
    size
    retransmission_policy
```

هذه المعلومات ليست كلها ضرورية على wire؛ بعضها application/transport internal metadata.

---

# 9. Generation-Aware State

لتقليل queue pressure:

```text
state_key
  generation
  sequence
  deadline
```

إذا وصلت generation حديثة، يمكن إسقاط generations الأقدم دفعة واحدة.

مثال:

```text
world_state generation 55
```

أي queued state من generation < 55 يمكن إسقاطه عندما تكون semantics تسمح بذلك.

---

# 10. Deadline Semantics

الـ deadline يجب أن يكون transport-aware.

الصيغة المفاهيمية:

```text
now
created_at
remaining_lifetime = deadline - now
```

والـ scheduler يجب أن يعرف:

- priority.
- urgency.
- freshness.
- size.
- expected delivery time.
- retransmission value.

الرسالة المنتهية:

```text
DROP
```

بغض النظر عن reliability الأصلية إذا كانت الدلالة تسمح بانتهاء صلاحيتها.

---

# 11. Scheduler

الـ scheduler المقترح ليس priority queue عادية.

المعمارية:

```text
Deadline Scheduling
        +
Priority Scheduling
        +
Freshness Filtering
        +
Weighted Fairness
        +
Cwnd / Pacing Budget
```

## 11.1 طبقات الأولوية

```text
P0 Control
P1 Player Input
P2 Fresh World State
P3 Reliable Gameplay
P4 Cosmetic/Bulk
```

ولكن جميعها تخضع للـ congestion budget.

## 11.2 starvation prevention

يجب منع starvation باستخدام weighted service أو aging.

## 11.3 stale drop

عند ازدحام queue:

```text
stale state
    ↓
drop first
```

قبل البيانات reliable غير المنتهية.

---

# 12. Effective Queue

لا ينبغي قياس queue فقط بعدد bytes.

نقترح مفهوم:

```text
effective_queue_bytes
```

أي حجم البيانات التي ما زالت تحمل utility فعلية.

مثال:

```text
queued = 100 KB
expired = 70 KB
valid = 30 KB
```

فيصبح الضغط المنطقي أقرب إلى:

```text
30 KB useful queue
```

بعد إزالة stale state.

---

# 13. Packet Format

الهيكل المنطقي:

```text
+-------------------------------+
| Flags / Version / Header Len  |
+-------------------------------+
| Connection ID                 |
+-------------------------------+
| Packet Number                 |
+-------------------------------+
| Timestamp / Timing metadata   |
+-------------------------------+
| Payload Length                |
+-------------------------------+
| ACK Section (optional)        |
+-------------------------------+
| Frames                        |
+-------------------------------+
| AEAD Tag                      |
+-------------------------------+
```

## 13.1 Common Header Target

الهدف المبدئي:

```text
~ 20–28 bytes
```

ولكن يجب ألا يفرض الرقم نفسه قبل profiling وwire-format review.

## 13.2 Long Header

يستخدم للـ:

- handshake.
- version negotiation.
- stateless validation.
- path/control transitions.

## 13.3 Short Header

يستخدم بعد استقرار الاتصال لتقليل overhead.

---

# 14. Frames

الـ packet قد يحتوي على:

```text
ACK
DATA
RELIABLE_DATA
RETX
CONTROL
PING
PATH_CHALLENGE
PATH_RESPONSE
MTU_PROBE
CLOSE
```

قد تُضاف:

```text
ACK_FREQUENCY
FEC
PADDING
```

كامتدادات مستقبلية.

---

# 15. ACK Architecture

ACK لا يعني فقط retransmission.

بل يوفر:

```text
Delivery evidence
RTT sample
Loss signal
Congestion signal
ECN feedback
```

لذلك:

```text
ACK
 ↓
RTT
 ↓
Loss detector
 ↓
Delivery-rate estimator
 ↓
CC
 ↓
Pacing
```

---

# 16. ACK Ranges

يستخدم GTP تمثيل ranges بدلاً من ACK packet لكل packet.

مثال:

```text
Largest = 1050

1050-1050
1047-1049
1030-1040
```

هذا يسمح بتمثيل loss المتفرق وإعادة الترتيب بكفاءة.

---

# 17. Adaptive ACK Frequency

GTP يضيف مفهوماً مشابهاً لاتجاه QUIC ACK Frequency الحديث: يمكن للمستقبل والمرسل التكيف مع packet rate والحالة.

سياسات محتملة:

```text
normal:
ACK every N packets

high rate:
higher N

loss/reordering suspicion:
ACK more frequently

critical measurement:
immediate ACK
```

ولا يجب تثبيت ACK every 2 packets كقانون دائم.

**ملاحظة زمنية:** كان QUIC ACK Frequency ما يزال Internet-Draft في 2026 وليس RFC نهائياً في وقت إعداد هذه الورقة، لذلك يستخدم GTP الفكرة كمرجع تصميمي لا كاعتماد معياري مباشر.

---

# 18. RTT Estimation

يجب الاحتفاظ على الأقل بـ:

```text
latest_rtt
smoothed_rtt
min_rtt
rttvar
ack_delay
```

ويجب عدم اتخاذ كل قرار congestion اعتماداً على RTT خام واحد.

---

# 19. Loss Detection

مصادر loss:

```text
ACK gap
Packet threshold
Time threshold
PTO-like timeout
```

يجب الفصل بين:

```text
loss declaration
retransmission policy
congestion reaction
```

لأن فقد state غير موثوقة قد لا يعني retransmission.

---

# 20. Selective Recovery

عند فقد packet:

```text
Packet 100
  DATA A
  DATA B
  DATA C
```

لا يعاد packet 100 كاملاً بالضرورة.

إذا كانت B فقط reliable:

```text
Packet 105
  RETX B
```

هذه قاعدة مركزية:

> Retransmission is logical-message/frame based, not packet-copy based.

---

# 21. Transmission Records

لكل transmission record يمكن الاحتفاظ:

```text
packet_number
message_id
fragment_id
transmission_id
send_time
bytes
ack_eliciting
retransmittable
```

ويجب أن يدعم هذا record:

- loss detection.
- RTT.
- delivery-rate.
- retransmission.
- debugging.

---

# 22. KCP v2.1.1 Lessons

تطور KCP في 2026 مهم لتصميم GTP. الإصدار 2.0 أضاف نظام congestion-control قابل للاستبدال، والإصدار 2.1.1 أضاف `acked_bytes` و`xmit` لدعم bandwidth estimation، ونقل callback الإرسال إلى نقطة الإرسال الفعلية، وأضاف pacing اختيارياً وحسّن ssthresh/cwnd growth.

GTP يجب أن يأخذ من ذلك:

```text
acked bytes
actual send timestamp/point
transmission count
pluggable congestion controller
optional pacing hooks
```

لكن لا يتبنى KCP نفسه كقلب transport.

---

# 23. Delivery-Rate Telemetry

كل ACK processing يجب أن يستطيع إنتاج:

```text
acked_bytes
send_time
ack_time
prior_inflight
delivery_interval
```

ثم:

```text
delivery_rate = delivered_bytes / delivery_interval
```

ويجب حفظ samples بطريقة لا تولد allocation على كل ACK.

---

# 24. Congestion Control API

الواجهة المنطقية:

```rust
trait CongestionController {
    fn on_packet_sent(&mut self, info: SentPacket);
    fn on_ack(&mut self, ack: AckEvent);
    fn on_loss(&mut self, loss: LossEvent);
    fn on_ecn(&mut self, ecn: EcnEvent);
    fn on_rtt(&mut self, rtt: RttSample);
    fn on_timeout(&mut self);
    fn cwnd(&self) -> u64;
    fn pacing_rate(&self) -> u64;
    fn inflight(&self) -> u64;
}
```

الـ actual API يمكن تحسينه أثناء التنفيذ.

---

# 25. Congestion Controllers

## 25.1 Baseline

النسخة المرجعية:

```text
CUBIC / NewReno-compatible behavior
```

لضمان سلوك مفهوم وسهل المقارنة.

## 25.2 Delivery-rate controller

تجريبي:

```text
BBR-inspired
```

مع قياس:

- delivery rate.
- RTT.
- inflight.
- ECN.
- loss.

## 25.3 GTP-specific controller

مستقبلاً يمكن تصميم:

```text
GTP-CC
```

ليوازن:

```text
fairness
RTT inflation
freshness utility
loss
throughput
```

لكن يجب عدم اعتماده production قبل benchmark واسع.

---

# 26. Fairness

GTP يعمل فوق Internet، وبالتالي لا يجوز تصميمه لاحتكار bottleneck.

يجب اختبار مشاركته مع:

```text
TCP
QUIC
GTP
```

والتحقق من:

- throughput fairness.
- RTT stability.
- no congestion collapse.
- ECN response.

لا يعني game priority السماح لـ GTP بتجاوز congestion control.

---

# 27. ECN

يدعم التصميم:

```text
ECT(0)
ECT(1)
CE
```

ويجب أن يكون للـ CC handler صريح:

```text
on_ecn()
```

لأن CE signal قد يصل قبل loss.

---

# 28. Pacing

Pacing mandatory architectural component.

```text
Congestion Controller
        ↓
Pacing Rate
        ↓
Send Budget
        ↓
Scheduler
        ↓
Packet Builder
```

الهدف منع bursts غير الضرورية وتقليل queue buildup.

---

# 29. Pacing Model

يستخدم:

```text
next_send_time
send_budget
burst_cap
```

ولا يعتمد فقط على `sleep()`.

يمكن أن يستخدم loop/event timer مع batching إذا كان الوقت يسمح.

---

# 30. Burst Control

يسمح Burst صغير controlled:

```text
burst <= configured bound
```

لكن لا يجوز تفريغ عشرات packets دفعة واحدة فقط بسبب وصول ACK burst.

---

# 31. Transport Budget

يقترح GTP متغيراً مفاهيمياً:

```text
send_budget
```

مصادره:

```text
cwnd
inflight
pacing tokens
queue state
path state
```

الـ scheduler يختار ما يدخل هذا budget.

---

# 32. Freshness-Aware Congestion

هذه إحدى أهم إضافات GTP.

الازدحام لا يجب أن يعامل 100 KB queued على أنها 100 KB useful دائماً.

إذا:

```text
70 KB expired
30 KB valid
```

فالأولوية هي حذف الـ70KB ودفع الـ30KB ذات القيمة.

هذا يساعد على مقاومة application-level queue buildup وbufferbloat.

---

# 33. Backpressure

مستويات الضغط المقترحة:

```text
LOW
    normal scheduling

MEDIUM
    reduce stale state

HIGH
    aggressively expire state
    drop cosmetic

CRITICAL
    preserve control + critical reliable
    stop non-essential injection
```

القيم النهائية يجب أن تضبط بالـ benchmark.

---

# 34. Realtime Redundancy

بدلاً من retransmission التقليدي يمكن إرسال:

```text
Packet 100:
state 100

Packet 101:
state 101 + compact state 100
```

لكن redundancy:

- optional.
- bounded.
- congestion-controlled.
- adaptive.

ولا يجوز أن تتحول إلى congestion amplification.

---

# 35. FEC

FEC ليست mandatory في v1.

لكن architecture يجب أن تسمح بـ:

```text
Protection Group
  data1
  data2
  data3
  parity
```

الأولوية التنفيذية:

```text
ACK/loss
→ congestion control
→ pacing
→ scheduling
→ FEC
```

---

# 36. Fragmentation

لا يسمح protocol بإرسال UDP/IP datagrams غير منضبطة قد تؤدي إلى fragmentation على IP.

ينبغي أن يكون:

```text
realtime message
    <= one packet whenever practical
```

والرسائل الكبيرة:

```text
message
 ↓
fragments
 ↓
selective recovery
```

كل fragment يجب أن يملك هوية مستقلة في recovery state.

---

# 37. MTU / PMTU

يبدأ الاتصال بحجم محافظ.

ثم:

```text
probe
 ↓
ack
 ↓
raise MTU
```

عند failure:

```text
lower MTU
```

لا ينبغي الاعتماد على IP fragmentation.

على المسار المرجعي، يمكن اعتماد 1200-byte-class packets كـ baseline محافظ ثم probing نحو قيم أعلى حسب path capability.

---

# 38. Linux UDP Offload

يجب أن يدعم backend Linux قدر الإمكان:

```text
UDP_SEGMENT / GSO
UDP_GRO
```

Linux يوثق أن UDP segmentation offload يسمح بتمرير عدة datagrams في إرسال واحد عبر kernel transmit path، مع تقسيم لاحق وفق segment size، بينما UDP GRO يعكس ذلك في RX ويجمع عدة datagrams ضمن buffer كبير.

هذه التحسينات **backend-specific** ولا تدخل في wire protocol.

---

# 39. GSO Strategy

بدلاً من:

```text
send()
send()
send()
send()
```

يمكن للـ backend تجميع عدة GTP packets المستقلة في إرسال أكبر عندما تكون:

- متوافقة مع MTU بعد segmentation.
- ضمن نفس socket/path.
- متوافقة مع pacing budget.

يجب ألا يسمح GSO بتجاوز pacing أو تشكيل burst غير مرغوب.

---

# 40. GRO Strategy

RX path يمكن أن يستقبل buffer يحتوي عدة datagrams.

يجب أن يبقى GTP parser قادراً على:

```text
split
validate
parse
process
```

من دون نسخ غير ضروري.

GRO لا يغير packet numbering semantics؛ هو optimization في I/O layer فقط.

---

# 41. I/O Backend Abstraction

الواجهة المفاهيمية:

```rust
trait PacketIo {
    fn recv_batch(&mut self, ...);
    fn send_batch(&mut self, ...);
    fn capabilities(&self) -> IoCapabilities;
}
```

الـ backends:

```text
portable UDP
Linux UDP
io_uring
Tokio adapter
Monoio adapter
AF_XDP future
DPDK future
```

---

# 42. io_uring Backend

بالنسبة إلى Linux الحديثة، يجب دراسة:

```text
multishot recv
provided buffer groups
recvmsg multishot
bundle-based receives
```

الإصدارات الحديثة من Rust `io-uring` توفر عمليات receive متعددة الرسائل، ويمكن لـ `RecvMsgMulti` إبقاء receive request واحدة فعالة وإصدار CQEs متعددة، كما يتوفر bundle-style receive في kernels حديثة.

يجب أن يكون backend قادراً على الاستفادة منها عند توفرها، مع fallback إلى recvmsg/recvfrom التقليدي.

---

# 43. Monoio

يعد Monoio backend/runtime مرجعاً مهماً لأن تصميمه thread-per-core، ويفصل العمليات بحيث يبقى state محلياً للـ thread في الحالات المناسبة.

GTP لا يجب أن يعتمد عليه في core API، لكنه مناسب جداً لبناء performance runtime.

---

# 44. Tokio

يستخدم كـ:

```text
reference / integration runtime
```

وليس من الضروري أن يكون transport core مبنياً حول Tokio.

ذلك يسمح بمقارنة:

```text
Tokio
vs
Monoio
vs
native io_uring loop
```

مع نفس protocol core.

---

# 45. Thread-per-Core Architecture

الهدف المرجعي:

```text
NIC RX queue
     ↓
CPU/core
     ↓
GTP worker
     ↓
Connection owner
```

بعد إنشاء الاتصال:

```text
Connection → worker affinity
```

ما أمكن.

هذا يقلل:

- locks.
- cache line bouncing.
- cross-core synchronization.
- shared mutable state.

---

# 46. Rust Ownership as Performance Tool

لا ينبغي أن تكون البنية الأساسية:

```rust
Arc<Mutex<Connection>>
```

في hot path.

يفضل:

```text
single owner
single writer
thread-local mutable state
```

والتفاعل بين cores عبر channels أو queues مصممة بعناية عند الحاجة.

---

# 47. Hot / Cold State

يجب تقسيم connection state:

```text
ConnectionHot
ConnectionCold
```

## Hot

- packet number.
- ACK state.
- RTT.
- loss state.
- cwnd.
- pacing.
- scheduler.
- inflight.
- path current.

## Cold

- debug data.
- historical metrics.
- migration history.
- rarely used extension state.
- verbose security metadata.

الهدف إبقاء hot state صغيرة وcache-friendly.

---

# 48. Memory Management

لا allocation لكل packet.

مكونات مقترحة:

```text
PacketPool
MessagePool
FragmentPool
TransmissionPool
ConnectionPool
```

ويجب استخدام recycling.

لكن لا يجب فرض pool واحد عالمي يخلق lock contention؛ الأفضل per-worker pools مع fallback محدود.

---

# 49. Slab / Arena Strategy

يمكن استخدام slabs محلية لكل worker.

مثال:

```text
worker 0
  packet slabs
  message slabs

worker 1
  packet slabs
  message slabs
```

وعند نقل ownership بين workers يجب تقليل cross-core transfer.

---

# 50. Zero-Copy Strategy

يجب أن يكون wire parsing تقريباً:

```text
&[u8]
  ↓
header view
  ↓
frame iterator
  ↓
message view
```

بدلاً من deserialize كامل إلى heap objects.

`zerocopy` مرشح مناسب لبناء typed byte views بشكل منخفض الكلفة مع الحفاظ على checks/validation المناسبة.

لكن يجب عدم فرض zero-copy إذا كان سيعقد lifetime management أو يؤدي إلى retaining buffers ضخمة؛ في هذه الحالة ينسخ GTP فقط البيانات التي يجب أن تعيش بعد RX buffer.

---

# 51. Wire Codec

الـ hot path يفضل أن يستخدم codec متخصصاً ومحدداً، مثل:

```text
fixed prefix parsing
bounded varints
zero-copy slices
frame iterator
```

ويجب ألا يعتمد path الحرج على serialization generic ثقيل.

Ser/de frameworks يمكن أن تستخدم للـ configuration وcontrol-plane data، وليس شرطاً للـ packet codec.

---

# 52. Parser Requirements

الـ parser يجب أن يكون:

- allocation-free.
- bounds-checked.
- incremental.
- branch-conscious.
- resistant to malformed lengths.
- resistant to integer overflow.
- capable of early rejection.

الترتيب المقترح:

```text
minimum header check
 ↓
CID lookup
 ↓
basic bounds
 ↓
cheap policy filters
 ↓
AEAD/auth verification
 ↓
full frame decode
```

---

# 53. Authentication Ordering

لا ينبغي تنفيذ expensive crypto قبل minimal packet sanity وconnection lookup عندما يمكن تجنب ذلك.

RX:

```text
RX
↓
minimal parse
↓
CID/shard lookup
↓
rate/replay prefilter
↓
AEAD
↓
full parse
```

هذه البنية تقلل تكلفة packets عشوائية/ضارة.

---

# 54. Security Model

Internet profile:

```text
authenticated + AEAD
```

أما plain mode فيبقى فقط:

```text
LAN
lab
benchmark
controlled environments
```

ولا يوصى بتمكينه كـ Internet default.

---

# 55. AEAD

يجب أن تكون packet protection مرتبطة بـ:

```text
Connection keys
Packet number
Nonce derivation
AAD
Authentication tag
```

ويجب توفير replay resistance.

crypto implementation يجب أن يسمح بالاستفادة من implementations عالية الأداء/hardware acceleration عند توفرها دون تغيير wire semantics.

---

# 56. Handshake

الحالة:

```text
CLIENT_INIT
      ↓
SERVER_INIT
      ↓
CLIENT_CONFIRM
      ↓
ESTABLISHED
```

ويجب أن تبقى handshake state مستقلة عن game state.

---

# 57. Stateless Validation / Anti-DoS

لا ينبغي إنشاء connection state مكلف فور وصول packet مجهول.

المسار:

```text
Unknown packet
     ↓
cheap validation
     ↓
stateless token/cookie
     ↓
validated client
     ↓
allocate connection state
```

مع anti-amplification limit قبل إثبات قابلية الوصول.

---

# 58. 0-RTT

اختياري ومستقبلي.

لا تستخدمه لأفعال غير idempotent أو حساسة مثل:

```text
purchase
inventory mutation
ranked result
```

إلا بعد تصميم replay protection على مستوى application semantics.

---

# 59. NAT Traversal

GTP يجب أن يكون صالحاً عبر NAT.

keepalive policy يجب ألا تكون aggressive بلا داعٍ.

يمكن استخدام:

```text
PING
PATH_CHALLENGE
PATH_RESPONSE
```

حسب حالة الاتصال.

---

# 60. NAT Rebinding

عند تغير:

```text
client IP
client port
```

لا تُنشأ session جديدة مباشرة.

المسار:

```text
new address
  ↓
PATH_CHALLENGE
  ↓
PATH_RESPONSE
  ↓
validate
  ↓
switch active path
```

---

# 61. Connection Migration

المفهوم:

```text
Path A
   X
Path B
```

يجب الحفاظ على session/game state متى كان ذلك آمناً.

migration يجب ألا يسمح لـ spoofed packet بفرض مسار جديد؛ التحقق mandatory.

---

# 62. Path State

يجب أن يكون هناك path object منطقي:

```text
Path
  local address
  remote address
  validation status
  RTT
  MTU
  ECN capability
  last activity
```

لكن connection يمكن أن يملك path واحداً active في v1، مع إمكانية إضافة multipath لاحقاً.

---

# 63. Versioning

Long header/handshake يجب أن يدعم version negotiation.

Short packet overhead يجب أن يبقى منخفضاً.

يجب أن تكون extensions قابلة للتعرف دون جعل parser هشاً أمام future frames.

---

# 64. Extension Model

يمكن استخدام type/length framing:

```text
TYPE
LENGTH
VALUE
```

Unknown non-critical frame:

```text
ignore/skip
```

Unknown critical frame:

```text
CONNECTION_CLOSE
```

---

# 65. Error Codes

تقسيم مقترح:

```text
0x0000–0x00FF  transport
0x0100–0x01FF  protocol
0x0200–0x02FF  security
0x1000+        application
```

---

# 66. Application API

Rust API يجب أن تكون semantic وليس packet-centric.

مقترح:

```rust
send_unreliable(data)
send_sequenced(key, seq, data)
send_reliable_unordered(data)
send_reliable_ordered(stream_or_group, data)
```

والـ state API:

```rust
send_state(entity_id, generation, sequence, deadline, data)
send_event(event_id, data)
send_rpc(rpc_id, data)
```

---

# 67. Poll / Flush Model

يمكن توفير نموذجين:

```rust
poll()
flush()
```

أو event-driven runtime adapter.

الـ core لا يجب أن يعرف هل التطبيق يستخدم:

```text
Tokio
Monoio
io_uring
custom event loop
```

---

# 68. Tick Integration

الـ transport لا يفرض game tick.

مثال:

```text
60 Hz simulation
30–120 Hz network send
```

حسب state وnetwork budget.

API مقترح:

```text
game_tick()
process_network()
flush()
```

لكن يمكن تغيير الترتيب حسب engine.

---

# 69. Input Pipeline

يفضل أن تدخل player input بسرعة:

```text
input
 ↓
sequencing
 ↓
queue
 ↓
network send budget
```

ويمكن إضافة redundant recent inputs ضمن حدود congestion budget.

---

# 70. Snapshot Pipeline

يفضل:

```text
world state
 ↓
delta / compression at game layer
 ↓
state generation
 ↓
sequenced unreliable
 ↓
deadline
 ↓
scheduler
```

GTP لا يفرض كيفية توليد delta compression.

---

# 71. Bulk / Cosmetic

يمكن أن تكون هناك queue منخفضة الأولوية:

```text
bulk/cosmetic
```

يجب أن تكون أول من يتعرض للتخفيض عند ضغط queue.

---

# 72. Shared Congestion Controller

كل logical traffic يشترك في:

```text
one connection
one path
one congestion controller
one pacing model
```

إذا فُتحت عدة sockets/flows لجلسة واحدة دون حاجة، قد ينكسر fairness؛ لذلك يجب أن يكون aggregate congestion behavior واضحاً.

---

# 73. Connection Affinity

يجب أن يحاول الخادم:

```text
CID
 ↓
worker
```

ثم تبقى connection على worker نفسه قدر الإمكان.

يمكن إعادة توزيعها فقط لأسباب تشغيلية مثل:

- core imbalance.
- overload.
- migration.

---

# 74. Multi-Core Scaling

نموذج:

```text
NIC queues
    ↓
RSS
    ↓
workers
    ↓
connection ownership
```

التصميم يستهدف scaling قريباً من linear حتى نقطة اختناق أخرى، لكن لا تعتبر linearity نتيجة مضمونة قبل القياس.

---

# 75. Server Fan-Out

في 256-player مثلاً:

```text
world state
  ↓
shared snapshot representation
  ↓
per-client relevance/delta
  ↓
batched packet construction
```

ويجب تجنب serialization كامل من الصفر لكل client إذا كان يمكن مشاركة أجزاء قابلة لإعادة الاستخدام.

---

# 76. Packet Batching

TX يجب أن يسمح:

```text
multiple logical packets
 → batch
 → one backend operation
```

ولكن batching must obey:

- pacing.
- MTU segmentation.
- per-path rules.
- deadlines.

---

# 77. Linux Backend Modes

الطبقات المقترحة:

```text
L0  Portable UDP
L1  Linux UDP + batch syscalls
L2  UDP GSO/GRO
L3  io_uring
L4  AF_XDP (future)
L5  DPDK (future)
```

ليست كل التطبيقات تحتاج L5.

---

# 78. لماذا لا نبدأ بـ DPDK

لأن kernel/network stack قد لا يكون bottleneck الرئيسي.

المنهج:

```text
correct core
 ↓
profile
 ↓
identify bottleneck
 ↓
optimize syscall/batching
 ↓
GSO/GRO/io_uring
 ↓
only then consider kernel bypass
```

---

# 79. Backend Capability Matrix

كل backend يجب أن يعلن capabilities:

```text
batch_rx
batch_tx
gso
gro
sendmsg
recvmsg
zerocopy_tx
multishot_rx
fixed_buffers
kernel_bypass
```

ثم GTP يختار fast path دون تغيير protocol semantics.

---

# 80. Rust Crate Strategy

## Core

```text
std/core
bytes
zerocopy
small fixed collections where justified
```

## IO

```text
socket2
io-uring
Tokio adapter
Monoio adapter
```

## Crypto

```text
rustls / rustls primitives where suitable
aws-lc-rs or ring depending deployment
```

## Testing

```text
proptest
fuzzing
criterion
perf/flamegraph externally
```

يجب ألا تُقفل architecture على dependency واحدة إذا لم تكن ضرورية.

---

# 81. `zerocopy`

نسخة حديثة من `zerocopy` توفر typed byte conversions وno_std-oriented design وtraits مثل:

```text
TryFromBytes
FromBytes
IntoBytes
```

وهي مناسبة لبناء byte views، لكن كل data القادمة من الشبكة يجب التحقق منها قبل اعتبارها valid protocol structure.

---

# 82. `socket2`

يسمح بالوصول إلى socket operations المتقدمة مثل:

```text
sendmsg
vectored send
socket options
```

مع portability أفضل من استدعاء libc مباشرة في كل المشروع.

---

# 83. `io-uring`

يجب استغلاله عندما:

- Linux kernel مناسب.
- workload packet-heavy.
- multishot receive فعلاً يقلل overhead.
- buffer management تحت السيطرة.

ويجب fallback gracefully عند غياب kernel features المطلوبة.

---

# 84. `monoio`

مرشح قوي لنسخة high-performance runtime لأن نموذجه thread-per-core يطابق connection affinity في GTP.

لكن `gtp-core` لا ينبغي أن يعتمد على Monoio مباشرة.

---

# 85. Tokio Adapter

يجب توفير adapter رسمي لتسهيل integration في engines/services تستخدم Tokio.

قد يكون أبطأ من custom runtime في بعض packet-rate workloads؛ يجب قياس ذلك بدلاً من افتراضه.

---

# 86. `s2n-quic` كمرجع هندسي

` s2n-quic` مهم كـ implementation reference لأنه يجمع عدة خصائص نحتاج إلى دراستها:

- CUBIC.
- pacing.
- GSO.
- PMTU discovery.
- connection IDs.
- extensive testing/fuzzing.

لكن GTP لا يعتمد عليه كـ protocol engine.

---

# 87. `quinn` كمرجع Rust QUIC

Quinn مرجع مفيد لفهم:

- Rust API design.
- QUIC datagrams.
- asynchronous integration.
- stream/datagram separation.

لكن GTP يبقى message-semantics-first.

---

# 88. Security Library Strategy

لا يجب أن يصبح crypto API جزءاً من application semantics.

يفضل abstraction:

```rust
trait PacketProtector {
    fn seal(...);
    fn open(...);
}
```

ويمكن تبديل backend وفق target/CPU/security policy.

---

# 89. Crypto Batching

عند ارتفاع packet rate يجب دراسة:

- batching.
- hardware acceleration.
- vectorized operations.
- avoiding repeated key setup.

لكن لا ينبغي التضحية بالـ protocol security من أجل micro-optimization.

---

# 90. API Threading Contracts

الاتجاه الافتراضي:

```text
one connection → one owner thread
```

وعند الحاجة إلى multi-producer:

```text
MP application queues
→ owner worker
```

بدلاً من shared locking على كل message.

---

# 91. Telemetry

كل connection يوفر counters/timestamps لـ:

```text
RTT
min_rtt
smoothed_rtt
rttvar
cwnd
inflight
pacing_rate
loss_rate
retransmissions
ECN CE
send queue
receive queue
stale drops
deadline misses
```

---

# 92. Gameplay Telemetry

أهم metrics ليست network throughput فقط.

يجب قياس:

```text
input_to_server_latency
server_to_client_latency
snapshot_age
stale_packet_rate
deadline_miss_rate
useful_delivery_ratio
```

## Useful Delivery Ratio

مؤشر مقترح:

```text
useful delivered state
----------------------
all delivered state
```

لفهم ما إذا كان transport يستهلك bandwidth في state أصبحت قديمة.

---

# 93. Logging

Production يجب ألا يسجل كل packet.

بدلاً من ذلك:

```text
sampling
aggregated metrics
rare error traces
```

Debug mode يمكن تفعيل:

```text
packet trace
ACK trace
CC trace
scheduler trace
path trace
```

---

# 94. Error Handling

الأخطاء تنقسم إلى:

```text
recoverable packet errors
connection errors
protocol violations
security errors
application errors
```

packet malformed غالباً لا يستلزم close فوراً إذا كان قد يكون مجرد packet غير موثوق، بينما critical protocol violations قد تفعل close.

---

# 95. Duplicate Handling

UDP يسمح بالتكرار والتأخير.

يجب أن يكون كل unreliable/reliable semantics robust ضد duplicates.

استخدم:

```text
packet number windows
message IDs
state sequence
```

حسب نوع الرسالة.

---

# 96. Reordering

يجب أن يتحمل GTP reordering الطبيعي.

لكن reorder threshold يجب أن يكون قابلًا للضبط.

الهدف هو عدم إعلان packet lost مبكراً جداً في مسارات تعاني reorder.

---

# 97. Wireless Networks

يجب اختبار:

- burst loss.
- jitter.
- variable RTT.
- transient outage.
- path change.

لا ينبغي افتراض أن كل loss congestion، ولا ينبغي أيضاً إخفاء loss عن CC بشكل مفرط.

---

# 98. Bufferbloat

يجب اعتبار RTT inflation إشارة مهمة.

عند:

```text
RTT >> min_rtt
```

يجب أن يتعامل controller/scheduler مع احتمال queue buildup.

Game state stale drop يمكن أن يعمل مع CC لتقليل application-induced queueing.

---

# 99. Keepalive

لا يجب إرسال keepalive aggressive.

يحدد deployment سياسة حسب NAT/middlebox behavior.

الـ protocol يمكن أن يستخدم PING ضمن حدود معقولة، مع فصل:

```text
liveness probe
NAT maintenance
path validation
```

حتى لا تصبح كلها نفس الآلية.

---

# 100. Anti-Amplification

قبل path validation يجب أن يطبق server حد amplification.

لا ينشئ server response ضخمة من packet صغيرة مجهولة.

هذه القاعدة مهمة خصوصاً على Internet.

---

# 101. Rate Limiting

يجب أن تدعم طبقة endpoint:

```text
per source IP
per prefix
per CID
per connection state
```

limits مناسبة لمنع abuse دون ضرب اللاعبين الطبيعيين.

---

# 102. Server Architecture

المعمارية المرجعية:

```text
                 GAME SERVER
                     |
                GTP API
                     |
          +----------+----------+
          |                     |
     Message Engine        Control Plane
          |
      Scheduler
          |
      TX Budget
          |
      CC + Pacing
          |
      Packet Builder
          |
      Crypto/AEAD
          |
      I/O Backend
          |
         NIC
```

---

# 103. RX Pipeline

```text
NIC
 ↓
GRO / recv batch
 ↓
minimal parse
 ↓
CID lookup
 ↓
rate/replay checks
 ↓
AEAD
 ↓
ACK processing
 ↓
loss processing
 ↓
frame dispatch
 ↓
message semantic dispatch
 ↓
Game
```

---

# 104. TX Pipeline

```text
Game
 ↓
message queue
 ↓
stale expiration
 ↓
deadline/priority scheduler
 ↓
congestion budget
 ↓
pacing
 ↓
packet builder
 ↓
AEAD
 ↓
GSO/batched send if available
 ↓
UDP
 ↓
NIC
```

---

# 105. Hot Path Rule

كل packet processing يجب أن يسعى إلى:

```text
no heap allocation
no blocking
minimal branches
minimal copies
local state access
batch processing
```

لكن يجب ألا تصبح هذه القاعدة سبباً في code معقد غير قابل للاختبار.

---

# 106. Unsafe Rust Policy

يجب أن يكون `unsafe` محصوراً في:

```text
FFI
OS-specific fast path
verified zero-copy primitives
hardware/kernel integrations
```

ويتم عزله في modules صغيرة ذات invariants موثقة.

الـ protocol logic الأساسي يجب أن يكون safe Rust قدر الإمكان.

---

# 107. Compile-Time Invariants

Rust يجب أن يستعمل لضمان:

- packet state transitions.
- ownership.
- lifetimes.
- valid enum states.
- separation بين validated/unvalidated buffers عندما يمكن.

مثال مفاهيمي:

```rust
UnvalidatedPacket
    ↓ authenticate
AuthenticatedPacket
    ↓ parse
ParsedPacket
```

هذا أفضل من تمرير `Vec<u8>` في كل مكان.

---

# 108. Data Structures

يفضل hot-path structures الصغيرة مثل:

```text
ring buffers
fixed arrays
small vectors
slabs
intrusive queues where justified
```

ولا تستخدم hash maps في كل عملية packet إذا كان يمكن استخدام indexing/routing table أكثر كفاءة.

---

# 109. Connection Lookup

يجب أن يكون lookup سريعاً.

الهدف:

```text
CID
 ↓
worker-local lookup
```

بدلاً من global lock.

يمكن أن يكون:

```text
hash table
sharded table
direct routing encoding
```

حسب CID design والعدد المتوقع للاتصالات.

---

# 110. Session Table

ينبغي أن تدعم:

```text
connection creation
lookup
retirement
timeout
migration
```

مع garbage collection تدريجي وليس pause كبيراً.

---

# 111. Timer Architecture

لا ينصح بـ timer object مستقل لكل connection إذا كان العدد ضخماً.

يفضل:

```text
timing wheel
hierarchical wheel
bucketed timers
```

للأحداث مثل:

- ACK delay.
- PTO.
- deadline.
- keepalive.
- idle timeout.
- MTU probe.

---

# 112. Deadline Timer

لا يجب أن تعني كل message deadline timer مستقل.

يفضل queue/bucket approach:

```text
time bucket
  ↓
messages expiring soon
```

وهذا يمنع timer explosion.

---

# 113. Receive Flow Control

GTP لا يحتاج QUIC stream flow control في نفس الصورة، لكن يحتاج حماية RX queue من memory exhaustion.

يجب وجود:

```text
per-connection RX cap
per-worker RX cap
endpoint cap
```

---

# 114. Send Flow Control

هناك مستويان:

```text
application backpressure
transport congestion control
```

لا ينبغي الخلط بين:

```text
receiver memory pressure
network congestion
```

---

# 115. Application Backpressure API

يجب أن يعرف التطبيق عند رفض/تأخير message:

```text
accepted
queued
expired
rejected due to pressure
```

هذا مفيد خصوصاً في bulk/cosmetic queues.

---

# 116. Message Admission Control

قبل enqueue:

```text
if expired => reject
if queue over limit and low utility => reject
if critical => admit subject to hard bounds
```

وهذا أفضل من ملء queue ثم إسقاطها لاحقاً.

---

# 117. Priority Must Not Become Starvation

P0 control لا يعني تجاهل كل شيء إلى الأبد.

لذلك scheduler يحتاج:

```text
hard priority
plus
fairness budget
```

---

# 118. Reliable Ordered Scoping

لا يُفضل ordered stream واحد عالمي.

بدلاً من ذلك يمكن أن تكون هناك ordered groups مستقلة:

```text
Group A
Group B
Group C
```

بحيث لا يمنع فقد event في Group A تقدم Group B.

هذه نقطة جوهرية لتقليل HoL.

---

# 119. Reliable Unordered Delivery

لا يجب أن تنتظر message 100 من أجل 101.

كل واحدة لها delivery state مستقلة.

إذا وصلت 101:

```text
deliver 101
```

حتى لو كانت 100 مفقودة، ثم تعاد 100 لاحقاً.

---

# 120. Ordered Group State

كل ordered group يحتاج:

```text
next_expected
received out-of-order set
pending reliable messages
```

ويجب وضع cap يمنع attacker من إنشاء huge gap state.

---

# 121. Packet Number Spaces

GTP يمكن أن يستخدم packet number space موحداً في v1 لتقليل التعقيد، ما لم تظهر حاجة أمنية/handshake واضحة للفصل.

أما loss detection فيتعامل مع handshake/control packets بحذر منفصل عند الحاجة.

---

# 122. Control Frames

Control frames يمكن أن تكون:

```text
ACK
PING
PATH_CHALLENGE
PATH_RESPONSE
CLOSE
MTU_PROBE
ACK_FREQUENCY
```

ويجب إعطاؤها priority مرتفعاً لكن مع rate limiting.

---

# 123. MTU Probe

يجب ألا تُعتبر probe packet game data.

وتحتاج:

```text
probe identifier
size
path
ack evidence
```

---

# 124. ACK-only Packet Policy

لا يجب أن تصبح ACK-only packets نسبة كبيرة من traffic.

يمكن piggyback ACK على outgoing data عندما يكون ذلك ممكناً.

---

# 125. ACK Piggybacking

يفضل:

```text
outgoing DATA available
   ↓
attach ACK
```

بدلاً من إرسال ACK منفصل.

لكن لا يجب تأخير ACK بما يضر loss detection/RTT estimation.

---

# 126. Packet Coalescing

يمكن وضع frames متعددة في packet واحد:

```text
ACK + INPUT + STATE
```

بحسب budget.

لكن ينبغي تجنب packetization التي تجعل loss في frame واحدة يؤثر على semantics غير مرتبطة بها.

---

# 127. Small Packet Optimization

لـ packets الصغيرة، الهدف تقليل:

- syscall count.
- allocations.
- copies.
- crypto setup.
- locks.

هذا أهم من الوصول إلى zero-copy كامل في كل حالة.

---

# 128. Large Packet Optimization

عند packets الكبيرة/البث batch:

- GSO.
- vectored I/O.
- zero-copy عند جدواه.
- batching.

يمكن أن تكون أكثر فائدة.

---

# 129. Zero-Copy Tradeoff

لا يجب افتراض:

```text
zero copy == always faster
```

للـ small game packets قد تكون تكلفة إدارة pages/buffers أو completion أعلى من تكلفة copy صغير.

لذلك يجب تحديد strategy حسب workload measured.

---

# 130. CPU Cache Strategy

يجب إبقاء hot connection state متجاورة قدر الإمكان.

لكن تجنب struct عملاق.

يفضل:

```text
small hot struct
pointers/indexes to cold state
```

مع تجنب indirection المفرط أيضاً.

---

# 131. False Sharing

يجب تجنب وضع counters التي يتم تعديلها على cores مختلفة في cache line واحدة.

يمكن استخدام cache-line padding حيث يقاس أنه مفيد.

---

# 132. Atomic Usage

القاعدة:

```text
prefer thread-local state
```

والـ atomics تستخدم فقط عند الحاجة الفعلية:

- cross-core metrics.
- lifecycle flags.
- shared endpoint counters.

وليس في كل packet.

---

# 133. Scheduler Complexity

لا ينبغي أن يصبح scheduling O(log N) لكل micro event إذا كان ذلك مكلفاً في high packet-rate.

يمكن استخدام:

```text
bucketed deadlines
priority rings
small heaps
```

حسب workload.

---

# 134. Benchmarking Philosophy

يجب عدم قياس GTP فقط ضد throughput.

القياسات الرئيسية:

```text
P50 latency
P95 latency
P99 latency
P99.9 latency
CPU/core
cycles/packet
packets/sec/core
allocations/packet
copies/packet
goodput
loss recovery latency
fairness
freshness utility
```

---

# 135. Packet Size Matrix

يجب اختبار:

```text
64 B
128 B
256 B
512 B
768 B
1200 B
1400 B
```

بحسب protocol overhead وMTU.

---

# 136. Packet Rate Matrix

اختبارات مثل:

```text
10 Kpps
50 Kpps
100 Kpps
250 Kpps
500 Kpps
1 Mpps+
```

مع اختلاف عدد الاتصالات.

---

# 137. RTT Matrix

```text
5 ms
20 ms
50 ms
100 ms
200 ms
```

مع jitter.

---

# 138. Loss Matrix

```text
0%
0.1%
1%
2%
5%
10%
```

مع burst-loss scenario.

---

# 139. Reordering Matrix

```text
0%
1%
5%
10%
```

ومزيج reorder + loss.

---

# 140. Bandwidth Matrix

```text
10 Mbps
50 Mbps
100 Mbps
1 Gbps
10 Gbps
100 Gbps lab
```

لكن لا ينبغي اعتبار 100Gbps Internet realistic assumption؛ هو stress test للـ implementation path.

---

# 141. Workload Matrix

## FPS

```text
60/120 Hz
64/128 players
```

## Racing

```text
60–120 Hz
```

## Battle Royale

```text
100–300 players
```

## MMO

```text
large fan-out
high concurrency
```

---

# 142. Mixed Traffic Benchmark

سيناريو أساسي:

```text
70% realtime state
20% reliable gameplay
5% control
5% bulk/cosmetic
```

ثم يتم تعريضه إلى:

```text
1% loss
50 ms RTT
jitter
queue pressure
```

هذا benchmark أهم من benchmark منفصل لكل class.

---

# 143. Failure Scenarios

يجب اختبار:

```text
NAT rebinding
server restart
packet duplication
reordering
delayed packet
path failure
temporary congestion
Wi-Fi transition
mobile network transition
worker overload
queue overflow
```

---

# 144. Soak Tests

يجب وجود tests لساعات/أيام:

- long-lived connections.
- memory stability.
- timer stability.
- sequence number wrap behavior.
- reconnect loops.
- NAT refresh.

---

# 145. Fuzzing

الهدف:

```text
wire parser
frame parser
varints
ACK ranges
CID handling
state machines
crypto framing
```

يجب ألا يصل malformed packet إلى panic.

---

# 146. Property Tests

أمثلة:

```text
encode(decode(packet)) == canonical packet

out-of-order reliable messages eventually deliver once

expired state is never delivered

packet duplicate never causes duplicate semantic delivery
```

بحسب semantics.

---

# 147. Model Testing

حالات state machine:

```text
Initial
Handshaking
Validated
Established
Migrating
Closing
Closed
```

يجب اختبار transitions غير القانونية.

---

# 148. Correctness Priority

ترتيب التطوير:

```text
1 correctness
2 loss/recovery
3 CC behavior
4 scheduler
5 observability
6 kernel optimization
7 DPDK/kernel bypass
```

لا يجوز تحسين hot path قبل تثبيت semantics الأساسية.

---

# 149. Reference Implementations

يجب وجود:

```text
reference single-thread
performance Linux
```

بنفس protocol core.

reference implementation تسهل debugging والـ interoperability tests.

---

# 150. Versioned Test Vectors

يجب تخزين:

```text
handshake vectors
packet vectors
ACK vectors
loss scenarios
crypto vectors
migration vectors
```

وتشغيلها في CI.

---

# 151. CI Requirements

كل merge مهم يجب أن يجتاز:

```text
unit tests
property tests
fuzz smoke
wire tests
benchmark regression
clippy
fmt
miri/unsafe validation where appropriate
```

---

# 152. Performance Regression Gates

يجب منع regression مثل:

```text
+20% cycles/packet
+20% allocations
+15% p99 latency
```

لكن thresholds النهائية تحدد بعد baseline أولي.

---

# 153. Observability Cost

telemetry نفسها لا يجب أن تضيف overhead كبيراً.

يجب استخدام:

```text
sampling
per-core counters
batched export
```

بدلاً من lock عالمي في كل packet.

---

# 154. Metrics Aggregation

كل worker ينتج:

```text
local metrics
```

ثم تجمع دورياً.

هذا يقلل atomic contention.

---

# 155. Runtime Configuration

الأشياء القابلة للضبط:

```text
ACK frequency
max burst
queue limits
deadline policy
scheduler weights
MTU probing
CC algorithm
keepalive policy
```

لكن يجب ألا تسمح dynamic configuration بتغيير state invariants الخطرة أثناء الاتصال بدون قواعد واضحة.

---

# 156. Config Profiles

مقترح:

```text
GAME_COMPETITIVE
GAME_CASUAL
LAN_LOW_LATENCY
SERVER_HIGH_FANOUT
```

كل profile يحدد defaults فقط؛ protocol semantics لا تتغير جذرياً.

---

# 157. Competitive Profile

الأولوية:

```text
fresh input/state
low p99
fast loss detection
moderate ACK frequency
strict stale-drop
```

---

# 158. High Fan-Out Profile

الأولوية:

```text
batching
CPU efficiency
serialization reuse
worker affinity
GSO/GRO
```

---

# 159. LAN Profile

قد يسمح:

```text
higher MTU
less conservative probing
optional plain/authenticated benchmark mode
```

لكن لا يغيّر wire semantics الأساسية.

---

# 160. Internet Profile

يجب أن يدعم:

```text
AEAD
anti-amplification
path validation
NAT rebinding
fair congestion control
safe MTU
```

---

# 161. Bulk Traffic

في GTP v1 لا نعتمد عليه لنقل bulk ضخمة.

إذا استُخدم:

```text
low priority
separately scheduled
strict fairness
reliable
```

حتى لا يضرب gameplay traffic.

---

# 162. Compression

لا يفرض protocol compression.

game/application layer هو الأنسب لـ:

```text
delta compression
quantization
state encoding
```

الـ transport يمكنه لاحقاً الإعلان عن compressed payload لكنه لا ينبغي أن يضيف compression تلقائياً لكل packet.

---

# 163. Header Compression

لا توجد حاجة إلى header compression معقدة في v1.

الهدف الأساسي هو short header صغير أصلاً.

---

# 164. DSCP

يمكن إتاحة DSCP كـ deployment feature، لكن يجب عدم افتراض أن network سيحترمه.

ولا يجب استخدامه كبديل للـ congestion control.

---

# 165. ECMP

عند استخدام عدة flows/ports يجب الانتباه إلى إعادة الترتيب.

GTP يفضل اتصالاً واحداً منطقيًا لكل session ما لم توجد حاجة واضحة.

---

# 166. Multiple Sockets

إذا احتاج server sockets متعددة لأسباب scaling، يجب الحفاظ على aggregate congestion semantics لكل connection/network flow model المناسب.

لا يجوز إنشاء عدة uncontrolled flows فقط للحصول على throughput أعلى.

---

# 167. Worker Rebalancing

في v1 يمكن إبقاء connection pinned.

Future:

```text
worker overload
 ↓
controlled migration
 ↓
ownership transfer
```

مع تجنب نقل hot state المتكرر.

---

# 168. Memory Limits Under Attack

كل peer غير موثوق يجب أن يكون له bounds على:

```text
handshake state
RX buffered data
reorder state
reliable outstanding messages
ACK ranges
fragments
```

---

# 169. ACK Range Limits

يجب وضع cap على عدد ranges في ACK.

عند تجاوز الحد يمكن:

```text
truncate older ranges
```

وفق سياسة لا تضر loss detection بشكل غير مقبول.

---

# 170. Reliable Message Size Limits

يجب أن توجد limits:

```text
max_message_size
max_fragments
max_outstanding_bytes
```

وتختلف حسب configuration.

---

# 171. Reassembly Protection

لا يجوز الاحتفاظ برسالة fragmented إلى أجل غير محدود.

كل reassembly له:

```text
deadline
memory cap
fragment count cap
```

---

# 172. Security and Performance Balance

الأمان ليس مرحلة منفصلة عن performance.

الهدف:

```text
secure fast path
```

وليس:

```text
security off => performance on
```

---

# 173. Fast Authentication Failure

يجب أن يكون فشل authentication cheap نسبياً بعد minimal filtering.

يجب منع attacker من تحويل connection table إلى crypto workload غير محدود.

---

# 174. Key Rotation

يمكن دعم مفاهيم key phase/rotation مستقبلاً.

يجب تصميم packet protection بحيث لا يفترض أن connection key واحدة تبقى للأبد.

---

# 175. Replay Protection

يجب أن يملك receiver نافذة packet-number/replay مناسبة، مع مراعاة reordering الطبيعي.

---

# 176. Close Semantics

```text
CLOSE
  error code
  optional reason
```

reason النصي غير ضروري في hot path.

---

# 177. Graceful Close

يوفر:

```text
application close
protocol close
idle timeout
security close
```

مع حد زمني واضح وعدم الانتظار إلى ما لا نهاية.

---

# 178. Idle Timeout

لكل connection:

```text
last_rx
last_tx
idle_deadline
```

وعند expiry:

```text
close/reclaim
```

---

# 179. Reconnection

GTP protocol core لا يفرض reconnect semantics، لكن API يجب أن تجعل reconnect سريعاً ولا تخلط session identity الجديدة مع القديمة.

---

# 180. Game Session vs Network Session

يجب فصل:

```text
Game Session ID
Network Connection ID
```

يمكن أن يستمر game session فوق connection replacement عند design application مناسب، لكن لا يعني ذلك أن transport يحتفظ تلقائياً بحالة اللعبة.

---

# 181. Server Restart

v1 transport لا يضمن transparent server restart.

يمكن لاحقاً بناء:

```text
session resumption
```

لكنها ليست جزءاً mandatory من transport core.

---

# 182. Reliability Semantics vs Game Authority

GTP لا يقرر صحة game state.

مثلاً reliable event:

```text
DamageEvent
```

transport guarantees delivery semantics، not game correctness.

---

# 183. Security Boundary vs Trust Boundary

GTP transport authenticity means packet came from authenticated peer/context. لا يعني أن application payload موثوق منطقياً.

Game server يجب أن يطبق validation مستقلاً.

---

# 184. Protocol Invariants

أهم invariants:

```text
packet number uniqueness within required space
CID routing consistency
no state delivery after explicit expiration
no reliable double-delivery
no ordered delivery before predecessor
no retransmission after terminal expiration
cwnd is never exceeded by transport flight accounting
path switch requires validation
```

---

# 185. Packet Number Wrap

يجب أن يحدد wire format حجم packet number بحيث يوفر مجالاً عملياً كافياً، مع سياسة واضحة قبل wrap.

لا يجب أن يصبح wrap event مفاجئاً للـ application.

---

# 186. Sequence Number Comparison

state sequence numbers تحتاج comparison modulo-safe إذا استخدمت مساحة محدودة.

كل implementation يجب أن يملك utility موحدة بدلاً من تكرار logic في كل message class.

---

# 187. Clock Model

الـ transport يحتاج clock monotonic للـ:

- RTT.
- pacing.
- deadline.
- timeout.

ولا يستخدم wall clock في حسابات network timing الحساسة.

---

# 188. Timestamp Width

الـ wire timestamp يجب ألا يكون مرتبطاً مباشرة بـ system wall-clock.

الأفضل استخدام relative/compact timing encoding حيث تكون الدقة والـ wrap واضحة.

---

# 189. Timestamp Usage

لا ينبغي الاعتماد على sender timestamp وحده لقياس RTT؛ يجب أن تستخرج RTT samples من packet/ACK events وفق state المعروفة للطرفين.

---

# 190. Time Precision

في الـ implementation يجب أن تكون monotonic timestamps عالية الدقة بما يكفي للـ local profiling.

لكن لا يجب إرسال nanoseconds كاملة على wire دون فائدة واضحة.

---

# 191. Scheduler Prediction

يمكن للـ scheduler مستقبلاً تقدير:

```text
expected_delivery = now + RTT/2 + queue_delay
```

ثم مقارنة ذلك بالـ deadline.

إذا لم يعد delivery المتوقع مفيداً، يتم إسقاط الرسالة قبل إرسالها.

هذه وظيفة Game-aware أساسية.

---

# 192. Deadline Admission

عند enqueue:

```text
if expected transmission + expected path delay > deadline:
    reject/drop if semantics permit
```

يجب عدم حساب ذلك بدقة زائفة؛ هو heuristic مبني على estimates.

---

# 193. Reliable Message Deadline

حتى reliable message يمكن أن تملك deadline.

مثال:

```text
reliable = true
deadline = now + 500ms
```

إذا انتهى deadline قبل delivery، يجوز إسقاط retransmissions بدلاً من إنشاء traffic عبثي.

---

# 194. Ordered Group Expiration

إذا انتهت رسالة ordered predecessor، لا ينبغي أن تتوقف المجموعة إلى الأبد.

يحتاج protocol/application إلى policy:

```text
skip
reset group
close group
```

بحسب semantics.

---

# 195. Ordered Group Reset

future extension:

```text
ORDER_RESET(group_id, new_sequence)
```

يمكن أن يسمح بإلغاء gap قديم دون إغلاق الاتصال كله.

---

# 196. Message Cancellation

يجب توفير API داخلي/تطبيقي:

```text
cancel(message_id)
```

مفيد إذا تغيرت اللعبة ولم تعد الرسالة مطلوبة قبل إرسالها.

---

# 197. Retransmission Cancellation

إذا أرسل application state أحدث، قد تلغى retransmission لstate قديمة:

```text
state generation 55
replaces generation 54
```

فتصبح retransmission للـ54 غير مفيدة ويمكن إلغاؤها.

---

# 198. Reliable Event Supersession

ليست كل reliable events قابلة للاستبدال.

لذلك يجب أن تحدد application:

```text
supersedable = yes/no
```

حتى لا تسقط event لها semantics لا تسمح بذلك.

---

# 199. Transport Semantics Contract

كل API send يجب أن توضح:

```text
reliability
ordering
freshness
deadline
cancellation
supersession
priority
```

هذا يقلل سوء استخدام transport من game code.

---

# 200. Final Message API Model

نموذج موحد مفاهيمياً:

```rust
MessageOptions {
    class,
    priority,
    deadline,
    state_key,
    sequence,
    generation,
    supersedable,
}
```

والـ transport يترجم options إلى queue/recovery policy المناسبة.

---

# 201. Data-Oriented Design

لا يجب أن يكون Connection object محور كل العمل بصورة object-heavy.

الأفضل تنظيم state حسب الوصول:

```text
RX hot arrays
TX hot arrays
loss records
scheduler records
cold metadata
```

هذا قد يكون أكثر cache-efficient في server كبير.

---

# 202. Struct of Arrays vs Array of Structs

لا يوجد قرار واحد mandatory.

يستخدم SoA عندما تتم معالجة نفس الحقل عبر عدد كبير من records، وAoS عندما يتم التعامل مع record كاملة كوحدة.

يجب الحسم profiling-driven.

---

# 203. Branch Prediction

الـ fast path يجب أن يجعل common cases common:

```text
valid packet
known CID
established connection
authenticated
fresh message
no loss
```

أما malformed/rare path فتعزل قدر الإمكان.

---

# 204. Slow Path Isolation

يجب فصل:

```text
HOT PATH
  normal packet

SLOW PATH
  malformed
  migration
  MTU probe
  extension
  close
  handshake
```

بحيث لا تزيد rare branches من كلفة packet العادي دون سبب.

---

# 205. Parser Fast Path

يمكن اعتماد fixed prefix للـ common packet.

إذا لم توجد:

```text
ACK
extension
fragment metadata
```

فلا يجب أن يمر packet خلال parser ثقيل بلا داعٍ.

---

# 206. Header Variants

يمكن وجود:

```text
Short header
Long header
```

مع optional sections.

لكن يجب ألا تنفجر عدد variants إلى عشرات الحالات؛ كل variant يزيد testing burden.

---

# 207. Varint Policy

استخدم varints فقط حيث توفر savings حقيقية.

الأرقام التي تحتاج غالباً إلى fixed width في hot path يمكن أن تبقى fixed width لتبسيط parsing.

---

# 208. ACK Range Encoding

يمكن استخدام compact gap/range representation قريباً من QUIC، لكن دون نسخ كافة semantics غير اللازمة.

الهدف:

```text
small ACK for sparse losses
```

مع cap على عدد ranges.

---

# 209. ACK Compression Defense

لا يجب السماح لpeer بإرسال ACK structures ضخمة بشكل غير محدود.

receiver يجب أن يطبق:

```text
max_ack_ranges
max_ack_bytes
```

---

# 210. ACK Delay Signaling

عند استخدام adaptive ACK frequency يجب أن تكون هناك إشارة واضحة للـ sender عن delay policy.

هذا يساعد RTT estimator على عدم تفسير delayed ACK كـ network latency كاملة.

---

# 211. Immediate ACK Triggers

قد يحدث immediate ACK عند:

```text
loss suspicion
reordering threshold exceeded
control event
path validation
MTU probe
```

لكن يجب ألا تُطلق هذه الحالات بلا limits.

---

# 212. ACK Frequency Safety

رفع ACK spacing كثيراً قد يؤخر loss detection.

خفضها كثيراً يزيد CPU/network overhead.

لذلك يجب أن توجد bounds:

```text
min_ack_interval
max_ack_interval
max_ack_eliciting_packets
```

---

# 213. Loss Threshold Tuning

Packet threshold لا يجب أن يكون ثابتاً إلى الأبد.

يمكن أن يكون:

```text
base threshold
plus reordering observation
```

ولكن لا بد من منع oscillation أو تأخير detection بشكل مفرط.

---

# 214. Spurious Loss

عندما يصل packet بعد إعلان loss، يجب أن يسجل:

```text
spurious_loss
```

وتستخدم الإشارة لضبط reordering thresholds عند الإمكان.

---

# 215. RTO/PTO Safety Net

حتى مع ACK-based loss detection يجب وجود timeout safety mechanism.

لا يجوز انتظار ACK gap إلى الأبد عندما يصبح الطرف غير مستجيب.

---

# 216. Timeout Behavior

عند timeout:

```text
probe/limited retransmission
reduce congestion state as configured
re-arm timer
```

ولا يعاد إرسال كل queued realtime state.

---

# 217. Timeout for Realtime Data

unreliable fresh state ليس targetاً لـ timeout retransmission.

عند timeout:

```text
send new state
```

إن كانت هناك بيانات أحدث، لا تحاول إنقاذ القديمة.

---

# 218. Recovery Priority

إذا كان هناك lost reliable message وfresh state:

```text
scheduler evaluates both
```

لكن retransmission لا تعطى automatic monopoly.

هذا يمنع reliable backlog من خنق realtime traffic.

---

# 219. Recovery Budget

يمكن تحديد:

```text
retransmission_budget
```

كنسبة/حد من send budget خلال فترة معينة، مع استثناءات للـ control.

هذا يقلل recovery storms.

---

# 220. Recovery Storm Protection

بعد burst loss:

```text
many lost packets
```

لا يجب أن يحاول sender إعادة كل شيء فوراً إذا كانت بعض messages قد أصبحت stale.

يجب ترتيب retransmissions وفق:

```text
deadline
importance
size
age
```

---

# 221. Loss Burst Detection

يمكن تسجيل:

```text
loss_run_length
loss_burst_rate
```

لكن لا يجب افتراض أن burst loss دائماً congestion.

هذه telemetry تساعد policy، ولا تستبدل CC evidence.

---

# 222. Congestion vs Wireless

يمكن للـ CC أن يأخذ في الحسبان:

```text
RTT inflation
ECN
delivery rate
loss pattern
```

لكن لا يملك GTP ضماناً لمعرفة سبب loss بدقة على Internet.

لذلك يجب تجنب heuristics شديدة الثقة.

---

# 223. BBR-like Controller Boundary

إذا أُنشئ GTP-BBR مستقبلاً، يجب أن يكون module مستقل:

```text
gtp-cc-bbr
```

ويرث فقط interface العام.

لا يجب أن تنتشر BBR-specific fields في protocol core.

---

# 224. CUBIC Baseline

CUBIC-compatible behavior يوفر baseline مهم لـ:

- fairness.
- interoperability expectations.
- regression comparisons.

ولا يعني أنه الأفضل لكل game workload.

---

# 225. Initial Congestion Window

يجب اختيار initial cwnd بعناية وفق path conditions وUDP guidance وقياس handshake/game startup.

لا ينبغي تثبيت رقم نهائي دون benchmark وreference analysis.

---

# 226. Slow Start

يجب أن يكون موجوداً أو يتم استبداله بمكافئ controller واضح.

الـ startup السريع أكثر من اللازم قد يسبب burst وloss على paths ضعيفة.

---

# 227. Pacing During Startup

Pacing يجب أن يعمل أثناء startup عندما يمكن ذلك، وليس فقط بعد congestion steady state.

---

# 228. Pacing Granularity

إذا كانت timer granularity منخفضة، قد تظهر bursts صناعية.

يجب أن يدعم implementation:

```text
high resolution monotonic clock
batch deadline scheduling
```

مع تكاليف CPU معقولة.

---

# 229. Busy Polling

Linux deployments قد تدرس busy polling عندما يكون low latency أهم من CPU efficiency، لكن يجب أن يكون feature deployment-specific وليس default.

---

# 230. CPU Pinning

يمكن pin workers إلى CPU cores معينة في high-performance deployment.

لكن protocol لا يجب أن يعتمد على هذا لكي يعمل.

---

# 231. NUMA

على خوادم متعددة NUMA nodes:

```text
NIC queue
→ NUMA-local worker
→ NUMA-local memory
```

يجب تجنب الوصول المتكرر لذاكرة NUMA أخرى.

---

# 232. Memory Locality

Packet/message pools يجب ideally أن تكون NUMA-aware في deployments الكبيرة.

---

# 233. NIC RSS

GTP deployment يجب أن يضبط RSS بحيث لا يوزع packets لنفس connection بصورة سيئة.

إذا احتاج النظام mapping أدق من RSS، يمكن استخدام software steering بعد receive.

---

# 234. Receive Steering

عندما تصل packets إلى worker غير مالك للـ connection:

```text
fast handoff
```

يجب أن يكون الاستثناء، لا الحالة الافتراضية.

---

# 235. Worker Messaging

cross-worker queue يجب أن تكون:

```text
bounded
batchable
low contention
```

ولا تستخدم synchronous locks على hot path.

---

# 236. Endpoint Sharding

يمكن تقسيم endpoint إلى:

```text
worker 0 socket/context
worker 1 socket/context
...
```

بحسب backend/platform capabilities.

---

# 237. UDP Port Strategy

يمكن استخدام port واحد مع connection IDs أو عدة ports في deployments خاصة.

لكن application يجب أن يفهم آثار ECMP وreordering وfirewalls.

---

# 238. Stateless Load Balancer Compatibility

وجود CID opaque وطويل مناسب للـ routing/load balancing.

يمكن أن يكون هناك front-end يختار backend دون إنهاء game session.

---

# 239. Server Sharding

النموذج:

```text
client
 ↓
edge/load balancer
 ↓
CID routing
 ↓
worker/shard
 ↓
game server
```

يجب أن يدعم route stability.

---

# 240. Observability at Edge

Edge لا يحتاج إلى decrypt game payload ليقيس:

```text
packet count
bytes
CID class if safely derivable
path liveness
loss/ACK metadata when exposed by architecture
```

لكن security/privacy يجب أن تحدد ما يمكن كشفه.

---

# 241. Protocol Privacy

CID لا يجب أن يكشف معلومات حساسة.

أي routing token داخلي لا ينبغي أن يكون plaintext قابلًا للتخمين بسهولة إذا كان يمكن أن يساعد abuse.

---

# 242. NAT Mapping Preservation

keepalive strategy يجب أن تكون endpoint-specific وقابلة للضبط.

الاتصال idle لفترات طويلة قد يحتاج PING، بينما active traffic يغني عنه.

---

# 243. Path Challenge Rate Limit

لا يجب إرسال PATH_CHALLENGE بلا حدود استجابة لكل packet غير موثوق.

يجب أن يكون challenge:

```text
bounded
state-light
rate-limited
```

---

# 244. Migration Attack Defense

يجب منع attacker من:

```text
forcing path migration
```

بإجبار server على إرسال traffic إلى عنوان مزور.

لذلك لا يصبح new path active قبل validation.

---

# 245. Stateless Reset / Equivalent

يمكن إضافة آلية مستقبلية لإنهاء connection عندما تكون state غير متاحة، لكن يجب تصميمها بعناية لمنع abuse.

---

# 246. Handshake Amplification

الـ server قبل validation يجب أن يحافظ على response budget.

قد يستخدم cookie/token لإثبات مصدر قابل للوصول.

---

# 247. Cookie Design

cookie يجب أن تكون:

- stateless أو منخفضة state.
- مرتبطة بعناوين/path وفق policy.
- قصيرة زمنياً.
- قابلة للدوران key rotation.

لا ينبغي وضع معلومات حساسة فيها بلا حاجة.

---

# 248. Connection Admission

قبل allocation كامل:

```text
packet sanity
 ↓
rate limits
 ↓
validation
 ↓
connection allocation
```

---

# 249. DoS Resource Model

يجب حساب موارد peer في:

```text
CPU
memory
crypto
queued packets
handshake state
```

وعدم السماح لمورد واحد مخفي مثل ACK ranges بفرض allocation غير محدود.

---

# 250. Resource Accounting

كل connection يجب أن يملك counters:

```text
rx_bytes
rx_packets
tx_bytes
tx_packets
queued_bytes
reassembly_bytes
crypto_failures
protocol_errors
```

لتطبيق quotas.

---

# 251. Reliability State Limits

هناك limits واضحة على:

```text
max unacked messages
max unacked bytes
max retransmission attempts where applicable
max reorder entries
```

---

# 252. Retry Policy

لا يجب أن توجد retransmission loop لا نهائية.

إذا كانت message ذات lifetime منتهية:

```text
stop retry
```

---

# 253. Application Relevance

أفضل transport decision يعتمد أحياناً على metadata يوفرها game engine.

مثلاً:

```text
entity relevance
player visibility
combat criticality
```

GTP يمكن أن يقبل priority/deadline/relevance hints، لكنه لا يحاول فهم game world نفسه.

---

# 254. API Ownership

لمنع lifetime bugs:

```text
application owns source until enqueue accepted
transport owns internal buffer if accepted
```

ويجب أن يكون هذا واضحاً في Rust API.

---

# 255. Borrowed vs Owned Send API

يمكن توفير:

```rust
send_borrowed(&[u8], options)
send_owned(Bytes, options)
```

مع توضيح متى يتم النسخ ومتى تنتقل ownership.

---

# 256. Buffer Lifetime

Borrowed zero-copy send يجب ألا يوهم التطبيق بأن transport سيحفظ reference بعد انتهاء call إذا لم يكن ذلك مدعومًا.

يمكن أن تكون API asynchronous واضحة في ownership.

---

# 257. Receive API

RX يمكن أن يعيد:

```text
BorrowedMessage<'a>
```

داخل callback/processing scope، أو:

```text
OwnedMessage
```

لمن يحتاج retention.

---

# 258. No Hidden Copy

يجب أن تكون API semantics واضحة بشأن copy count.

لا نريد abstraction يبدو zero-copy لكنه ينسخ packet داخلياً بلا علم المستخدم.

---

# 259. Packet Builder

يجب أن يكون builder قادرًا على:

```text
reserve header
append ACK
append frames
seal payload
encrypt
finalize
```

مع buffer reuse.

---

# 260. Deferred Encryption

لا يجب تشفير packet قبل أن يصبح content نهائياً.

يمكن تجميع frames ثم seal مرة واحدة.

---

# 261. AEAD AAD

header fields التي يجب أن تكون authenticated يجب أن تدخل في AAD وفق تصميم ثابت.

أي تعديل بعد seal يجب أن يجعل packet invalid.

---

# 262. Packet Number and Nonce

nonce derivation يجب أن يكون deterministic من connection key/packet number وفق algorithm ثابت يمنع reuse.

---

# 263. Key Phase

future versions قد تحتاج key update.

يجب ترك مساحة في flags/header دون توسيع common header بشكل مبالغ فيه.

---

# 264. Extension Registry

يجب تعريف registry داخلي لأنواع frames والـ transport parameters.

أرقام محجوزة لـ:

```text
core
experimental
private use
```

---

# 265. Experimental Extensions

لا يجوز أن تستخدم experimental feature أرقاماً تتعارض مع future standardized features دون namespace واضح.

---

# 266. Wire Compatibility Policy

v1.x يجب أن يحافظ على:

```text
backward-compatible extensions where practical
```

أما تغيير semantics جوهري فيحتاج version جديد.

---

# 267. Feature Negotiation

الـ handshake يمكن أن يتضمن capabilities:

```text
ACK frequency
GSO-safe batching profile
extensions
FEC
0-RTT
```

لكن capability negotiation لا يعني أن endpoint يملك نفس kernel capabilities؛ تلك local implementation detail.

---

# 268. Local Backend Capability

مثال:

```text
peer supports GTP v1
local supports GSO
peer does not need to know GSO
```

GSO ليس protocol feature بين peerين.

---

# 269. Protocol Parameters vs Runtime Parameters

يجب فصل:

```text
wire-negotiated parameters
local-only parameters
operator configuration
```

حتى لا تنتقل تفاصيل Linux إلى protocol.

---

# 270. Configuration Safety

configuration invalid combination يجب رفضها قبل بدء endpoint.

مثلاً:

```text
max_message_size < minimum fragment overhead
```

---

# 271. Default Policy

Default production profile يجب أن يكون:

```text
secure
congestion-controlled
paced
bounded
adaptive ACK
freshness-aware
```

---

# 272. Secure-by-Default

plain transport لا يكون enabled في public production profile.

---

# 273. Performance-by-Default

لكن security default يجب ألا يؤدي إلى architecture تمنع batching أو zero-copy أو hardware acceleration.

---

# 274. Performance Envelope

الهدف design targets، وليس نتائج مثبتة:

```text
steady-state packet allocation: 0
common-path copy count: 0–1
small hot state: cache-friendly
P99 transport overhead: low single-digit microseconds target in local lab where architecture permits
```

القيمة الأخيرة ليست guarantee ويجب إثباتها عبر benchmark.

---

# 275. Benchmark Baselines

تجب مقارنة:

```text
UDP custom baseline
TCP
KCP v2.1.1
QUIC streams
QUIC DATAGRAM
GTP reference
GTP Linux fast path
GTP io_uring
GTP kernel-bypass experimental
```

---

# 276. Benchmark Fairness

كل protocol يجب أن يحصل على:

```text
same payloads
same RTT
same loss
same CPU
same MTU
same crypto conditions where comparable
```

وإلا تصبح المقارنة غير عادلة.

---

# 277. Crypto Benchmark

يجب توفير:

```text
crypto off
crypto on
crypto batched
```

مع عدم استخدام insecure mode كنتيجة production، بل كـ performance reference فقط.

---

# 278. Scheduler Benchmark

اختبارات:

```text
all fresh
50% stale
90% stale
mixed priorities
deadline collisions
large reliable backlog
```

يجب قياس:

```text
CPU
queue latency
useful delivery
```

---

# 279. Loss Recovery Benchmark

قياس:

```text
loss declaration delay
retransmission delay
gameful recovery delay
stale recovery suppression
```

---

# 280. ACK Benchmark

مقارنة:

```text
ACK every 1
ACK every 2
ACK every 4
ACK every 8
adaptive
```

مع:

```text
CPU
ACK traffic
loss reaction
RTT accuracy
```

---

# 281. GSO/GRO Benchmark

مقارنة:

```text
single send/recv
batch send/recv
GSO/GRO
```

مع packet sizes مختلفة.

---

# 282. io_uring Benchmark

يجب مقارنة:

```text
recvfrom loop
recvmmsg where available
io_uring recv
io_uring multishot recv
```

والنتائج تقاس end-to-end، لا syscall microbenchmark فقط.

---

# 283. Runtime Benchmark

نقارن:

```text
manual poll loop
Tokio
Monoio
native io_uring
```

بنفس protocol workload.

---

# 284. DPDK Benchmark

DPDK لا يقاس فقط Gbps.

يجب قياس:

```text
latency
CPU isolation cost
implementation complexity
packet loss under saturation
application integration cost
```

---

# 285. Kernel Bypass Decision Rule

لا نعتمد DPDK/AF_XDP إلا إذا أثبت profiling أن:

```text
kernel UDP path
```

هو bottleneck المسيطر بعد:

```text
batching
GSO/GRO
socket tuning
i/o_uring
CPU affinity
memory tuning
```

---

# 286. Linux Socket Tuning

backend يجب أن يدعم configuration لمقاييس مثل:

```text
SO_RCVBUF
SO_SNDBUF
busy polling where applicable
socket reuse policy
ECN/DSCP options
```

لكن القيم يجب أن تكون benchmark-driven.

---

# 287. Buffer Sizing

buffer كبير جداً قد يزيد latency تحت congestion.

buffer صغير جداً قد يزيد drops.

لذلك tuning يجب أن يراعي:

```text
cwnd
packet rate
BDP
application queue
```

---

# 288. BDP Awareness

التخطيط الأساسي:

```text
BDP = bandwidth × RTT
```

والمخزون الشبكي لا يجب أن يكون أقل كثيراً من المتطلبات اللازمة لمسار عالي BDP عندما تكون throughput مهمة، لكن game state قد يفضل freshness على ملء BDP بالكامل.

---

# 289. Throughput vs Freshness

GTP لا يحاول دائماً maximize throughput.

هدفه:

```text
maximize useful game information delivered on time
```

وهذا معيار مختلف عن bulk transport.

---

# 290. Useful Throughput

نقترح KPI:

```text
useful_goodput
```

ويحسب فقط bytes التي وصلت ضمن freshness/deadline semantics.

---

# 291. Stale Byte Ratio

مؤشر:

```text
stale_bytes_delivered / total_state_bytes_delivered
```

هدفه أن يكون منخفضاً.

---

# 292. Deadline Miss Ratio

مؤشر:

```text
deadline_missed / deadline_bound_messages
```

ويجب عرضه حسب message class.

---

# 293. Tail Latency by Class

لا يكفي P99 العام.

يجب أن نرى:

```text
P99 input
P99 realtime state
P99 reliable event
P99 control
```

---

# 294. End-to-End Game Latency

يجب الفصل بين:

```text
input capture
network uplink
server queue
server simulation
network downlink
client render/input application
```

GTP يقيس transport components وليس كامل player-perceived latency وحده.

---

# 295. Benchmark Reproducibility

كل benchmark يجب تسجيل:

```text
CPU model
NIC model
kernel version
Rust toolchain
build flags
MTU
network emulator config
packet sizes
traffic profile
```

حتى يمكن إعادة التجربة.

---

# 296. Rust Build Profile

Performance build يجب أن يستخدم optimization مناسبة، وأن تقاس النتائج على:

```text
release
LTO variants where justified
panic policy
CPU target features
```

لكن لا يجوز استخدام build-specific hacks تمنع deployment compatibility بلا سبب.

---

# 297. CPU Feature Detection

crypto/codec fast paths يمكن أن تعتمد على runtime or compile-time CPU feature detection.

core semantics لا تعتمد عليها.

---

# 298. SIMD

يمكن استخدام SIMD في:

```text
crypto
checksums if relevant
bulk parsing
FEC
compression
```

ولكن فقط إذا كانت gains مثبتة.

---

# 299. Branchless Code

لا يجب تحويل كل logic إلى branchless code بلا قياس.

في Rust، readability + correct branch prediction قد تكون أفضل من clever bit hacks.

---

# 300. Unsafe Isolation

كل unsafe block يجب أن يكون:

```text
small
commented
invariant-documented
tested
```

والـ protocol state machine نفسها safe قدر الإمكان.

---

# 301. API Stability

الإصدار الأول يجب أن يحافظ على API واضحة حتى لو تطورت internal implementations.

لا تربط public API بالـ kernel-specific types.

---

# 302. Crate Layering

المقترح النهائي:

```text
gtp-wire
gtp-types
gtp-recovery
gtp-cc
gtp-scheduler
gtp-path
gtp-crypto
gtp-memory
gtp-io
gtp-runtime-tokio
gtp-runtime-monoio
gtp-runtime-uring
gtp-bench
gtp-fuzz
```

يمكن دمج بعض crates أثناء prototype ثم فصلها عندما يستقر التصميم.

---

# 303. Dependency Policy

يجب تقليل dependencies في core حتى:

- يظل audit أسهل.
- build time منخفض.
- binary size معقول.
- behavior deterministic.

---

# 304. Core `no_std` Consideration

يمكن تصميم `gtp-wire` وprimitive types لتكون no_std-compatible حيث يكون ذلك عملياً.

لكن endpoint/server الكامل يحتاج std/OS.

---

# 305. Error Type Design

Rust errors يجب أن تكون structured:

```rust
enum TransportError {
    InvalidPacket,
    AuthenticationFailed,
    ProtocolViolation,
    ResourceLimit,
    PathValidationFailed,
    Timeout,
    Io(...),
}
```

مع عدم إنشاء strings allocations على hot path.

---

# 306. Logging API

يفضل structured events:

```text
connection_created
path_changed
loss_detected
message_expired
connection_closed
```

بدون format strings مكلفة في كل packet.

---

# 307. Metrics API

يمكن استخدام per-worker counters ثم aggregate periodically.

هذا أفضل من global atomic increments في كل packet حيث يكون contention ملموساً.

---

# 308. Debug Trace

يجب أن يكون trace sampling controlled.

مثلاً:

```text
1/1000 packets
```

أو trace per connection/episode.

---

# 309. Production Safety

debug features يجب ألا تكون accidentally enabled في production.

---

# 310. Deployment Profiles

```text
Dev
CI
Staging
Production
Benchmark
```

ولكل profile defaults مناسبة.

---

# 311. Network Emulator

يجب بناء harness يعتمد على Linux `tc/netem` لتوليد:

```text
delay
jitter
loss
reorder
duplicate
rate limit
```

وهذا يتماشى مع test methodology الأصلية للورقة.

---

# 312. Network Topology Tests

اختبارات:

```text
client ↔ server
client ↔ relay ↔ server
multi-hop WAN
LAN
same rack
cross region
```

---

# 313. Cross-Region

RTT 100–250 ms يجب أن يبقى supported، لكن message deadline policies يجب أن تتكيف بدلاً من محاولة ضمان نفس behavior كـ1ms LAN.

---

# 314. High RTT Reliability

عند RTT مرتفع، retransmission قد تكون غالية.

لهذا:

```text
freshness
redundancy
selective retransmission
```

قد تكون أكثر فعالية من aggressive retry.

---

# 315. High Loss Policy

عند loss ~5–10%، يجب عدم الحفاظ على نفس realtime send rate بشكل أعمى.

CC يحدد network budget، بينما scheduler يحذف stale traffic.

---

# 316. Burst Loss Policy

يمكن استخدام redundancy مؤقتة أو إرسال latest state only، لكن تحت congestion budget.

---

# 317. Mobile Transition

عند تبدل الشبكة:

```text
old path
 ↓
new path challenge
 ↓
validate
 ↓
new active path
```

ثم يعاد ضبط بعض path-specific estimators حسب policy.

---

# 318. Wi-Fi Roaming

قد يحدث:

```text
same device
new AP
new NAT/path
```

يجب ألا يعني ذلك game reconnect فوراً إذا نجحت path validation.

---

# 319. NAT Timeout

لا توجد قيمة عالمية مضمونة لكل middleboxes؛ keepalive interval configurable.

لا تجعل protocol يرسل heartbeat كثيفاً افتراضياً فقط لأن بعض NATs قصيرة العمر.

---

# 320. Path Idle

في connection active لا حاجة لـ PING إضافي إذا كانت data traffic كافية للحفاظ على path/liveness.

---

# 321. Control Traffic Priority

Control frames يجب أن تكون high priority ولكن bounded، لأن attacker قد يحاول خلق control amplification.

---

# 322. Ping Suppression

إذا كان outgoing traffic موجوداً بالفعل، يمكن piggyback keepalive/liveness evidence بدلاً من packet إضافي.

---

# 323. Application Heartbeat

لا يخلط بين:

```text
transport liveness
application heartbeat
```

كلاهما قد يحتاج فترات مختلفة.

---

# 324. Session Ownership

Connection ID identifies transport connection، وليس ملكية اللعبة أو الحساب.

game layer يحتفظ بهوية اللاعب.

---

# 325. Authentication Identity

قد يرتبط handshake بهوية application，但 transport يجب أن يظل modularاً.

---

# 326. Network Owner / Game Server

في deployments الخاصة يمكن أن يكون server هو root of trust للاتصال.

لكن GTP نفسه لا يفرض membership model؛ هذا يقع في handshake/application authorization.

---

# 327. Authorization

بعد authentication يمكن application أن يحدد:

```text
allowed player
allowed game room
allowed shard
```

transport لا يحول ذلك إلى packet routing semantics.

---

# 328. Session Admission

يمكن للخادم ألا يقبل game connection إلا بعد:

```text
stateless validation
crypto establishment
application authorization
```

حسب deployment.

---

# 329. Abuse vs Congestion

لا يجب استخدام congestion controller كـ abuse limiter الوحيد.

DoS protection يحتاج:

```text
rate limit
admission control
connection quotas
crypto budgets
```

---

# 330. Protocol State Machine

حالات مقترحة:

```text
INITIAL
HANDSHAKING
VALIDATED
ESTABLISHED
DRAINING
CLOSED
```

Migration state يمكن أن يكون sub-state بدلاً من حالة عليا مستقلة.

---

# 331. Handshake Failure

عند فشل crypto/protocol negotiation يجب إنهاء handshake state فقط، وعدم ترك memory allocations معلقة.

---

# 332. Close Draining

بعد close قد يحتفظ endpoint بقدر صغير من state لمنع retry/stale packets من إعادة إنعاش session القديمة.

---

# 333. CID Retirement

يجب أن يكون هناك lifecycle واضح للـ CID:

```text
allocated
active
retiring
retired
```

---

# 334. CID Collision

يجب أن يكون احتمال collision غير عملي مع random opaque IDs، ويجب أن يكون هناك server-side defense.

---

# 335. Stateless Lookup

في server front-end يمكن أن يكون CID كافياً لتحديد shard/worker routing، لكن لا يجب أن يكشف mapping بصورة مباشرة للمهاجم.

---

# 336. Load Balancer

يمكن أن يستخدم load balancer نسخة من CID/cryptographic routing token دون فهم game payload.

---

# 337. Connection Migration Through LB

عند migration، load balancer يجب أن يوجه CID إلى نفس session owner، إلا إذا حدث explicit rebalance.

---

# 338. Failure Recovery

إذا مات worker، connection state قد تضيع في v1.

future design قد تستخدم session replication لكن ذلك ليس هدف low-latency core.

---

# 339. State Replication Cost

لا ينبغي نسخ hot connection state بين cores لمجرد high availability؛ التكلفة قد تكون أعلى من فائدتها.

---

# 340. Game-Level Recovery

عند server failure:

```text
new transport connection
→ game session resume
```

إذا احتاج application ذلك، وليس شرطاً أن يحتفظ GTP بالـ game state.

---

# 341. Protocol Layer Boundaries

الحدود الرسمية:

```text
Game API
↓
Message Semantics
↓
Transport Engine
↓
Recovery/CC/Pacing
↓
Security
↓
I/O
```

---

# 342. Security != IO

crypto لا يعرف هل backend يستخدم:

```text
UDP socket
io_uring
DPDK
```

---

# 343. Protocol Core != Runtime

core timing/state machine لا يعتمد على:

```text
Tokio task
Monoio task
io_uring CQE
```

---

# 344. Runtime Adapter Responsibilities

runtime adapter يدير:

- readiness.
- event polling.
- submission/completion.
- buffer lifetime.
- wakeups.

ولا يدير game semantics.

---

# 345. I/O Backend Responsibilities

backend يملك:

```text
socket creation
send/recv
batching
gso/gro
os options
```

وليس:

```text
cwnd
message reliability
ordered delivery
```

---

# 346. Protocol Engine Responsibilities

engine يملك:

```text
packet parsing
ACK
loss
RTT
CC
pacing
scheduling
message semantics
```

---

# 347. Game Engine Responsibilities

game layer يملك:

```text
world simulation
state generation
relevance
serialization
prediction
authority
```

---

# 348. Serialization Boundary

GTP لا يفرض protobuf/serde/custom serializer.

يستقبل byte payloads أو zero-copy application frames.

---

# 349. Delta Compression Boundary

game layer هو المسؤول عن تحديد:

```text
full snapshot
Delta
quantized state
compressed state
```

transport فقط ينقلها بالدلالة المناسبة.

---

# 350. Interest Management

game server قد يقرر أن client A لا يحتاج entity B.

GTP لا يجب أن يعرف هذه القاعدة، لكنه يجب أن يوفر priority/deadline hooks ليستفيد منها.

---

# 351. Entity State Keys

يمكن أن يكون:

```text
state_key = entity_id + state_type
```

مثلاً:

```text
entity 183 / transform
entity 183 / aim
```

حتى يمكن تحديث كل واحدة مستقلاً.

---

# 352. Snapshot Generation

كل game tick يمكن أن يولد:

```text
generation N
```

والـ transport يستعملها لإدارة supersession.

---

# 353. Cross-Tick State

لا يجوز أن يحتفظ transport تلقائياً بكل snapshot للأبد.

Game layer يحدد ما إذا كانت redundancy/recovery مطلوبة.

---

# 354. State Coalescing

إذا queued:

```text
position 100
position 101
position 102
```

يمكن coalesce إلى:

```text
position 102
```

إذا كان state key وsemantics تسمح.

هذه optimization مهمة جداً.

---

# 355. Event Coalescing

بعض الأحداث يمكن دمجها:

```text
multiple cosmetic updates
```

لكن events ذات semantics مستقلة لا يجوز دمجها تلقائياً.

---

# 356. Transport-side Coalescing Rules

يجب أن تكون explicit عبر message metadata:

```text
coalescible
supersedable
ordered
```

---

# 357. Packetization Policy

لا يجب أن يحمل packet خليطاً يجعل أي failure يعطل unrelated recovery state.

ومع ذلك يمكن batching multiple independent frames لتقليل overhead.

الحل هو:

```text
shared packet
independent frame state
```

---

# 358. Frame-level Recovery

كل reliable frame يجب أن يكون recoverable بشكل مستقل حتى لو شارك packet مع غيره.

---

# 359. ACK Frame Semantics

ACK يقر packet reception، وليس semantic message delivery بالضرورة.

قد يحتاج sender إلى message-level confirmation فقط في حالات خاصة، ولا يجب افتراض ذلك لكل message.

---

# 360. Delivery Confirmation

إذا كان application يحتاج proof أن event processed، فهذا application ACK منفصل عن transport ACK.

---

# 361. Application ACK

مثال:

```text
transaction_id
processed=true
```

لا ينبغي الخلط بينه وبين packet ACK.

---

# 362. Idempotency

reliable event handlers في game/application ينبغي أن تكون idempotent قدر الإمكان لأن transport recovery قد يواجه duplicates قبل suppression النهائي في حالات edge.

---

# 363. Duplicate Suppression Window

يجب أن توجد window كافية لاكتشاف duplicate packets القديمة ضمن limits memory.

---

# 364. Stale Duplicate

حتى لو duplicate اجتازت packet-level window، message semantics مثل sequence/generation يمكنها إسقاطها.

---

# 365. Security Replay

الـ replay protection يجب أن تكون مستقلة عن application duplicate handling.

---

# 366. Control Replay

PATH_RESPONSE وclose/control frames تحتاج validation مناسب حتى لا يمكن packet قديم تغيير state الحالي.

---

# 367. Close Replay

يجب ألا يؤدي close packet قديم إلى إغلاق connection جديدة بسبب CID reuse.

CID lifecycle يجب أن يمنع ذلك.

---

# 368. Version Negotiation Security

version negotiation يجب أن تكون مقاومة بقدر معقول لمحاولات downgrade/spoofing، مع binding إلى handshake transcript حيث يلزم.

---

# 369. Handshake Cryptography

التفاصيل النهائية للـ cryptographic handshake يجب أن تكون formalized لاحقاً كجزء من security specification مستقل.

---

# 370. Security Specification Separation

هذه الورقة تعرف boundary فقط؛ لا تعتبر بديلاً عن cryptographic protocol review.

---

# 371. Formal Verification Candidates

الأجزاء المرشحة:

```text
sequence comparison
ACK range parser
state machine
packet number transitions
reassembly
path validation state
```

يمكن لاحقاً استخدام model checking أو property-based testing.

---

# 372. Parser Fuzzing Targets

خصوصاً:

```text
varint
ACK ranges
frame lengths
fragment offsets
CID parsing
header flags
```

---

# 373. Memory Safety Targets

Rust يقلل أخطاء memory safety، لكن يجب اختبار:

```text
buffer lifetime
pool reuse
unsafe I/O
DMA/buffer ownership
```

---

# 374. Async Cancellation

يجب أن تكون cancelation semantics واضحة عند shutdown أو worker migration.

لا يجوز أن يبقى borrowed buffer بعد إلغاء operation.

---

# 375. Completion Ownership

في io_uring يجب أن يحدد كل operation owner للbuffer حتى completion.

---

# 376. Fixed Buffers

يمكن استخدام registered/provided buffers عندما تقدم benefit مثبتة، لكن يجب ألا تعقد memory management بلا داعٍ.

---

# 377. Buffer Pools

يجب أن تكون pools:

```text
bounded
reusable
per-worker when possible
NUMA-aware when useful
```

---

# 378. Pool Exhaustion

عند exhaustion:

```text
drop low-value incoming state
apply backpressure
preserve control
```

ولا يجب panic.

---

# 379. Backpressure Signaling

Game API يمكن أن تتلقى:

```text
QueueFull
Expired
ResourceLimited
```

بدلاً من silently accepting message لا يمكن تنفيذها.

---

# 380. Reliability and Queue Limits

إذا امتلأت reliable queue، لا ينبغي أن تسقط reliability silently.

إما:

```text
reject send
block asynchronously by policy
or fail connection/application operation
```

حسب API.

---

# 381. Realtime Queue Limits

realtime يمكن إسقاط stale entries، ولذلك يملك degradation graceful أفضل من reliable queues.

---

# 382. Bulk Queue Limits

bulk يجب أن يكون bounded بشدة، لأنه أقل أولوية.

---

# 383. Memory DoS Through Fragmentation

attacker يمكن أن يبدأ fragmented message ثم يرسل أجزاء قليلة فقط.

الحماية:

```text
reassembly timeout
per-peer fragment cap
bytes cap
```

---

# 384. Memory DoS Through Ordering Gaps

ordered reliable messages يمكن أن تسبب huge pending gaps.

الحماية:

```text
max gap size
max buffered ordered bytes
```

---

# 385. ACK CPU DoS

peer قد يرسل ACKs كثيرة أو ranges معقدة.

الحماية:

```text
parse budget
range cap
rate limit
```

---

# 386. Crypto CPU DoS

الـ server يجب أن يطبق admission قبل crypto expensive processing عندما يمكن ذلك.

---

# 387. Scheduler CPU DoS

لا تسمح peer message metadata بخلق ملايين queue entries بلا حدود.

---

# 388. Message Metadata Limits

كل peer له limits على:

```text
pending messages
keys
ordered groups
state keys
```

---

# 389. Control Plane Abuse

control frames rate-limited per connection and source.

---

# 390. Endpoint Isolation

endpoint-level resource exhaustion يجب ألا يقتل جميع connections بسبب peer واحد.

---

# 391. Failure Domains

يفضل أن يكون:

```text
worker crash
```

معزولاً عن workers أخرى إن أمكن، خصوصاً في multi-process deployment.

---

# 392. Process vs Thread

GTP يعمل جيداً داخل process متعدد threads، لكن يمكن تشغيل workers في processes منفصلة إذا كان isolation مطلوباً.

---

# 393. Shared Memory

لا تحتاج v1 إلى shared-memory data plane بين processes.

يمكن استخدام socket/IPC control plane عند الحاجة.

---

# 394. NIC Queue to Worker Mapping

يجب توثيق deployment guidance لضمان affinity ثابت.

---

# 395. Horizontal Scaling

server fleet:

```text
edge
 ↓
CID routing
 ↓
worker
 ↓
game process
```

أو:

```text
load balancer
 ↓
server shard
```

---

# 396. Geographic Routing

game matchmaking يقرر region؛ GTP لا يقرر region.

---

# 397. Session Migration Across Servers

ليس mandatory v1.

يمكن لاحقاً اعتماد application session handoff.

---

# 398. Benchmark Target Matrix Summary

| البعد | القيم الأساسية |
|---|---|
| RTT | 5/20/50/100/200 ms |
| Loss | 0/0.1/1/2/5/10% |
| Reorder | 0/1/5/10% |
| Packet size | 64/128/256/512/768/1200/1400 B |
| Bandwidth | 10M/50M/100M/1G/10G/100G lab |
| Players | 16/64/128/256/300 |
| Tick | 30/60/120 Hz |
| ACK | fixed + adaptive |
| Runtime | manual/Tokio/Monoio/io_uring |
| I/O | UDP/GSO/GRO/DPDK experimental |

---

# 399. Baseline Acceptance Criteria

لا تعتبر implementation candidate مرشحاً لـ v1 إذا فشل في:

```text
correctness
loss recovery
fairness
resource bounds
security baseline
```

حتى لو كان أسرع في microbenchmark.

---

# 400. Performance Acceptance Criteria

بعد prototype، يجب تحديد targets كمية لـ:

```text
P99 latency
CPU/connection
cycles/packet
allocations/packet
memory/connection
packets/sec/core
stale delivery ratio
```

الأرقام في النسخة الحالية design targets وليست نتائج مثبتة.

---

# 401. Initial Suggested Targets

للتخطيط فقط:

```text
0 heap allocations / packet in steady state
0–1 copies common path
< 16 KB hot+cold average target is NOT mandatory; optimize based on actual state
P99 local transport processing in low-single-digit microseconds as a stretch target
```

يجب عدم تحويلها إلى promises تسويقية.

---

# 402. Important Benchmark Rule

قارن latency end-to-end وليس فقط function-level:

```text
application
→ scheduler
→ GTP
→ kernel
→ network emulation
→ peer
```

---

# 403. Instrumentation Points

ضع timestamps في:

```text
app enqueue
scheduler select
packet build
crypto done
syscall submit
NIC send if available
peer receive
message dispatch
```

حتى نعرف أين تضيع microseconds.

---

# 404. Latency Budget

يمكن في server LAN وضع budget تقريبي للتحليل:

```text
application enqueue
+ scheduling
+ codec
+ crypto
+ I/O
+ wire
```

لكن القيم الفعلية تعتمد على hardware.

---

# 405. P99.9 Requirement

يجب أن يكون جزءاً من production acceptance لأن الألعاب التنافسية تتأثر بالtail latency حتى لو كان P50 ممتازاً.

---

# 406. Soak Memory Criterion

بعد ساعات من traffic المختلط:

```text
memory trend must stabilize
```

ولا ينبغي وجود monotonically growing queues/pools.

---

# 407. Connection Churn Criterion

اختبر آلاف/مئات آلاف connection create-close cycles حسب deployment.

يجب ألا تسبب:

```text
fragmentation
FD leaks
timer leaks
CID table leaks
```

---

# 408. High-Concurrency Criterion

اختبار:

```text
many mostly-idle connections
few high-rate connections
mixed workload
```

للتأكد من أن scheduler/timers لا تكلف كثيراً للـ idle peers.

---

# 409. Timer Scalability

يجب أن تكون تكلفة timers تقريبية sublinear أو bounded per active deadline bucket بدلاً من object لكل connection لكل event.

---

# 410. Idle Connection Cost

يجب أن تكون connection idle رخيصة جداً CPU-wise.

---

# 411. Active Connection Cost

الكلفة يجب أن تتدرج أساساً مع:

```text
packets/sec
queued work
retransmission activity
```

لا مع مجرد وجود connection.

---

# 412. Error Path Cost

invalid packet يجب أن يكون cheap reject ولا يصل إلى game layer.

---

# 413. Packet Capture

يجب توفير internal packet capture format لغرض debugging، مع القدرة على redaction/disable في production.

---

# 414. Wire Decoder Tool

project يجب أن يتضمن CLI مثل:

```text
gtp-dissect packet.pcap
gtp-trace session-id
gtp-stats capture.pcap
```

في المستقبل.

---

# 415. Wireshark Integration

يفضل تطوير dissector مخصص لـ GTP لتمكين:

- packet inspection.
- ACK visualization.
- loss visualization.
- frame decoding.

---

# 416. Deterministic Simulation

يجب بناء simulator مستقل يستطيع تشغيل:

```text
packet loss
reorder
delay
ACK behavior
CC
scheduler
```

بدون kernel/network.

---

# 417. Simulation Benefits

يسمح بعمل آلاف/ملايين السيناريوهات أسرع من network tests الحقيقية.

---

# 418. Property-Based Network Simulation

يمكن توليد:

```text
random loss
bursts
ACK delay
reordering
path changes
```

وفحص invariants.

---

# 419. Fuzz + Model Combination

أفضل coverage تأتي من:

```text
byte fuzzing
+
stateful simulation
```

---

# 420. Protocol Documentation

المواصفة النهائية يجب أن تحتوي على:

```text
normative protocol
implementation notes
security considerations
IANA-like registry if public
performance profile
```

---

# 421. Normative Language

استخدم:

```text
MUST
MUST NOT
SHOULD
SHOULD NOT
MAY
```

بحسب الاستخدام المعياري لهذه الكلمات.

---

# 422. Experimental Language

أي algorithm لم يثبت يجب أن يسمى:

```text
EXPERIMENTAL
```

ولا يوصف كأفضل خوارزمية نهائية.

---

# 423. Reference Profile

GTP/1 reference profile يجب أن يكون محدداً بالكامل:

```text
AEAD on
CUBIC-compatible CC
adaptive ACK enabled
pacing on
PMTU probing on
GSO/GRO opportunistic
stale-drop enabled
```

---

# 424. Minimal Profile

يمكن توفير profile للـ embedded/small systems:

```text
simple UDP
fixed ACK policy
no migration
minimal telemetry
```

لكن Internet profile يبقى أكثر صرامة.

---

# 425. High-Performance Profile

```text
thread-per-core
GSO/GRO
io_uring/Monoio
per-worker pools
CPU affinity
batching
```

---

# 426. Kernel-Bypass Profile

Experimental:

```text
DPDK
AF_XDP
user-space NIC processing
```

ولا يكون v1 baseline.

---

# 427. Why Rust

Rust مناسبة لأن GTP يحتاج توازناً بين:

```text
memory safety
zero-copy opportunities
predictable ownership
low overhead abstractions
FFI
systems programming
```

لكن Rust وحدها لا تضمن performance؛ architecture هي العامل الأساسي.

---

# 428. Rust Performance Principle

لا تستخدم abstraction إذا لم تستطع compiler والـ generated code جعله قريباً من cost المباشر.

لكن لا تفترض أن abstraction سيء قبل profiling.

---

# 429. Generic Programming

يمكن استخدام generics/traits في boundaries مثل CC وI/O.

يجب أن تكون hot path قابلة لـ monomorphization أو dispatch منخفض الكلفة عندما يكون ذلك مناسباً.

---

# 430. Dynamic Dispatch Policy

لا مانع من `dyn Trait` في control plane، لكن لا يجب وضع virtual dispatch في packet-per-packet inner loop دون قياس.

---

# 431. Compile-Time Backend Selection

يمكن أن يكون:

```text
feature = "tokio"
feature = "monoio"
feature = "io-uring"
feature = "dpdk"
```

بحسب build.

لكن wire/protocol semantics تبقى نفسها.

---

# 432. Runtime Backend Selection

ممكن أيضاً إذا كان code size/deployment يسمح، خصوصاً في endpoint abstraction.

---

# 433. Feature Flags

لا ينبغي أن تتسبب feature flags في combinatorial explosion غير قابل للاختبار.

يجب الحفاظ على profiles رسمية محدودة.

---

# 434. Dependency-Free Core

الـ packet/state machine core يجب أن يكون قليل dependencies جداً.

---

# 435. Testing Dependency Isolation

يمكن تشغيل protocol simulator دون OS أو socket.

وهذا يساعد fuzzing وmodel tests.

---

# 436. Public API Documentation

كل public function يجب أن توضح:

```text
latency/copy semantics
ownership
threading contract
errors
```

---

# 437. Threading Contract

يجب أن تعرف كل object:

```text
Send + Sync?
owning worker?
borrow-only?
```

ولا تعتمد على assumptions غير موثقة.

---

# 438. Connection Handle

يفضل أن يكون handle خفيفاً:

```text
ConnectionHandle
```

يشير إلى worker-owned connection، بدلاً من نقل connection object كاملاً بين threads.

---

# 439. Cross-Thread Send

إذا احتاجت game threads متعددة إرسال messages إلى connection worker:

```text
bounded MPSC
```

ويجب batch requests.

---

# 440. Game Thread Integration

لا يجب أن ينتظر game thread network syscall.

الـ send API enqueue/nonblocking ضمن التصميم المعتاد.

---

# 441. Receive Integration

game simulation يحصل على network messages عبر queue/channel مناسبة، مع إمكانية polling مباشر في engines منخفضة latency.

---

# 442. Shared Memory In-Process

في game server single-process، يمكن أن تكون الرسائل بين game systems وGTP views zero-copy حيث lifetime يسمح.

---

# 443. Async Message Ownership

message قد تتعايش مع packet buffer حتى نهاية callback، وبعدها تنتقل ownership فقط إذا احتاج التطبيق.

---

# 444. Backpressure to Game

عندما يصبح network queue saturated، يجب أن يستطيع transport إبلاغ game system ليخفض update rate أو coalesces state.

---

# 445. Adaptive Send Frequency

game layer يمكن أن تغير:

```text
120Hz → 60Hz → 30Hz
```

عندما تكون state rate أعلى من network budget.

GTP يوفر telemetry اللازمة لاتخاذ القرار، ولا يفرض معدل tick.

---

# 446. Adaptive Snapshot Rate

إذا كان:

```text
loss high
RTT high
queue high
```

يمكن للgame layer خفض snapshot rate أو payload detail.

هذا cooperation بين transport وgame layer.

---

# 447. Adaptive Quality

مثلاً:

```text
normal:
full state

congested:
less frequent
smaller deltas

critical:
input + important events
```

---

# 448. Transport Feedback API

يمكن توفير:

```rust
NetworkFeedback {
    rtt,
    loss,
    cwnd,
    pacing_rate,
    queue_pressure,
    freshness_pressure,
}
```

لـ game adaptation.

---

# 449. Feedback Rate

لا ترسل telemetry إلى game logic لكل packet.

تقدم snapshot/updates كل tick أو على interval صغير.

---

# 450. Control/Data Separation

في codebase:

```text
data plane
control plane
```

بشكل واضح.

---

# 451. Data Plane Objective

أعلى packet efficiency وأقل latency.

---

# 452. Control Plane Objective

Configuration, handshake, migration, extension negotiation, lifecycle.

يمكن أن يكون أقل حساسية للـ microseconds.

---

# 453. Control Plane Scheduling

لكن control frames لا يجب أن تكون starvation victims عند congestion شديد؛ لها reserved budget صغير.

---

# 454. Reserved Control Budget

مثلاً conceptual:

```text
reserved_control_budget
```

بحيث يستطيع ACK/path/close العمل تحت queue pressure.

القيمة تحدد بالbenchmark.

---

# 455. Reliable Data Budget

باقي budget يشارك بين reliable/realtime وفق scheduler.

---

# 456. Realtime Reservation

في competitive profile يمكن حجز جزء من budget لـ fresh state/input، مع عدم تجاوز cwnd/pacing.

---

# 457. Fairness Within Connection

لا تسمح reliable queue كبيرة بابتلاع كل connection bandwidth.

---

# 458. Class Weights

يمكن تمثيل:

```text
control = reserved
input = high
realtime = high
reliable = medium
bulk = low
```

مع deadline overrides.

---

# 459. Utility Scheduling

النظام النهائي يمكن أن يستخدم:

```text
hard constraints
+
score-based ranking
+
weighted fairness
```

بدلاً من pure priority.

---

# 460. Scheduler Score Inputs

```text
priority
remaining lifetime
expected delivery time
size
retransmission value
supersession risk
class weight
```

---

# 461. Scheduler Explainability

للتشخيص، يجب أن يسجل سبب الاختيار/الإسقاط في debug mode:

```text
expired
priority
budget
superseded
queue pressure
```

---

# 462. Transport Drop Reasons

على الأقل:

```text
expired
superseded
queue_limit
memory_limit
cwnd_limit
path_invalid
protocol_reject
```

---

# 463. Drop Metrics

هذه metrics مهمة جداً لتحديد ما إذا كانت المشكلة:

```text
network loss
application overproduction
scheduler pressure
```

---

# 464. Overproduction Detection

إذا game layer تنتج:

```text
500 KB/tick
```

بينما network budget يسمح:

```text
100 KB/tick
```

GTP يجب أن يظهر ذلك بوضوح في telemetry.

---

# 465. Application/Transport Contract

الهدف أن يعرف الفريق:

```text
network isn't slow;
application is overproducing stale state
```

عندما تكون هذه هي المشكلة.

---

# 466. End-to-End Queue Analysis

يجب تتبع:

```text
application queue
GTP queue
kernel socket queue
network bottleneck queue
peer receive queue
```

حتى لا يتم تحسين طبقة خاطئة.

---

# 467. Socket Queue Visibility

Linux backend يمكنه جمع بعض kernel socket metrics إن كان deployment يحتاج ذلك، لكن ليس على كل packet.

---

# 468. Network Emulator Correlation

الـ benchmark harness يجب أن يسجل actual configured delay/loss مقابل observed RTT/loss.

---

# 469. Tail Latency Attribution

كل test run يجب أن يعطي breakdown إن أمكن:

```text
transport CPU
kernel I/O
network emulator
peer processing
```

---

# 470. Reference Target Hardware

reference benchmark server يفضل توثيق:

```text
modern x86-64 multi-core CPU
10/25GbE NIC
Linux recent kernel
```

لكن النتائج لا تعمم بدون benchmark على target hardware.

---

# 471. ARM Consideration

Rust protocol core يجب أن يبقى portable إلى ARM64 عندما لا تعتمد backend على x86-specific features.

---

# 472. Hardware Crypto Variability

crypto backend يجب أن يختار implementation مناسباً للـ CPU.

---

# 473. Kernel Version Feature Detection

GSO/GRO/io_uring advanced operations يجب feature-detect أو fallback، ولا تفترض أن كل Linux environment يملك نفس capabilities.

مثلاً، توثيق Rust `io-uring` الحالي يبين multishot `recvmsg` على kernels مناسبة، وبعض bundle receive support مرتبط بـ kernel حديث؛ لذلك implementation يجب أن يتحقق من الدعم runtime بدلاً من افتراضه.

---

# 474. GSO Limits

Linux UDP GSO موثق بحدود segmentation وعدد datagrams في call، ويجب على backend احترام قيود kernel/NIC مع إبقاء كل segmented datagram صالحاً بالنسبة إلى MTU.

---

# 475. GRO Semantics

GRO لا يعني أن wire packet أصبح أكبر.

هو فقط تجميع receive-side buffers، ويجب إعادة تفسير segment boundaries قبل GTP packet parsing.

---

# 476. GSO/GRO and Timing

لا يجب أن تجعل batching يخفي timing الفردي إذا كان congestion controller يحتاج per-packet send timestamps.

يجب الاحتفاظ بمعلومات كل logical packet داخل batch.

---

# 477. Batching and RTT

ACK/loss logic يستمر packet-oriented حتى إذا كانت I/O operation batch-oriented.

هذه نقطة معمارية مهمة.

---

# 478. Batching and Pacing

batch creation يجب أن يكون محدوداً بالـ pacing budget، وليس "كل شيء جاهز الآن".

---

# 479. Batching and Deadlines

لا يؤخر packet ذو deadline قريب فقط كي يكتمل batch كبير.

deadline sensitivity يجب أن تتغلب على batching عندما تكون الكلفة الزمنية أعلى من مكسب syscall.

---

# 480. Adaptive Batch Size

قد يختار backend:

```text
small batch at low traffic
larger batch at high packet rate
```

بحسب measured cost.

---

# 481. Polling Strategy

endpoint loop يمكن أن يكون:

```text
event-driven
busy-poll
hybrid
```

حسب profile.

---

# 482. Hybrid Loop

مثلاً:

```text
spin briefly
 ↓
poll CQ/socket
 ↓
sleep only when idle
```

لكن يجب قياس power/CPU trade-off.

---

# 483. Power vs Latency

mobile/client profile قد يفضل power efficiency، server profile يفضل latency.

نفس protocol core يجب أن يدعم الاثنين.

---

# 484. Client vs Server

GTP يمكن أن يكون asymmetric في implementation:

```text
server:
thread-per-core
GSO/GRO
high concurrency

client:
lightweight event loop
less CPU
power-aware
```

wire semantics واحدة.

---

# 485. Client Network Conditions

client قد يكون خلف:

```text
NAT
Wi-Fi
mobile
VPN
```

لذلك conservative path startup مهم.

---

# 486. Server Network Conditions

server قد يملك:

```text
10/25/100GbE
low RTT intra-region
large fan-out
```

وهنا batching becomes critical.

---

# 487. Datacenter Path

Homa-inspired scheduling useful أكثر في low-RTT data center-like scenarios، لكن Internet deployment يحتاج loss/path/NAT handling مختلفاً.

---

# 488. Internet Safety

GTP يجب أن يبقى fair ومتحكم بالازدحام حتى عندما لا يستخدم transport standards العامة مثل QUIC.

RFC 8085 يؤكد أن UDP applications عبر Internet تحتاج congestion control أو rate adaptation مناسبة وأن traffic aggregate يجب أن يكون controlled.

---

# 489. Why Custom UDP Remains Justified

فقط إذا كانت:

```text
game semantics
freshness
deadline
low HoL
```

تنتج فوائد لا يمكن الحصول عليها بكلفة مناسبة من generic QUIC.

---

# 490. Why QUIC Remains a Baseline

QUIC يظل baseline مهم لأنه يوفر Internet-hardened transport concepts.

GTP يجب أن يتفوق في game-specific efficiency، لا أن يعيد اختراع Internet transport correctness دون داعٍ.

---

# 491. Why KCP Remains a Baseline

KCP يوفر comparison مفيد في:

```text
ARQ
low overhead
CPU
loss recovery
pacing
CC experimentation
```

KCP v2.1.1 تحديداً أضاف telemetry/pacing improvements تستحق المقارنة.

---

# 492. Why Homa Remains a Reference

Homa مفيد لفهم:

```text
message scheduling
receiver-driven service
latency vs throughput tradeoffs
```

لكن لا يترجم مباشرة إلى Internet protocol بسبب اختلاف environment.

---

# 493. Protocol Positioning

GTP يقع بين:

```text
raw custom UDP
```

و:

```text
full generic QUIC
```

ويهدف إلى أخذ correctness patterns من الثاني وspecialization efficiency من الأول.

---

# 494. Final Architecture

```text
                             GAME
                               |
                        Game Transport API
                               |
                  +------------+------------+
                  |                         |
             Message Semantics        Control API
                  |
       +----------+-----------+
       |          |           |
   Realtime   Reliable    Ordered Groups
       |          |           |
       +----------+-----------+
                  |
        Freshness / Deadline
                  |
        Coalescing / Supersession
                  |
             Scheduler
                  |
         Send Budget / Pacing
                  |
        +---------+---------+
        |                   |
   Congestion          ACK/RTT/Loss
     Control                 |
        |                    |
        +---------+----------+
                  |
             Path Manager
                  |
             Security/AEAD
                  |
              Wire Codec
                  |
              I/O API
         +--------+---------+
         |        |         |
        UDP     io_uring  Kernel-bypass
         |        |
      GSO/GRO  multishot
         |        |
         +--------+---------+
                  |
                 NIC
```

---

# 495. Final Rust Workspace

```text
gtp/
├── gtp-wire
├── gtp-types
├── gtp-core
├── gtp-recovery
├── gtp-cc
├── gtp-scheduler
├── gtp-path
├── gtp-crypto
├── gtp-memory
├── gtp-io
├── gtp-io-linux
├── gtp-io-uring
├── gtp-runtime-tokio
├── gtp-runtime-monoio
├── gtp-dpdk
├── gtp-bench
├── gtp-sim
├── gtp-fuzz
├── gtp-cli
└── wireshark-gtp
```

يمكن أن يبدأ prototype بعدد crates أقل، لكن هذا يمثل الفصل المعماري المستهدف.

---

# 496. Recommended Rust Stack

| الطبقة | الاختيار المفضل | البديل/الملاحظة |
|---|---|---|
| Wire views | zerocopy + custom codec | manual parsing |
| buffers | bytes / custom pools | benchmark-driven |
| sockets | socket2 | std where enough |
| Linux fast I/O | io_uring | raw syscalls only where justified |
| async integration | Tokio adapter | not core dependency |
| high-perf runtime | Monoio candidate | native loop |
| crypto | rustls ecosystem primitives / audited backend | implementation-specific |
| benchmarks | Criterion | perf/flamegraph for system profiling |
| fuzzing | cargo fuzz / libFuzzer ecosystem | property-based tests |

---

# 497. Current Ecosystem Verification

في وقت إعداد هذه الوثيقة، `zerocopy` و`s2n-quic` و`socket2` و`rustls` و`criterion` لديها إصدارات/توثيق حديثة في 2026، ويدعم `s2n-quic` خصائص مثل CUBIC وpacing وGSO وPMTU وconnection IDs؛ كما أن Rust `io-uring` الحالي يوفر multishot receive APIs. هذه الأدوات لا تعني أنها يجب أن تكون dependencies إجبارية؛ هي reference candidates يجب أن تخضع للـ benchmark والتدقيق.

---

# 498. Current QUIC Update Policy

QUIC RFC 9002 remains a foundational reference for recovery and congestion behavior. QUIC DATAGRAM RFC 9221 provides the model for unreliable congestion-controlled datagrams. QUIC v2 (RFC 9369) provides a lesson in version agility/ossification resistance.

ACK Frequency was still an IETF Internet-Draft in the retrieved 2026 material, so GTP should treat its concepts as evolving design input rather than claim standard finalization.

---

# 499. Current UDP/Linux Update Policy

Linux UDP documentation currently describes `UDP_SEGMENT`/GSO and `UDP_GRO` as available kernel features that reduce send/receive cost by batching datagrams. These belong in the Linux backend, not in the GTP wire protocol.

io_uring documentation also exposes multishot receive and buffer-group mechanisms appropriate for a high-rate backend, subject to kernel capability detection.

---

# 500. Version 1.1 Final Design Decision

GTP/1.1 يعتمد المعمارية التالية كخط أساس:

```text
UDP-based
+
Game-aware message semantics
+
QUIC-derived packet/ACK/RTT/path concepts
+
KCP-derived selective reliability and tunability lessons
+
Delivery-rate telemetry
+
CUBIC-compatible baseline CC
+
Experimental BBR/GTP-CC
+
Mandatory pacing architecture
+
Adaptive ACK strategy
+
Deadline + freshness scheduling
+
Message/frame-level retransmission
+
Generation-aware state coalescing
+
Linux GSO/GRO
+
Batch I/O
+
io_uring/Monoio-ready architecture
+
Rust ownership-driven hot path
+
AEAD secure Internet profile
+
NAT rebinding/path validation
+
Anti-amplification
+
Extensive simulation/fuzz/benchmarking
```

---

# 501. ما الذي لا يدخل v1.1 كـ mandatory feature؟

```text
DPDK
AF_XDP
FEC
Multipath
0-RTT
compression
advanced hardware offload
custom BBR
transparent server failover
```

هذه كلها يمكن إضافتها بعد إثبات الحاجة.

---

# 502. ترتيب التنفيذ العملي

## Phase A — Specification

```text
wire format
state machines
message semantics
error codes
security boundary
```

## Phase B — Reference Core

```text
UDP
CID
packet number
frames
ACK
RTT
loss
```

## Phase C — Reliability

```text
reliable unordered
reliable ordered groups
retransmission
expiry
```

## Phase D — CC/Pacing

```text
CUBIC baseline
pacing
ECN
adaptive ACK
```

## Phase E — Game Scheduler

```text
deadline
freshness
coalescing
supersession
priority
```

## Phase F — Internet Hardening

```text
AEAD
handshake
anti-amplification
NAT rebinding
path validation
```

## Phase G — Performance

```text
batching
GSO/GRO
memory pools
thread-per-core
io_uring
Monoio
```

## Phase H — Extreme Performance

```text
AF_XDP
DPDK
hardware experiments
```

---

# 503. Gate قبل الانتقال من Phase إلى Phase

لا ينتقل المشروع إلى المرحلة التالية لأن الكود "يعمل" فقط.

يجب أن ينجح في:

```text
correctness
stress
fuzz
performance regression
resource limits
```

---

# 504. Gate قبل اعتماد CC جديد

أي CC جديد يجب أن يقارن مع baseline من حيث:

```text
fairness
loss responsiveness
RTT inflation
throughput
P99 gameplay latency
```

---

# 505. Gate قبل اعتماد GSO/GRO

يجب إثبات أنه يحسن:

```text
CPU/packet
syscalls
packets/sec/core
```

دون الإضرار بـ:

```text
pacing
timing
packet accounting
```

---

# 506. Gate قبل اعتماد io_uring

يجب إثبات end-to-end improvement، لا فقط benchmark syscall.

---

# 507. Gate قبل DPDK

لا يعتمد إلا بعد profile يظهر أن kernel/UDP path bottleneck حقيقي.

---

# 508. Design Risks

أكبر المخاطر:

1. **تعقيد scheduler.** إذا أصبح scheduler معقداً جداً، قد يفقد GTP ميزة البساطة.
2. **Congestion algorithm غير ناضج.** throughput العالي لا يعني fairness أو game quality.
3. **Security/handshake scope creep.** قد يتحول transport إلى QUIC جديد بالكامل.
4. **Kernel optimization premature.** قد نحل bottleneck غير موجود.
5. **Excessive metadata.** deadlines/priorities/generations يجب ألا تجعل كل packet ثقيلًا.
6. **Cross-thread sharing.** قد يقتل cache locality.
7. **Feature explosion.** كل optional feature يزيد correctness/test burden.

---

# 509. أكبر مخاطرة تصميمية

أكبر خطأ هو محاولة جعل GTP:

```text
QUIC + KCP + Homa + BBR + FEC + DPDK + 0-RTT
```

في بروتوكول واحد من الإصدار الأول.

المنتج الصحيح يجب أن يكون أصغر:

```text
semantics
recovery
CC
pacing
path
security
fast I/O
```

ثم تزيد features بناءً على benchmarks.

---

# 510. أهم ابتكار يجب الحفاظ عليه

الابتكار ليس header صغيراً.

وليس مجرد reliable UDP.

بل:

> **نقل يعرف القيمة الزمنية للمعلومة.**

أي أن transport يستطيع التمييز بين:

```text
must arrive
may arrive
latest only
ordered
expires soon
already obsolete
```

---

# 511. تعريف النجاح الحقيقي

GTP ناجح عندما يستطيع في ظروف congestion/loss أن يحافظ على:

```text
fresh input delivery
fresh state delivery
fast critical event recovery
bounded tail latency
fair congestion behavior
stable CPU cost
```

حتى لو لم يحقق أعلى raw throughput مقارنة بــ bulk-optimized transports.

---

# 512. الخلاصة النهائية

التصميم المقترح لـ GTP/1.1 ليس clone من KCP أو QUIC.

هو transport متخصص للألعاب مبني فوق UDP ويجمع بين:

```text
QUIC-style Internet correctness patterns
+
KCP-style controllable selective reliability
+
delivery-rate measurement
+
Homa-inspired message scheduling ideas
+
Game-specific freshness/deadlines
+
shared congestion/pacing
+
Rust-native data-oriented implementation
+
Linux batched UDP fast paths
```

التركيز الأساسي ليس على إضافة أكبر عدد من features، وإنما على بناء pipeline قصير ومتوقع:

```text
receive batch
→ cheap validate
→ CID lookup
→ authenticate
→ ACK/RTT/loss
→ dispatch
```

و:

```text
game enqueue
→ stale/coalesce
→ deadline scheduler
→ congestion budget
→ pacing
→ packet build
→ crypto
→ batch send
```

وهذه هي البنية التي يجب أن تكون نقطة الانطلاق الرسمية للـ implementation.

---

# 513. المراجع التقنية الخارجية الأساسية

1. IETF RFC 9000 — QUIC: A UDP-Based Multiplexed and Secure Transport.
2. IETF RFC 9001 — Using TLS to Secure QUIC.
3. IETF RFC 9002 — QUIC Loss Detection and Congestion Control.
4. IETF RFC 9221 — An Unreliable Datagram Extension to QUIC.
5. IETF RFC 9369 — QUIC Version 2.
6. IETF draft-ietf-quic-ack-frequency — QUIC Acknowledgment Frequency (2026 working draft; not treated as final RFC here).
7. IETF RFC 8085 — UDP Usage Guidelines.
8. Linux `udp(7)` — UDP_SEGMENT / UDP_GRO.
9. skywind3000/kcp — KCP releases, including v2.0 and v2.1.1 (2026).
10. s2n-quic — Rust QUIC implementation and performance-oriented networking features.
11. io-uring Rust crate — multishot receive / buffer group APIs.
12. Monoio — Rust thread-per-core runtime.
13. zerocopy — Rust zero-cost memory conversion toolkit.
14. socket2 — low-level cross-platform socket configuration.
15. rustls — modern Rust TLS library.
16. Criterion — statistics-driven Rust benchmarking.

---

# 514. المصادر التي تم التحقق منها أثناء إعداد هذه النسخة

- KCP v2.1.1 release notes: إضافة `acked_bytes`, `xmit`, actual-send callback location، pacing اختياري وتحسينات ssthresh/cwnd.
- QUIC loss/recovery RFC 9002.
- QUIC DATAGRAM RFC 9221.
- QUIC v2 RFC 9369.
- QUIC ACK Frequency Internet-Draft 2026.
- RFC 8085 UDP Usage Guidelines.
- Linux `udp(7)` عن UDP_SEGMENT وUDP_GRO.
- Rust `io-uring` multishot receive APIs.
- Monoio thread-per-core design.
- zerocopy current documentation.
- s2n-quic current feature set.
- socket2 current low-level socket APIs.
- rustls current documentation/version line.
- Criterion current benchmark line.

---

# 515. Final Recommendation

**لا يبدأ التنفيذ بكتابة packet header النهائي.**

يجب أولاً تثبيت المواصفات التالية كوثائق فرعية مستقلة:

```text
GTP-ARCH-01   Architecture
GTP-WIRE-01   Wire Format
GTP-REC-01    ACK/Loss/Recovery
GTP-CC-01     Congestion Control
GTP-SCHED-01  Deadline/Freshness Scheduler
GTP-PATH-01   Path/NAT/Migration
GTP-SEC-01    Security/Handshake
GTP-RUST-01   Rust Implementation Architecture
GTP-LINUX-01  Linux Fast I/O Backend
GTP-TEST-01   Verification & Benchmark Plan
```

بعد تثبيتها يمكن استخراج implementation tasks وRust traits والـ packet diagrams والـ state machines بصورة دقيقة.

---

# 516. القرار الهندسي النهائي

**GTP/1.1 = Game-aware UDP Transport، Rust-native، Internet-safe، performance-first، وليس generic QUIC clone.**

القواعد الأساسية:

```text
Reliable when necessary.
Unreliable when useful.
Sequenced when freshness matters.
Ordered only when semantics require it.
Expired data is disposable.
Priority never bypasses congestion control.
Retransmit logical messages, not stale packets.
One connection owns one congestion state.
One connection should have one primary worker owner.
Batch I/O wherever useful.
Use GSO/GRO/io_uring when the measured workload benefits.
Keep protocol core independent from Linux/runtime/DPDK.
Secure the Internet profile by default.
Optimize only after profiling.
```

**هذه النسخة هي baseline المقترح قبل الانتقال إلى تصميم الـ wire protocol والتنفيذ الفعلي بلغة Rust.**

## Appendix A — روابط المراجع

- QUIC RFC 9002: https://www.rfc-editor.org/rfc/rfc9002.html
- QUIC DATAGRAM RFC 9221: https://www.rfc-editor.org/rfc/rfc9221.html
- QUIC Version 2 RFC 9369: https://www.rfc-editor.org/rfc/rfc9369.html
- UDP Usage Guidelines RFC 8085: https://www.rfc-editor.org/rfc/rfc8085.html
- QUIC ACK Frequency draft: https://datatracker.ietf.org/doc/draft-ietf-quic-ack-frequency/
- Linux UDP man page: https://man7.org/linux/man-pages/man7/udp.7.html
- KCP releases: https://github.com/skywind3000/kcp/releases
- s2n-quic: https://docs.rs/s2n-quic/latest/s2n_quic/
- io-uring Rust: https://docs.rs/io-uring/latest/io_uring/
- Monoio: https://docs.rs/monoio/latest/monoio/
- zerocopy: https://docs.rs/zerocopy/latest/zerocopy/
- socket2: https://docs.rs/socket2/latest/socket2/
- rustls: https://docs.rs/rustls/latest/rustls/
- Criterion: https://docs.rs/criterion/latest/criterion/
