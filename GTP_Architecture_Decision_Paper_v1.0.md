# GTP/1 — Architecture Decision & Implementation Boundary Paper v1.0
## ورقة تقنية لتحديد ما يُبنى داخل البروتوكول، وما يُعاد استخدامه، وما يُؤخذ كمرجع تصميمي فقط

**الحالة:** Technical Architecture / Architecture Decision Record
**الإصدار:** 1.0
**التاريخ:** 28 أغسطس 2026
**المشروع:** Game Transport Protocol (GTP/1)
**اللغة المستهدفة:** Rust
**المنصة المرجعية:** Linux / Internet / Data Center / LAN
**المرجع الأساسي:** GTP/1.1 Comprehensive Technical Specification

---

## 1. الغرض من هذه الورقة

هذه الورقة وثيقة قرار معماري مستقلة مشتقة من المواصفة التقنية الشاملة لـ GTP/1.1. هدفها حسم سؤال جوهري قبل بدء التنفيذ:

> هل GTP بروتوكول جديد يُعاد تصميمه من الصفر، أم مجرد تجميع لمكونات KCP وQUIC وHoma والمكتبات الجاهزة؟

القرار المعتمد هنا هو:

> **GTP بروتوكول نقل جديد ومخصص للألعاب من حيث الـ wire semantics، ونموذج الرسائل، وحالات الاتصال، والاسترجاع، والجدولة، والـ pacing، وإدارة المسار، وواجهة التطبيق؛ لكنه يستخدم مكونات تنفيذية جاهزة ومجربة عندما تكون هذه المكونات infrastructure أو primitives وليست جزءاً من هوية GTP.**

وبالتالي لا يتم تعريف GTP على أنه fork من KCP أو QUIC، ولا على أنه wrapper حول مكتبة QUIC جاهزة.

---

## 2. القرار المعماري الأساسي

المواصفة الأصلية تصف GTP كطبقة نقل متخصصة للألعاب تعمل فوق UDP، وتؤكد أنه لا ينسخ أي بروتوكول بصورة كاملة. وهي تستلهم من KCP الانتقائية والمرونة، ومن QUIC مفاهيم Connection ID وpacket numbering وACK ranges وRTT/loss recovery وpath validation، ومن QUIC DATAGRAM فكرة datagrams غير الموثوقة مع congestion control، ومن Homa أفكار scheduling الموجه بالرسائل ووعي المستقبل والـ deadline، ومن Linux UDP آليات GSO/GRO وbatching وio_uring، ومن Rust ownership وzero-copy views وthread-local state. 

هذا يعني أن هناك ثلاثة مستويات منفصلة يجب ألا تختلط:

```text
A. GTP Protocol Definition
   ما هو GTP على السلك وفي state machines؟

B. GTP Implementation Infrastructure
   كيف ننفذ GTP بكفاءة في Rust/Linux؟

C. External Design References
   ما الذي نتعلمه من QUIC/KCP/Homa وغيرها؟
```

---

## 3. القاعدة الذهبية للمشروع

### 3.1 البروتوكول ملك GTP

كل شيء يحدد هوية GTP أو سلوكه الشبكي المعياري يجب أن يُعرّف داخل GTP، وليس استعارة wire semantics من بروتوكول آخر.

يشمل ذلك:

- packet format.
- frame model.
- message semantics.
- message identity.
- packet identity.
- ordering semantics.
- reliability semantics.
- freshness/deadline semantics.
- ACK semantics.
- loss declaration rules.
- retransmission policy.
- scheduler semantics.
- pacing contract.
- congestion-control interface.
- path state machine.
- CID semantics.
- handshake state machine.
- anti-amplification behavior.
- replay and packet-state rules.
- GTP application API.

### 3.2 المكتبة ملك طبقة التنفيذ

لا يعاد اختراع primitive ناضج لمجرد أن البروتوكول جديد.

يمكن إعادة استخدام:

- Linux UDP facilities.
- socket2.
- io_uring.
- runtime adapters.
- audited cryptographic implementations.
- low-level byte-view tooling مثل zerocopy.

لكن لا يجوز أن تتحول هذه المكونات إلى مصدر غير معلن لسلوك GTP المعياري.

### 3.3 المرجع ليس dependency

KCP وQUIC وHoma وs2n-quic وquinn وغيرها تُستخدم لفهم التصميم، ودراسة الحلول المجربة، واختبار قراراتنا مقابلها، وليس كي يصبح GTP معتمداً عليها كـ protocol engine.

---

## 4. تعريف الطبقات الثلاث

### 4.1 GTP-Native

مكوّن يُعتبر GTP-Native عندما يحدد semantics أو state machine أو wire behavior أو scheduling/recovery behavior الخاص بالبروتوكول.

**قاعدة القرار:**

> إذا غيّر المكوّن معنى packet/message أو طريقة تعامل الطرفين معه، فهو جزء من GTP ويجب أن يكون تحت ملكية المشروع.

### 4.2 Reused Implementation Component

مكوّن تنفيذي لا يحدد معنى GTP على السلك، بل يوفر آلية تشغيل أو primitive منخفض المستوى.

أمثلة:

- socket2.
- io_uring.
- Linux UDP.
- GSO/GRO.
- crypto backend.
- Tokio/Monoio adapters.

**قاعدة القرار:**

> يمكن استبدال implementation component دون تغيير GTP wire protocol أو semantics.

### 4.3 Design Reference

حل أو مشروع يُدرس لتبني فكرة أو نمط هندسي، لكن لا يُربط به GTP عند التنفيذ.

أمثلة:

- QUIC.
- KCP.
- Homa.
- s2n-quic.
- Quinn.
- BBR-like concepts.

---

## 5. Architecture Decision Matrix

| المكوّن | التصنيف | القرار | ملكية الكود | dependency إلزامية؟ |
|---|---|---|---|---|
| GTP Wire Format | GTP-Native | يُكتب داخل GTP | GTP | لا |
| Common Header | GTP-Native | يُكتب داخل GTP | GTP | لا |
| Long Header | GTP-Native | يُكتب داخل GTP | GTP | لا |
| Short Header | GTP-Native | يُكتب داخل GTP | GTP | لا |
| Connection ID | GTP-Native | تعريف وسلوك مستقل | GTP | لا |
| Packet Number | GTP-Native | تعريف مستقل | GTP | لا |
| Message ID | GTP-Native | تعريف مستقل | GTP | لا |
| Fragment ID | GTP-Native | تعريف مستقل | GTP | لا |
| Transmission ID | GTP-Native | تعريف مستقل | GTP | لا |
| State Sequence | GTP-Native | تعريف مستقل | GTP | لا |
| Generation ID | GTP-Native | تعريف مستقل | GTP | لا |
| UNRELIABLE | GTP-Native | semantic GTP | GTP | لا |
| UNRELIABLE_SEQUENCED | GTP-Native | semantic GTP | GTP | لا |
| RELIABLE_UNORDERED | GTP-Native | semantic GTP | GTP | لا |
| RELIABLE_ORDERED | GTP-Native | semantic GTP | GTP | لا |
| ACK Format | GTP-Native | إعادة تصميم متوافقة مع GTP | GTP | لا |
| ACK Ranges | GTP-Native | implementation داخل GTP | GTP | لا |
| Adaptive ACK Frequency | GTP-Native | مفهوم GTP مستلهم من QUIC | GTP | لا |
| RTT Estimation | GTP-Native | تنفيذ ضمن recovery | GTP | لا |
| Loss Detection | GTP-Native | تنفيذ ضمن recovery | GTP | لا |
| Selective Recovery | GTP-Native | message/frame based | GTP | لا |
| Retransmission Cancellation | GTP-Native | مرتبط بالـ freshness/generation | GTP | لا |
| Deadline Engine | GTP-Native | تصميم خاص بالألعاب | GTP | لا |
| Freshness Filtering | GTP-Native | تصميم خاص بالألعاب | GTP | لا |
| Generation-aware Coalescing | GTP-Native | تصميم خاص بالألعاب | GTP | لا |
| Supersession | GTP-Native | semantic contract | GTP | لا |
| Scheduler | GTP-Native | تصميم خاص بالألعاب | GTP | لا |
| Weighted Fairness | GTP-Native | جزء من scheduler | GTP | لا |
| Pacing | GTP-Native | جزء أساسي من transport | GTP | لا |
| Send Budget | GTP-Native | يربط CC + pacing + scheduler | GTP | لا |
| Congestion Controller API | GTP-Native | abstraction مستقلة | GTP | لا |
| CUBIC Baseline | Algorithm | baseline | GTP adapter/implementation | ليست dependency إلزامية |
| BBR-inspired CC | Experimental | تجريبي | GTP | لا |
| GTP-CC | Experimental | مستقبل غير mandatory | GTP | لا |
| Delivery-rate telemetry | GTP-Native | جزء من recovery/CC telemetry | GTP | لا |
| ECN processing | GTP-Native | integrated into CC | GTP | لا |
| Path State | GTP-Native | state machine خاصة | GTP | لا |
| PATH_CHALLENGE | GTP-Native | frame GTP | GTP | لا |
| PATH_RESPONSE | GTP-Native | frame GTP | GTP | لا |
| NAT Rebinding | GTP-Native | سلوك GTP | GTP | لا |
| Migration Logic | GTP-Native | سلوك GTP | GTP | لا |
| Anti-Amplification | GTP-Native | قاعدة GTP | GTP | لا |
| Handshake | GTP-Native | state machine خاصة | GTP | لا |
| Stateless Validation | GTP-Native | سلوك GTP | GTP | لا |
| AEAD Interface | GTP-Native abstraction | interface داخل GTP | GTP | لا |
| AEAD primitive | Reused implementation | استخدام مكتبة مدققة | External backend | نعم على مستوى implementation |
| Packet Codec | GTP-Native | custom hot-path codec | GTP | لا |
| Zero-copy Views | Implementation support | استخدام library أو primitives | External + GTP wrapper | لا |
| Packet Pools | GTP-Native | إدارة مخصصة للـ workload | GTP | لا |
| Transmission Pool | GTP-Native | إدارة recovery records | GTP | لا |
| Connection Table | GTP-Native | تصميم worker-local/sharded | GTP | لا |
| Timer Architecture | GTP-Native policy | implementation profiling-driven | GTP + optional crate | لا |
| Portable UDP | Backend | OS/socket facility | External OS API | لا |
| Linux UDP | Backend | استخدام Linux APIs | Linux | نعم كمنصة مرجعية |
| socket2 | Reused library | integration | External | اختياري |
| io_uring | Reused mechanism | backend | Linux/library | اختياري |
| GSO / UDP_SEGMENT | Reused backend feature | optimization | Linux | اختياري |
| GRO | Reused backend feature | optimization | Linux | اختياري |
| Tokio | Runtime | adapter فقط | External + adapter | لا |
| Monoio | Runtime | adapter/candidate | External + adapter | لا |
| AF_XDP | Future Backend | لاحقاً | External/Linux | لا في v1 |
| DPDK | Future Backend | لاحقاً | External | لا في v1 |
| FEC | Future Extension | لا يدخل mandatory v1 | — | لا |
| Multipath | Future Extension | لا يدخل mandatory v1 | — | لا |
| 0-RTT | Future Extension | لا يدخل mandatory v1 | — | لا |
| s2n-quic | Design/Implementation Reference | دراسة فقط | خارج GTP | لا |
| Quinn | Design/Implementation Reference | دراسة فقط | خارج GTP | لا |
| QUIC | Design Reference | source of concepts | خارج GTP | لا |
| KCP | Design Reference | source of reliability lessons | خارج GTP | لا |
| Homa | Design Reference | source of scheduling ideas | خارج GTP | لا |
| BBR | Design Reference | source of delivery-rate/control ideas | خارج GTP | لا |

---

## 6. ما يجب كتابته فعلياً داخل GTP

### 6.1 طبقة Wire

يجب أن تملك GTP بشكل كامل:

```text
Header layout
Version
Flags
Header length
Connection ID
Packet number
Timing metadata
Payload length
ACK section
Frames
Authentication boundary
```

السبب أن الـ wire format يحدد interoperability مع أي implementation مستقبلية لـ GTP. لا يجوز ربطه بكائنات أو structures خاصة بمكتبة QUIC أو KCP.

### 6.2 نموذج الهوية

يجب أن يظل الفصل واضحاً بين:

```text
Packet Number
Message ID
Fragment ID
Transmission ID
State Sequence
Generation ID
```

هذا الفصل جزء من تصميم GTP وليس مجرد implementation detail، لأن GTP يسمح بإعادة إرسال frame/message بدلاً من إعادة إرسال packet كاملاً.

### 6.3 Message Semantics

الطبقات الأربع يجب أن تكون GTP semantics رسمية:

```text
UNRELIABLE
UNRELIABLE_SEQUENCED
RELIABLE_UNORDERED
RELIABLE_ORDERED
```

وتمثل هذه semantics جوهر الاختلاف عن نقل stream تقليدي.

### 6.4 Freshness / Deadline

هذه الطبقة يجب ألا تُستبدل بمكتبة خارجية. transport نفسه يجب أن يعرف:

```text
created_at
remaining_lifetime
deadline
priority
freshness
state generation
supersession
```

وعليه اتخاذ قرار `DROP` أو `SEND` أو `RETX` عندما تسمح semantics بذلك.

### 6.5 Scheduler

الـ scheduler من أكثر الأجزاء التي يجب تطويرها داخل GTP، لأنه يربط:

```text
Priority
Deadline
Freshness
Weighted fairness
Congestion budget
Pacing
```

ولا ينبغي استخدام priority queue عامة كبديل مباشر قبل profiling؛ الورقة الأصلية تقترح deadline buckets وpriority rings وsmall heaps بحسب workload.

### 6.6 Recovery

الـ recovery محلي لـ GTP ويجب أن يدعم:

- ACK ranges.
- RTT estimation.
- loss declaration.
- packet tracking.
- logical-message retransmission.
- retransmission cancellation.
- duplicate handling.
- reordering.
- deadline-aware recovery.

ويجب الحفاظ على الفصل بين:

```text
loss declaration
retransmission policy
congestion reaction
```

### 6.7 Path Management

يجب أن تكون state machine الخاصة بـ:

```text
PATH_CHALLENGE
PATH_RESPONSE
NAT rebinding
path validation
migration
anti-amplification
```

ملكية GTP بالكامل.

### 6.8 GTP API

واجهة التطبيق يجب أن تكون semantic لا packet-centric، مثل:

```rust
send_unreliable(data)
send_sequenced(key, seq, data)
send_reliable_unordered(data)
send_reliable_ordered(group, data)

send_state(entity_id, generation, sequence, deadline, data)
send_event(event_id, data)
send_rpc(rpc_id, data)
```

ويجب أن تستطيع API التعبير عن:

```text
reliability
ordering
freshness
deadline
cancellation
supersession
priority
```

---

## 7. ما يجب إعادة استخدامه كمكتبات أو آليات جاهزة

### 7.1 Cryptography

لا يُعاد اختراع AEAD primitive. يجب أن يعرّف GTP abstraction مثل:

```rust
trait PacketProtector {
    fn seal(...);
    fn open(...);
}
```

ثم تستخدم implementation موثوقة ومدققة خلف abstraction. بهذا تظل semantics الخاصة بالحماية داخل GTP، بينما primitive الحسابية تأتي من implementation جاهزة.

### 7.2 socket2

يُستخدم للوصول إلى socket operations مثل `sendmsg` وvectored I/O وخيارات socket مع الحفاظ على portability أفضل من إدارة libc calls يدوياً في المشروع بأكمله.

### 7.3 io_uring

io_uring ليس جزءاً من protocol semantics. هو backend يمكنه توفير multishot receive وbuffer groups وغيرها عند توافر kernel capabilities المناسبة.

GTP يجب أن يعمل أيضاً دون io_uring عبر backend بديل.

### 7.4 Linux GSO/GRO

`UDP_SEGMENT`/GSO وUDP GRO يجب النظر إليهما كـ transport I/O optimizations فقط.

القاعدة:

```text
GSO/GRO ON  → performance optimization
GSO/GRO OFF → identical GTP semantics
```

وهذا شرط أساسي حتى لا يصبح wire protocol مرتبطاً بقدرات kernel معينة.

### 7.5 Runtime

GTP core لا يجب أن يصبح متشابكاً مع Tokio أو Monoio.

المعمارية المستهدفة:

```text
                gtp-core
                    │
         ┌──────────┼──────────┐
         │          │          │
      Tokio       Monoio   custom loop
       adapter     adapter
```

يمكن لاحقاً إضافة backend آخر دون إعادة كتابة transport semantics.

### 7.6 Zero-copy

يمكن استخدام `zerocopy` كأداة مساعدة لبناء typed byte views، لكن ownership وvalidation وlifetime policy تبقى مسؤولية GTP.

القاعدة:

> المكتبة تساعد GTP على قراءة bytes بكفاءة؛ لكنها لا تحدد معنى packet.

---

## 8. ما يجب استخدامه كـ Design Reference فقط

### 8.1 QUIC

يُستخدم لدراسة:

- packet numbering.
- ACK ranges.
- RTT/loss recovery.
- Connection IDs.
- path validation.
- anti-amplification.
- version agility.
- congestion-control integration.

لكن GTP لا يستورد QUIC packet format أو stream machinery كقاعدة إلزامية.

### 8.2 KCP

يُستخدم لدراسة:

- selective ARQ.
- tunability.
- low-overhead transport behavior.
- delivery telemetry.
- pacing/congestion-control evolution.

لكن GTP لا يعتمد على KCP runtime أو session engine.

### 8.3 Homa

يُستخدم كمرجع لأفكار:

- message-oriented scheduling.
- urgency/deadline awareness.
- receiver-aware transport concepts.

لكن scheduling النهائي يجب أن يكون مصمماً وفق متطلبات الألعاب وGTP.

### 8.4 s2n-quic وQuinn

هما implementation references لفحص هندسة Rust، وpacing، وGSO، وPMTU، واختبارات QUIC، وAPI design، وdatagrams، لكن لا ينبغي أن يكون أي منهما protocol engine داخل GTP.

### 8.5 BBR-like Concepts

تستخدم delivery-rate measurements وRTT وinflight وECN كمرجع لتصميم controller تجريبي، لكن baseline الأول يجب أن يكون واضحاً ومقابلاً للاختبار، وتظل GTP-CC وظيفة تجريبية إلى أن تثبت نتائجها.

---

## 9. معيار فصل البروتوكول عن implementation

يجب أن تنجح GTP architecture في الاختبار التالي:

### اختبار 1 — استبدال backend

يجب أن نتمكن من تشغيل:

```text
Linux UDP
io_uring
Tokio
Monoio
portable socket backend
```

مع بقاء packet semantics نفسها.

### اختبار 2 — استبدال crypto backend

يجب تغيير implementation cryptographic backend دون تغيير GTP wire semantics.

### اختبار 3 — تعطيل GSO/GRO

يجب ألا يتغير سلوك reliability أو ordering أو deadlines بسبب تشغيل أو تعطيل GSO/GRO.

### اختبار 4 — تغيير congestion controller

يجب أن يمكن تبديل:

```text
CUBIC baseline
experimental BBR-like
future GTP-CC
```

من خلال abstraction نفسها.

### اختبار 5 — إزالة جميع المراجع الخارجية

إذا أزلنا QUIC/KCP/Homa libraries من dependencies، يجب أن يبقى GTP protocol implementation قابلاً للبناء. هذا الاختبار مهم جداً لإثبات أن QUIC/KCP ليستا hidden dependencies.

---

## 10. Architecture Ownership Rule

يجب اعتماد القاعدة التالية في code review:

> **أي كود يقرر "ماذا تعني packet/message" ينتمي إلى GTP. أي كود يقرر "كيف تصل bytes إلى kernel/NIC" يمكن أن ينتمي إلى backend خارجي.**

أمثلة:

```text
ACK range semantics       → GTP
ACK syscall               → backend

Reliable unordered        → GTP
UDP socket                → backend

Deadline                  → GTP
Timerfd/io_uring timer    → backend

CID routing semantics     → GTP
RSS/NIC queue setup       → backend

AEAD nonce semantics      → GTP
AES/ChaCha implementation → crypto library
```

---

## 11. الحدود بين crates المقترحة

البنية المستهدفة:

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
├── gtp-bench
├── gtp-sim
├── gtp-fuzz
├── gtp-cli
└── wireshark-gtp
```

هذه البنية لا تعني أن كل crate يجب أن يظهر منذ اليوم الأول. يمكن البدء بعدد أقل ثم الفصل عندما يصبح ذلك مفيداً. لكن الحدود المعمارية يجب أن تكون واضحة منذ البداية.

---

## 12. Dependency Policy

### 12.1 Allowed

المكتبة الخارجية مقبولة عندما تكون:

- primitive منخفض المستوى.
- implementation مدققة لمشكلة معروفة.
- backend platform integration.
- أداة testing/fuzzing/benchmarking.
- utility لا تحدد GTP semantics.

### 12.2 Discouraged

المكتبة الخارجية تكون غير مفضلة عندما:

- تفرض model مختلفاً للرسائل.
- تفرض stream semantics غير مطلوبة.
- تفرض scheduler غير مناسب للألعاب.
- تربط GTP runtime واحداً بشكل لا يمكن فصله.
- تجعل تغيير backend يغير wire semantics.

### 12.3 Forbidden as Core Dependency

لا ينبغي أن يعتمد `gtp-core` على:

```text
KCP engine
QUIC protocol engine
Homa protocol engine
full-stack QUIC runtime
```

حتى لو كانت هذه المشاريع تحتوي على implementation ممتازة لمكونات نحتاجها.

السبب ليس الجودة، بل **ملكية semantics والاستقلال المعماري**.

---

## 13. لماذا لا نستخدم QUIC مباشرة ونضيف فوقه Game Scheduler؟

لأن ذلك سيكون حلاً مختلفاً عن GTP.

إذا كان transport السفلي ما زال يفرض semantics الخاصة به، فإن game scheduler سيعمل داخل القيود التي صممها transport الأصلي.

أما GTP فهدفه أن تكون الرسالة منذ البداية معروفة كـ:

```text
latest state
reliable event
ordered event
expired state
supersedable state
critical control
```

ثم تُحوّل هذه الدلالة إلى:

```text
queue policy
retransmission policy
priority
scheduler decision
congestion budget
pacing
```

أي أن game semantics تدخل transport نفسه، وليس layer خارجياً بعد إنشاء transport مختلف.

---

## 14. لماذا لا نستخدم KCP مباشرة ونضيف freshness؟

KCP يقدم selective reliability وخصائص مفيدة، لكن نموذج GTP يتجاوز reliable UDP التقليدي بإضافة هوية مستقلة للـ packet والـ message والـ state generation، وبإدخال deadline/freshness إلى admission وscheduling وretransmission.

في GTP يمكن أن تكون الرسالة reliable من حيث المبدأ ولكن تصبح retransmission عديمة القيمة بسبب وصول generation أحدث. لذلك يجب أن تكون هذه السياسة جزءاً من transport semantics نفسه.

---

## 15. المخاطر التي تمنع الانحراف المعماري

### 15.1 QUIC Clone Risk

العلامة:

```text
نضيف features QUIC واحدة تلو الأخرى
```

النتيجة:

GTP يصبح QUIC variant بدلاً من transport جديد.

**القاعدة:** نأخذ behavior الذي نحتاجه، لا protocol architecture بالكامل.

### 15.2 KCP Fork Risk

العلامة:

```text
نعدل KCP حتى يصبح مناسباً للألعاب
```

النتيجة:

نصبح مرتبطين بالـ KCP model ثم نضيف استثناءات كثيرة.

**القاعدة:** نستفيد من selective ARQ والتجارب العملية، ثم نبني recovery model الخاص بـ GTP.

### 15.3 Feature Explosion

إضافة:

```text
FEC
Multipath
0-RTT
DPDK
AF_XDP
compression
custom BBR
```

قبل إثبات الحاجة ستزيد correctness burden وتشتت المشروع.

### 15.4 Premature Kernel Bypass

لا يبدأ DPDK أو AF_XDP إلا بعد إثبات أن kernel/network stack هو bottleneck الفعلي عبر profiling.

### 15.5 Over-abstracting

لا يجب إنشاء طبقات interfaces لا تضيف قيمة. abstraction مقبول عندما يسمح بتبديل backend دون المساس بـ protocol core.

### 15.6 Under-abstracting

على الجانب الآخر، لا ينبغي وضع Linux-specific types داخل `gtp-core` أو ربط core مباشرة بـ Tokio/Monoio.

---

## 16. قواعد code ownership

### GTP-owned code

يجب أن يراجع فريق البروتوكول:

```text
wire codec
frame parser
message semantics
recovery
ACK
loss detection
scheduler
pacing policy
congestion interface
path state
handshake
GTP API
```

### Platform-owned code

يمكن عزله في modules منفصلة:

```text
Linux socket setup
io_uring integration
GSO/GRO
RSS tuning
NIC interaction
```

### External-owned code

يُفضّل عدم نسخ libraries الخارجية إلى المشروع إلا لضرورة واضحة:

```text
crypto primitives
runtime
socket utilities
benchmark tooling
fuzzing infrastructure
```

---

## 17. اختبار الاستقلالية المعمارية

قبل إعلان GTP/1 implementation مستقراً يجب إثبات ما يلي:

### Protocol Independence

تشغيل GTP دون أي QUIC/KCP engine dependency.

### Backend Independence

تشغيل نفس protocol core فوق أكثر من I/O backend.

### Crypto Independence

استبدال crypto backend دون تعديل wire semantics.

### CC Independence

تبديل congestion controller من خلال GTP interface.

### Runtime Independence

تشغيل core مع event-driven integration مختلف.

### Performance Independence

تشغيل benchmark بنفس workload عبر:

```text
portable UDP
Linux batch I/O
GSO/GRO
io_uring
```

ثم مقارنة cost دون تغيير semantics.

---

## 18. ترتيب التنفيذ الناتج عن هذا القرار

### Phase A — GTP-owned Core

```text
wire format
packet/frame types
CID
packet number
message semantics
state machine
```

### Phase B — Recovery

```text
ACK
ACK ranges
RTT
loss
reordering
duplicate handling
selective retransmission
```

### Phase C — GTP Game Semantics

```text
deadline
freshness
state generation
supersession
scheduler
priority
```

### Phase D — Congestion / Pacing

```text
CC interface
CUBIC baseline
pacing
send budget
ECN
adaptive ACK
```

### Phase E — Internet Path

```text
handshake
AEAD
anti-amplification
path validation
NAT rebinding
migration
```

### Phase F — Performance Backend

```text
batch I/O
Linux UDP
GSO/GRO
memory pools
thread-per-core
io_uring
runtime adapters
```

### Phase G — Experimental

```text
BBR-like CC
GTP-CC
FEC
AF_XDP
DPDK
Multipath
0-RTT
```

لا تدخل Phase G إلى المنتج الأساسي قبل benchmark وsimulation وfuzzing وإثبات الحاجة.

---

## 19. Definition of Done لكل طبقة

### Wire

لا توجد ambiguity في parsing، وpacket format ثابت وversioned وقابل للتوسعة.

### Recovery

يمكن إعادة بناء reliable message مفقودة دون افتراض إعادة إرسال packet كامل.

### Scheduler

يمكن إثبات أن stale state تتعرض للإسقاط قبل أن تملأ queue، وأن critical traffic يخضع للـ congestion budget ولا يتجاوزه.

### Congestion

يظهر behavior مفهوماً وعادلاً مع TCP/QUIC في الاختبارات، ولا يؤدي إلى congestion collapse.

### Backend

يمكن تعطيل optimization مثل GSO/GRO/io_uring دون تغيير semantics.

### Runtime

نفس protocol core يعمل مع أكثر من integration model.

---

## 20. Benchmarks المطلوبة لإثبات صحة القرار

يجب ألا يقتصر القياس على throughput.

المؤشرات الأساسية:

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
snapshot age
stale drop rate
deadline miss rate
useful delivery ratio
```

ويجب استخدام matrix تشمل packet sizes وpacket rates وRTT وloss وreordering كما ورد في المواصفة الأصلية.

---

## 21. معيار نجاح GTP

لا يُعتبر GTP ناجحاً لأنه أسرع في raw throughput فقط.

الهدف الحقيقي هو أن يحافظ على:

```text
fresh input delivery
fresh state delivery
fast critical-event recovery
bounded tail latency
fair congestion behavior
stable CPU cost
```

خصوصاً تحت:

```text
loss
jitter
reordering
RTT inflation
queue pressure
variable path conditions
```

---

## 22. Final Architecture Statement

القرار الرسمي لهذه الورقة هو:

```text
                    GTP/1
                       │
          ┌────────────┼────────────┐
          │            │            │
          ▼            ▼            ▼
     GTP Protocol   GTP Runtime   External References
          │            │            │
          │            │            ├─ QUIC
          │            │            ├─ KCP
          │            │            ├─ Homa
          │            │            ├─ BBR concepts
          │            │            ├─ s2n-quic
          │            │            └─ Quinn
          │            │
          │            ├─ socket2
          │            ├─ io_uring
          │            ├─ Tokio adapter
          │            ├─ Monoio adapter
          │            ├─ Linux UDP
          │            └─ GSO/GRO
          │
          ├─ Wire
          ├─ Message Semantics
          ├─ Recovery
          ├─ Scheduler
          ├─ Pacing
          ├─ CC Interface
          ├─ Path
          ├─ Handshake
          ├─ Security Boundary
          └─ Game-aware transport API
```

### الخلاصة

**GTP ليس مجموعة مكتبات مدمجة تحت اسم جديد.**

**GTP هو بروتوكول جديد تملك architecture الخاصة به، ويستفيد من أفضل الخوارزميات والأفكار المثبتة دون أن يرث architecture كاملة من أي بروتوكول آخر.**

يمكن الاستفادة من QUIC دون استخدام QUIC implementation.

يمكن الاستفادة من KCP دون استخدام KCP engine.

يمكن الاستفادة من Homa دون نسخ Homa transport.

يمكن استخدام io_uring دون جعل io_uring جزءاً من protocol semantics.

يمكن استخدام crypto library جاهزة دون جعل المكتبة هي security model الخاصة بـ GTP.

وهذا الفصل هو ما يحافظ على استقلال GTP ويمنع المشروع من الانحراف إلى QUIC fork أو KCP fork.

---

## 23. Source Basis

هذه الورقة مبنية مباشرة على الاستنتاجات والقرارات المعمارية الموجودة في:

**GTP/1.1 Comprehensive Technical Specification** بتاريخ 28 أغسطس 2026، خصوصاً الأقسام التي تحدد أن GTP لا ينسخ أي بروتوكول بصورة كاملة، وأنه يستخدم QUIC/KCP/Homa كمصادر أفكار، وأن Linux GSO/GRO وio_uring وTokio/Monoio تنتمي إلى طبقة التنفيذ، وأن s2n-quic وQuinn مراجع هندسية وليستا protocol engines، وأن القرار النهائي هو بناء transport متخصص فوق UDP مع semantics خاصة بالألعاب.

---

## 24. Architecture Decision Summary

| السؤال | القرار النهائي |
|---|---|
| هل GTP بروتوكول جديد؟ | **نعم** |
| هل هو clone لـ QUIC؟ | **لا** |
| هل هو fork لـ KCP؟ | **لا** |
| هل نعيد كتابة كل شيء من الصفر؟ | **لا** |
| هل نعيد استخدام primitives منخفضة المستوى؟ | **نعم** |
| هل نعيد استخدام QUIC/KCP كـ engines؟ | **لا** |
| هل نأخذ خوارزميات وأفكاراً مجربة؟ | **نعم** |
| هل نملك GTP wire semantics؟ | **نعم، بالكامل** |
| هل GTP core يعتمد على runtime معين؟ | **لا** |
| هل GSO/GRO جزء من wire protocol؟ | **لا** |
| هل io_uring جزء من protocol semantics؟ | **لا** |
| هل cryptographic primitive يُعاد اختراعه؟ | **لا** |
| هل security boundary يملكه GTP؟ | **نعم** |
| هل scheduler مخصص للألعاب؟ | **نعم** |
| هل freshness/deadline جزء من transport؟ | **نعم** |
| هل experimental features إلزامية في v1؟ | **لا** |

---

**القرار النهائي:**

> **Build the protocol. Reuse the infrastructure. Study the proven protocols. Do not inherit their architecture unless the requirement explicitly demands it.**
