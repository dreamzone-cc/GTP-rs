# GTP Protocol — G2 Gap Analysis & Comprehensive Validation Plan

## 1. الغرض من الوثيقة

تهدف هذه الورقة إلى تحويل نتائج جولة **G2 + Route-Prototype** إلى خطة تقنية منهجية لسد الفجوات المتبقية، والتحقق من كفاءة وأداء واستقرار بروتوكول GTP وآلية اختيار المسار، قبل الانتقال إلى مراحل التفعيل الفعلي للتبديل.

تركز الوثيقة على:

- كشف النقائص التقنية والاختبارية الحالية.
- تحديد ما يجب مراجعته أو تنفيذه.
- بناء مصفوفة اختبارات تغطي الصحة، الدقة، الأداء، الاستقرار، التعافي، والأمان التشغيلي.
- ضمان أن قرارات اختيار المسار لا تعتمد على قياسات مضللة أو غير مكتملة.
- ضمان التكيف الديناميكي مع تغيرات الشبكة دون oscillation أو flapping أو قرارات متسرعة.
- إنشاء خط أساس قابل لإعادة الاختبار والمقارنة في G3/G4 وما بعدهما.

هذه الوثيقة لا تعتبر أي مرحلة تنفيذية محددة مكتملة لمجرد ورودها هنا؛ حالة كل بند يجب أن تحددها الاختبارات والكود الفعلي.

---

## 2. خط الأساس الحالي

أثبتت جولة G2 + Route-Prototype وجود سلسلة تشغيلية متكاملة تقريبًا:

```text
wire timestamp
    ↓
authenticated RX
    ↓
estimator
    ↓
per-direction aggregates
    ↓
MeasurementReport exchange
    ↓
score / confidence
    ↓
deterministic selection
    ↓
explainable shadow verdict
```

كما تم إثبات:

- حتمية تشغيلية مع نفس الـ seed في اختبارات connection-driven.
- استقلال القياسات لكل اتجاه.
- bounded event queue مع drop-oldest وعداد drops.
- صحة selector واختبارات reason codes وtie-breaks.
- اختيار قائم على قياسات فعلية من SimulationRunner.
- تبادل MeasurementReport عبر المسار الفعلي داخل الاختبار.
- عدم وجود actuation في مسار الـ shadow prototype.
- ثلاث جولات WAN ناجحة ومتسقة نسبيًا.
- نجاح الـ full gate محليًا وعلى الـ VPS.

مع ذلك، لا تزال هناك فجوة بين:

> **قياس الشبكة واختيار المسار في وضع shadow**

وبين:

> **اتخاذ قرار تبديل آمن ومستقر ومتكيف ديناميكيًا في ظروف شبكة متغيرة ومضطربة.**

---

# 3. الفجوات الرئيسية

## 3.1 فجوة معايرة الـ Scoring Model

### الحالة
الـ scorer يثبت صحة حساباته وترتيب المسارات، لكن لم تثبت معايرته على طيف واسع من ظروف الشبكة.

### المطلوب
بناء dataset اصطناعي وحقيقي يغطي على الأقل:

- low latency / low jitter.
- low latency / high jitter.
- high latency / low jitter.
- high latency / high jitter.
- asymmetric forward/reverse conditions.
- transient spikes.
- sustained degradation.
- recovery after degradation.
- intermittent impairment.
- competing advantages بين latency وjitter.
- اختلاف أحجام العينات.
- اختلاف أعمار القياسات.

### اختبارات القبول
- ترتفع/تنخفض النتيجة بصورة monotonic عندما يتحسن/يسوء العامل المستهدف.
- لا يغير ID أو ordering النتيجة.
- لا تؤدي عينة صغيرة جدًا إلى winner زائف.
- عدم اليقين يؤدي إلى hold بدل قرار عدواني.
- الحالات الحدية deterministic بالكامل.

---

## 3.2 غياب Loss Axis حقيقي

### الحالة
تم استبعاد loss من القرار عمدًا بسبب `loss_ratio()` المختلط الوحدات، وهو قرار صحيح في هذه المرحلة.

### الفجوة
لا يمكن اعتبار scorer حاليًا تمثيلًا شاملًا لجودة المسار.

### المطلوب
إضافة loss measurement مستقل وواضح الوحدات لاحقًا، مع تعريف صريح لـ:

- packet loss.
- burst loss.
- consecutive loss.
- loss over sliding window.
- loss confidence.
- interaction بين loss والlatency/jitter.

### الاختبارات
- known-loss injection بنسبة 0%, 0.1%, 1%, 5%, 10%, 25%, 50%.
- burst-loss patterns.
- random-loss patterns.
- loss recovery.
- منع أي وحدة غير متوافقة من دخول scorer.

لا يسمح بإدخال loss إلى القرار قبل وجود metric موثوق ومُعاير.

---

## 3.3 غياب نموذج Staleness / Freshness قوي

### الفجوة
المسار قد يبدو ممتازًا وفق تقرير قديم بينما حالته الحالية تغيرت.

### المطلوب
تحديد:

- measurement timestamp.
- report receive timestamp.
- report age.
- maximum acceptable age.
- stale threshold.
- expiry behavior.
- freshness contribution إلى confidence.

### الاختبارات
- تأخير report عمدًا.
- إعادة تشغيل report قديم.
- out-of-order report.
- duplicate report.
- توقف التقارير ثم استئنافها.
- clock offset ضمن الحدود المقبولة.

القاعدة المطلوبة: **قياس ممتاز لكنه قديم لا يجب أن يفوز على قياس حديث موثوق.**

---

# 4. فجوات الاستقرار الديناميكي

## 4.1 Hysteresis

يجب منع switching بسبب فروق ضئيلة.

### اختبارات

```text
A = 0.910
B = 0.911
```

القرار المتوقع: `HOLD`.

ثم:

```text
A = 0.91
B = 0.96
```

مع confidence مرتفع: يسمح بالانتقال إذا استوفت باقي الشروط.

يجب اختبار عدة hysteresis thresholds والتأكد من أن السلوك لا يعتمد على ترتيب وصول القياسات.

---

## 4.2 Anti-Flapping

### الفجوة
لم يُثبت بعد أن selector يستطيع التعامل مع شبكة يتبادل فيها التفوق بين المسارين بسرعة.

### الاختبارات

- A أفضل → B أفضل → A أفضل → B أفضل بتواتر مرتفع.
- تفوق مؤقت لمدة sample واحدة.
- تفوق متكرر لكنه غير مستقر.
- oscillation بالقرب من threshold.

### القياسات المطلوبة

- عدد قرارات switching.
- minimum dwell time.
- average time between switches.
- false switch rate.
- stabilization time.

يجب أن يكون الهدف هو الوصول إلى أفضل مسار **المستقر** وليس أفضل مسار في آخر sample فقط.

---

## 4.3 Minimum Evidence / Sample Count

وجود confidence floor خطوة جيدة، لكن يجب معايرتها.

### اختبارات

- 1 sample.
- 2 samples.
- 5 samples.
- 10 samples.
- 50 samples.
- عينة قليلة ولكن متطابقة.
- عينة كثيرة ولكن شديدة التذبذب.

يجب إثبات أن عدد العينات وحده لا يكفي؛ بل يجب أن يؤثر أيضًا consistency وvariance وfreshness على الثقة.

---

## 4.4 Recovery Hysteresis

يجب فصل شروط:

- الدخول في حالة degraded.
- الخروج منها.
- اختيار مسار بديل.
- العودة للمسار الأصلي.

حتى لا يحدث ping-pong أثناء التعافي.

اختبار أساسي:

```text
Healthy → Degraded → Recovering → Healthy
```

مع قياس زمن الاستقرار وعدد القرارات خلال التعافي.

---

# 5. Dynamic Adaptation Model

## 5.1 التغيرات البطيئة

اختبار شبكة تتدهور تدريجيًا:

```text
low jitter
→ moderate jitter
→ high jitter
→ recovery
```

يجب أن يتحرك score والconfidence والقرار تدريجيًا بدون jumps غير مبررة.

## 5.2 التغيرات السريعة

اختبار spikes قصيرة وعنيفة:

```text
Healthy → spike → Healthy
```

يجب ألا تسبب spike واحدة تبديلًا إلا عند تحقق شروط واضحة.

## 5.3 تغير الاتجاه فقط

اختبار:

- forward degradation فقط.
- reverse degradation فقط.
- الاثنين معًا.

يجب أن يبقى القرار قائمًا على policy اتجاهية واضحة وألا يعيد RTT إخفاء المشكلة.

---

# 6. Direction-Aware Validation

يجب توسيع اختبارات directional independence إلى حالات adversarial.

### الحالات

| الحالة | Forward | Reverse | المتوقع |
|---|---|---|---|
| F1 | ممتاز | ممتاز | Healthy |
| F2 | سيئ | ممتاز | degraded/وفق policy |
| F3 | ممتاز | سيئ | degraded/وفق policy |
| F4 | سيئ | سيئ | reject/poor |
| F5 | spike فقط | مستقر | عدم الخلط |
| F6 | مستقر | spike فقط | عدم الخلط |

يجب توثيق السياسة النهائية بوضوح: هل القرار path-global، أم directional، أم مزيج موزون حسب نوع traffic/class؟

---

# 7. MeasurementReport Exchange Gaps

## 7.1 Reliability Semantics

تم إثبات recovery عبر ReliableOrdered، لكن يجب قياس تكلفة هذا الاختيار.

### اختبارات

- loss أثناء إرسال reports.
- burst loss.
- reordering.
- retransmission storm.
- report backlog.
- sender أبطأ من receiver والعكس.

### مؤشرات

- report delivery latency.
- retransmission count.
- queue occupancy.
- stale-report percentage.
- HOL delay.

يجب التأكد أن آلية القياس لا تصبح سببًا في انخفاض جودة قرار routing نفسه.

---

## 7.2 Backpressure

اختبار producer أسرع بكثير من consumer.

يجب التأكد من:

- bounded memory.
- bounded queue.
- accurate drop counter.
- no deadlock.
- no starvation.
- no unbounded CPU.
- no cascading queue growth بين layers.

---

# 8. RT-2 Event Queue — اختبارات إضافية

نجاح boundedness الحالي جيد، لكنه لا يكفي لإثبات الأداء تحت ضغط مستدام.

### Load Matrix

| Producer Rate | Drain Rate | Duration | Expected |
|---:|---:|---:|---|
| أقل | أعلى | 60s | 0 drops |
| متساوٍ | متساوٍ | 60s | 0/near-zero drops |
| أعلى 2× | ثابت | 60s | bounded drops |
| أعلى 10× | ثابت | 60s | bounded memory |
| burst | ثابت | 60s | recovery |
| extreme burst | ثابت | 60s | no crash |

يجب اختبار capacity values متعددة:

`0, 1, 16, 64, 256, 1024, 4096`.

---

# 9. Performance Validation

## 9.1 CPU

يجب قياس:

- baseline بدون routing.
- routing enabled في shadow.
- routing + reports.
- routing تحت impairment.
- routing تحت high connection count.

المؤشرات:

- process CPU.
- per-thread CPU.
- CPU spikes.
- scheduler contention.

## 9.2 Memory

قياس:

- RSS.
- heap growth.
- allocations/sec.
- memory under sustained load.
- memory after repeated degradation/recovery.

المطلوب إثبات عدم وجود memory leak أو retained history غير محدود.

## 9.3 Throughput

اختبار:

- low traffic.
- medium traffic.
- high traffic.
- maximum sustained traffic.
- multiple concurrent connections.

يجب مقارنة throughput مع وبدون route measurement.

## 9.4 Latency Overhead

قياس overhead الناشئ عن:

- timestamping.
- estimator.
- aggregation.
- report generation.
- report transport.
- selector.

يجب فصل measurement overhead عن application/data-plane overhead.

---

# 10. Long-Run Stability

اختبار soak طويل ضروري قبل switching.

### المقترح

- 1 ساعة كاختبار تطويري سريع.
- 6 ساعات كاختبار nightly.
- 24 ساعة كاختبار release candidate.
- اختبار أطول عند الحاجة قبل الإنتاج.

خلال الاختبار يجب تدوير:

- latency.
- jitter.
- directional asymmetry.
- burst loss.
- report delay.
- queue pressure.
- connection churn.

### شروط الفشل

- memory growth غير مبرر.
- CPU drift.
- queue growth.
- increasing stale reports.
- decision oscillation.
- lost state.
- counter overflow.
- panic/crash.
- deadlock.
- divergence بين peers.

---

# 11. Connection Churn

اختبار إنشاء وإغلاق connections بمعدلات مختلفة.

### الحالات

- sequential open/close.
- high-rate churn.
- simultaneous creation.
- simultaneous teardown.
- reconnect after impairment.
- repeated reconnect loops.

يجب التأكد من:

- عدم تسرب state.
- عدم بقاء measurement state لمسارات ميتة.
- تنظيف queues والتقارير.
- عدم اختلاط state بين connection IDs.

---

# 12. Failure Injection

يجب بناء impairment matrix قابلة لإعادة الإنتاج.

### أنواع الأعطال

- latency injection.
- jitter injection.
- packet loss.
- burst loss.
- reordering.
- duplication.
- temporary blackout.
- asymmetric outage.
- recovery burst.
- report-path impairment.

كل سيناريو يجب أن ينتج:

```text
scenario id
seed
parameters
start/end
expected state
actual state
selected path
confidence
reason code
switch count
recovery time
```

---

# 13. Clock / Timestamp Validation

نظرًا لاعتماد estimator على timestamps، يجب اختبار:

- clock offset.
- clock skew.
- timestamp granularity.
- monotonicity.
- suspend/resume.
- clock adjustment.
- delayed packet containing old timestamp.

يجب ألا يؤدي تغير wall clock إلى كسر determinism أو إلى OWD سالبة/غير منطقية.

عند الإمكان، يجب تفضيل monotonic time semantics داخل العمليات الزمنية المحلية، مع تعريف دقيق لحدود الزمن عبر الأجهزة.

---

# 14. Concurrency and Race Testing

ينبغي اختبار:

- concurrent measurement updates.
- concurrent report receive/send.
- queue drain أثناء enqueue.
- connection teardown أثناء report processing.
- selector أثناء تحديث measurements.
- concurrent path lifecycle transitions.

### المطلوب

- race detector / sanitizer عند توفره.
- stress tests.
- repeated execution بمئات وآلاف التكرارات.
- assertions على state transitions.

---

# 15. Determinism Expansion

حاليًا تم إثبات deterministic event sequences في حالة محددة. يجب توسيع ذلك إلى:

- different queue capacities.
- impairment schedules.
- concurrent connections.
- report exchange.
- selector output.
- reason codes.
- serialized decision logs.
- recovery scenarios.

يجب أن ينتج نفس الـ seed ونفس السيناريو نفس:

```text
event sequence
measurements
aggregates
reports
scores
selection
reason codes
state transitions
```

مع السماح فقط للعناصر الزمنية أو البيئية المعرّفة صراحة بأنها غير deterministic.

---

# 16. Replayability

يجب بناء replay artifact يتيح تشغيل سيناريو محفوظ لاحقًا وإعادة استخراج:

- measurement stream.
- estimator output.
- scores.
- selection decisions.
- state transitions.

الهدف هو أن يصبح أي routing decision قابلًا لإعادة الإنتاج والتحقيق بعد وقوعه.

---

# 17. Structured Decision Logging

قبل تفعيل switching يجب أن تحتوي كل قرارات selector على سجل structured، مثل:

```text
decision_id
connection_id
path_set
measurement_window
sample_count
freshness
score_per_path
confidence_per_path
thresholds
hysteresis
selected_path
previous_path
reason_code
policy_class
state_transition
```

يجب أن يكون log قابلًا للبحث والاختبار آليًا، وليس مجرد نص بشري.

---

# 18. Path Lifecycle

يجب تعريف lifecycle صريح للمسار.

مثال عام:

```text
Unknown
  ↓
Probing
  ↓
Healthy
  ↓
Degraded
  ↓
Unhealthy
  ↓
Recovering
  ↓
Healthy
```

المطلوب تحديد:

- شروط الانتقال.
- شروط العودة.
- minimum dwell time.
- stale expiry.
- failure counters.
- recovery counters.
- hysteresis لكل انتقال.

كل transition يجب أن يمتلك test case مستقلًا.

---

# 19. Per-Class Routing Policy

مع وجود B-12، يجب منع افتراض أن جميع أنواع traffic تحتاج policy واحدة.

ينبغي أن تدعم البنية اختلاف السياسة حسب class عند الحاجة، مع بقاء القرار deterministic وقابلًا للتفسير.

يجب اختبار:

- class disabled.
- class enabled.
- different thresholds.
- class fallback.
- policy conflict.
- default policy.

كما يجب منع policy configuration غير الصالحة من إنتاج قرارات غير متوقعة.

---

# 20. Configuration Strategy

قبل الإنتاج يجب تثبيت قواعد واضحة لـ:

- defaults.
- validation.
- bounds.
- hot reload إن وجد.
- rollback.
- invalid configuration handling.
- compatibility.
- versioning.

ويجب اختبار تغيير الإعدادات أثناء الاتصال، بما في ذلك التغييرات الحساسة مثل thresholds وintervals وqueue capacities.

---

# 21. Shadow Engine Validation

يجب جعل shadow engine مصدرًا مستقلًا للمقارنة قبل actuation.

اختبارات أساسية:

1. selector decision معروفة مسبقًا مقابل expected oracle.
2. shadow لا يغير data plane.
3. shadow لا يغير path state غير المخصص لذلك.
4. shadow لا يسبب packet scheduling changes.
5. shadow يعمل عند measurement loss.
6. shadow يعمل عند stale reports.
7. shadow يوقف القرارات عند kill switch.

---

# 22. Kill Switch / INV-15

قبل G4 يجب اختبار kill switch باعتباره safety control وليس feature عادية.

### الحالات

- enabled.
- disabled.
- toggled أثناء القرار.
- toggled أثناء degradation.
- toggled أثناء recovery.
- configuration invalid.
- process restart.

المطلوب إثبات:

> kill switch = no actuation

بشكل يمكن التحقق منه آليًا.

---

# 23. Pre-G4 Switching Safety Tests

لا يجب الانتقال إلى switching قبل اجتياز مجموعة tests حرجة.

## 23.1 False Positive Switching

مسار غير أفضل يجب ألا يتم اختياره بسبب:

- sample noise.
- report staleness.
- transient spike.
- tie-break artifact.
- confidence error.

## 23.2 False Negative Switching

عندما يصبح المسار الحالي غير صالح بصورة واضحة، يجب أن يستطيع النظام تحديد البديل ضمن زمن bounded.

## 23.3 Failback Safety

بعد التبديل إلى مسار بديل:

- لا يعود مباشرة للمسار القديم.
- ينتظر evidence كافيًا.
- يقيس stability بعد recovery.

---

# 24. KPIs المقترحة

يجب اعتماد مجموعة مؤشرات موحدة لكل round.

### Correctness

- deterministic pass rate.
- selector oracle accuracy.
- reason-code correctness.
- report integrity.
- state-transition correctness.

### Performance

- CPU overhead.
- memory overhead.
- report latency.
- measurement processing latency.
- decision latency.
- throughput overhead.

### Stability

- switches/hour.
- false switch rate.
- flap rate.
- minimum dwell violations.
- stale decision rate.
- recovery time.
- degraded-state duration.

### Robustness

- behavior under loss.
- behavior under jitter.
- behavior under asymmetry.
- behavior under queue overflow.
- behavior under reconnect.
- behavior under long-running load.

---

# 25. Acceptance Gates المقترحة

## Gate A — Measurement Integrity

لا يسمح بالانتقال قبل:

- صحة timestamps.
- صحة aggregation.
- directional independence.
- stale detection.
- deterministic replay.

## Gate B — Selector Safety

يجب اجتياز:

- confidence tests.
- hysteresis.
- adversarial scenarios.
- tie cases.
- oscillation tests.
- stale-report cases.

## Gate C — Sustained Stability

يجب اجتياز:

- soak test.
- queue stress.
- connection churn.
- failure injection.
- memory/CPU bounds.

## Gate D — Shadow/Actuation Separation

إثبات آلي أن shadow لا يستطيع تغيير data plane.

## Gate E — Controlled Switching

لا يتم تفعيله إلا بعد نجاح all prior gates، وبسياسة محدودة وقابلة للإيقاف.

---

# 26. Test Matrix موحدة

| المجال | Tests أساسية | الأولوية |
|---|---|---|
| Determinism | replay / seed / event log / decision log | P0 |
| Directionality | asymmetric impairment | P0 |
| Confidence | sample count / variance / freshness | P0 |
| Hysteresis | tiny advantage / threshold | P0 |
| Anti-flap | alternating winners | P0 |
| Staleness | old / reordered / delayed reports | P0 |
| Loss | random / burst / sustained | P0 |
| Queue | overflow / sustained pressure | P0 |
| Shadow | zero-actuation proof | P0 |
| Performance | CPU / memory / throughput | P0 |
| Soak | 6h / 24h | P0 |
| Churn | reconnect storms | P1 |
| Clock | skew / offset / monotonicity | P1 |
| Concurrency | race/stress | P0 |
| Configuration | validation / reload / rollback | P1 |
| Path lifecycle | all transitions | P0 |
| Decision logging | structured replay | P0 |

---

# 27. Adversarial Scenario Suite

يجب وجود suite ثابتة من السيناريوهات التي تعاد في كل regression run.

### S01 — Stable Winner

مسار واحد أفضل بصورة واضحة.

Expected: clear winner.

### S02 — Near Tie

الفارق صغير جدًا.

Expected: hold.

### S03 — Low Confidence Winner

الفائز score أعلى لكن evidence ضعيف.

Expected: hold.

### S04 — Stale Winner

الفائز لديه قياس ممتاز لكنه قديم.

Expected: reject stale evidence.

### S05 — Flapping

الاثنان يتبادلان التفوق باستمرار.

Expected: stable decision.

### S06 — One-Sided Impairment

اتجاه واحد يتدهور فقط.

Expected: directional-aware behavior.

### S07 — Burst Loss

فقدان متجمع في bursts.

Expected: no false optimism.

### S08 — Transient Spike

تدهور قصير.

Expected: no unnecessary switch.

### S09 — Sustained Failure

تدهور مستمر.

Expected: bounded detection and safe selection.

### S10 — Recovery

المسار السيئ يعود تدريجيًا.

Expected: hysteretic recovery.

### S11 — Report Channel Impairment

التقارير نفسها تتعرض للتأخير والفقد.

Expected: bounded and stale-aware behavior.

### S12 — Queue Saturation

إجبار event queue على overflow طويل.

Expected: bounded memory + correct accounting.

---

# 28. Observability Requirements

يجب أن يستطيع النظام الإجابة آليًا عن الأسئلة التالية:

1. لماذا اختير هذا المسار؟
2. ما القياسات التي بني عليها القرار؟
3. ما عمر هذه القياسات؟
4. ما confidence؟
5. ما threshold الذي تم تجاوزه؟
6. ما سبب عدم اختيار المسار الآخر؟
7. هل حدث switch سابقًا؟
8. هل القرار متأثر بالـ hysteresis؟
9. هل كانت التقارير stale أو ناقصة؟
10. كم استغرق الوصول إلى القرار؟
11. كم استغرق التعافي؟
12. هل تدخل kill switch؟

---

# 29. Benchmark Harness

ينبغي إنشاء benchmark harness موحد يسمح بتشغيل نفس السيناريو:

- محليًا.
- داخل simulation.
- بين جهاز وVPS.
- بين عدة VPS عند الحاجة.

مع output موحد يتيح المقارنة:

```text
scenario
seed
build
configuration
traffic profile
impairment profile
metrics
selection
confidence
switch count
recovery time
CPU
memory
errors
```

الهدف هو منع اختلاف أدوات القياس بين جولة وأخرى.

---

# 30. مقارنة Baseline قبل وبعد Routing

يجب إجراء benchmark pairwise:

```text
Baseline GTP
vs
GTP + measurement
vs
GTP + measurement + shadow
vs
GTP + controlled actuation
```

يجب قياس أثر كل طبقة على:

- latency.
- throughput.
- CPU.
- memory.
- packet delivery.
- application goodput.

حتى لا يتم تحسين اختيار المسار على حساب كفاءة البروتوكول الأساسية.

---

# 31. معايير النجاح طويلة الأمد

الهدف النهائي ليس أن ينجح النظام في اختبار واحد، بل أن يحقق الخصائص التالية باستمرار:

### Correct

القرارات تعتمد فقط على البيانات والسياسة الصحيحة.

### Deterministic

نفس المدخلات تنتج نفس النتيجة ضمن حدود البيئة المعلنة.

### Stable

لا يوجد switching غير ضروري.

### Responsive

التدهور الحقيقي يتم اكتشافه ضمن زمن bounded.

### Adaptive

النظام يعيد تقييم المسارات عند تغير الشبكة.

### Direction-aware

لا تختزل الاختلافات الاتجاهية إلى RTT فقط.

### Freshness-aware

البيانات القديمة لا تقود القرار.

### Explainable

كل قرار قابل للتفسير وإعادة البناء.

### Resource-bounded

CPU والذاكرة والqueues تبقى ضمن حدود معرفة.

### Fail-safe

عند عدم كفاية الأدلة، يكون السلوك الآمن هو hold وليس switch عشوائي.

---

# 32. الأولويات التنفيذية

## P0 — يجب إغلاقها قبل أي Switching فعلي

1. scorer calibration dataset.
2. confidence calibration.
3. freshness/staleness model.
4. hysteresis.
5. anti-flapping.
6. path lifecycle.
7. structured decision log.
8. comprehensive failure injection.
9. queue pressure tests.
10. CPU/memory/throughput benchmarks.
11. long-run soak.
12. concurrency/race validation.
13. deterministic replay expansion.
14. kill-switch validation.
15. loss-axis design/implementation عندما يصبح جاهزًا.

## P1 — قبل التوسع في الإنتاج

1. connection churn.
2. advanced clock validation.
3. configuration hot-reload validation.
4. multi-class routing policies.
5. extensive WAN matrix.
6. benchmark automation.

## P2 — تحسينات لاحقة

1. multi-path optimization.
2. advanced predictive scoring.
3. richer adaptive policy.
4. historical model calibration.
5. automatic scenario generation.

---

# 33. ترتيب التنفيذ المقترح

المسار المقترح هو:

```text
Measurement integrity
        ↓
Freshness + confidence
        ↓
Scorer calibration
        ↓
Hysteresis + anti-flap
        ↓
Path lifecycle
        ↓
Failure injection
        ↓
Performance + soak
        ↓
Structured decision logging + replay
        ↓
Shadow engine validation
        ↓
Controlled switching safety tests
        ↓
G4 actuation
```

ولا ينبغي عكس هذا الترتيب بحيث يتم تفعيل switching قبل اكتمال safety envelope.

---

# 34. Definition of Done

لا تعتبر آلية routing جاهزة للـ controlled actuation حتى يمكن إثبات جميع النقاط التالية باختبارات آلية:

- [ ] القياسات directional وصحيحة.
- [ ] القياسات لا تصبح stale دون اكتشاف ذلك.
- [ ] scorer calibrated على حالات متعددة.
- [ ] confidence يعكس كمية وجودة الأدلة.
- [ ] near-ties لا تسبب switching.
- [ ] oscillation لا تسبب flapping.
- [ ] degradation الحقيقي يؤدي إلى قرار ضمن زمن bounded.
- [ ] recovery لا تسبب failback متكررًا.
- [ ] queue تبقى bounded تحت الضغط.
- [ ] لا توجد memory/CPU regressions غير مقبولة.
- [ ] long-run soak مستقر.
- [ ] concurrency tests ناجحة.
- [ ] deterministic replay متاح.
- [ ] decision logs structured وقابلة للتحقيق.
- [ ] shadow mode لا يستطيع تغيير data plane.
- [ ] kill switch مثبت آليًا.
- [ ] loss measurement الحقيقي جاهز قبل إدخاله في القرار.
- [ ] WAN validation تغطي حالات صحية ومتدهورة ومتغيرة، لا healthy-only فقط.

---

# 35. الخلاصة التقنية

جولة G2 أثبتت أن الأساس المعماري لقياس المسار واختياره قابل للعمل، وأن البروتوكول أصبح يملك spine متكاملة من القياس إلى verdict. الفجوة الحالية ليست في إضافة selector آخر، وإنما في بناء **Safety + Stability Envelope** حول القرار.

الهدف في المرحلة التالية يجب أن يكون إثبات أن النظام يستطيع:

```text
measure
  → understand uncertainty
  → detect stale evidence
  → compare paths
  → wait when evidence is insufficient
  → react when degradation is real
  → avoid flapping
  → recover safely
  → explain every decision
  → remain bounded under load
```

وبذلك يصبح الانتقال من shadow selection إلى actual switching انتقالًا مبنيًا على أدلة واختبارات، وليس مجرد إضافة actuation code.

---

## Appendix A — الحد الأدنى لنتيجة كل Test Run

كل test run يجب أن يحتفظ على الأقل بـ:

```text
Test ID
Scenario ID
Build / commit
Seed
Configuration
Traffic profile
Impairment profile
Duration
Connections
Forward metrics
Reverse metrics
Sample count
Freshness
Confidence
Scores
Selected path
Previous path
Reason code
State transitions
Switch count
Recovery time
CPU
Memory
Queue occupancy
Drops
Errors / panics
Pass / Fail
```

## Appendix B — قاعدة هندسية أساسية

> **لا يجب أن يتخذ GTP قرار routing اعتمادًا على أفضل metric منفردة؛ القرار يجب أن يعتمد على evidence حديث، كافٍ، متسق، ومُعاير، مع آليات واضحة للثبات والتراجع الآمن.**

---

## Appendix C — ملحق التحقق المستقل (Independent Verification Addendum)

> **تاريخ التحقق:** 7 سبتمبر 2026 — جلسة تحقق مستقلة مقابل الكود والاختبارات والبنية الفعلية عند الالتزام `ff85cc7` (تطابق تام بين جهاز التطوير وخادم VPS)، بوابة كاملة خضراء **168/168** وfmt/clippy نظيفان.

### C.1 نتيجة التحقق من ادعاءات خط الأساس (§2) — كلها مؤكدة ✅

| الادعاء في §2 | الحالة | الدليل الفعلي |
| :--- | :---: | :--- |
| سلسلة القياس التشغيلية (timestamp → RX موثّق → estimator → aggregates → report → score/confidence → selection → verdict) | ✅ مؤكد | مبنية ومختبرة عبر `owd.rs` → `connection.rs` → `report.rs` → `score.rs`/`select.rs`؛ مرجعها: `docs/routing/MEASUREMENT-AND-SELECTION-REFERENCE.md` |
| حتمية نفس الـseed في اختبارات connection-driven | ✅ مؤكد | `same_seed_connection_driven_runs_are_byte_identical` ناجح (سجل أحداث وتسليمات متطابقة بايت-بايت) |
| استقلال القياسات لكل اتجاه | ✅ مؤكد | `per_direction_impairment_is_independently_visible` ناجح + جولات WAN الحية (1486 مقابل 892 µs) |
| bounded event queue مع drop-oldest وعداد | ✅ مؤكد | اختبارا RT-2 ناجحان؛ لا تحذيرات إسقاط في التشغيل الحي عند السعة 1024 |
| صحة selector وأكواد الأسباب وtie-breaks | ✅ مؤكد | 13 اختبارًا في gtp-route ناجحة |
| اختيار من قياسات SimulationRunner فعلية | ✅ مؤكد | `selection_from_sim` (المسار السليم يحمل معرفًا أعلى فلا يفسر الترتيب النتيجة) |
| تبادل MeasurementReport عبر المسار الفعلي | ✅ مؤكد | `route_probe_e2e` + ثلاث جولات WAN حية بـ 11/11 تقريرًا لكل جولة |
| لا actuation في مسار الـshadow | ✅ مؤكد | بالبناء (لا مسار تنفيذ) + مؤكد بالاختبار؛ متوافق INV-15 |
| ثلاث جولات WAN ناجحة ومتسقة نسبيًا | ✅ مؤكد | درجات 0.931/0.933/0.943، فقد 0.00%، تعافٍ حي من فقد وسط الجولة (journalctl Msg#36-42) |
| نجاح الـfull gate محليًا وعلى VPS | ✅ مؤكد | 168/0 على الطرفين عند تطابق `07cbc47`؛ أعيد التحقق عند `ff85cc7` |

### C.2 نتيجة التحقق من الفجوات المقترحة — حقيقيّة مع الربط بالخطة المعتمدة

الورقة لا تدّعي إنجازًا زائفًا (نصها الصريح: «حالة كل بند يجب أن تحددها الاختبارات والكود الفعلي» — منهجية سليمة). تحقق كل فجوة:

| فجوة الورقة | هل فجوة فعلية؟ | موقعها في خطة ARDP المعتمدة |
| :--- | :---: | :--- |
| §3.1 معايرة الـscorer | ✅ فعلية (الأوزان نقاط بداية) | مجدولة أصلًا: §11.1 — معايرة على بيانات G3 لـ24 ساعة؛ **إضافة الورقة القيّمة:** dataset اصطناعي عبر FabricRunner قبل G3 |
| §3.2 محور الفقد | ✅ موصوفة بدقة (استبعاد مقصود RT-1) | مطابقة لسجل العيوب ووصفة التمديد §5.1 في مرجع القياس |
| §3.3 staleness/freshness | ✅ **أدقّ فجوة جديدة** — تنسيق `GTPRP1` لا يحمل أي حقل زمن، فتقرير قديم غير قابل للكشف حاليًا | عامل recency موجود في صيغة B-9 المعتمدة (quality×count×recency) لكن التطبيق الحالي يحوي عامل العدد فقط؛ **الحل المعماري المتوافق: توسيع التقرير إلى GTPRP2 بحقل زمن (رسالة تطبيق، لا تغيير سلكي) عند G3** |
| §4.1 hysteresis | ⚠️ راجع C.3-1 | متطلب HOLD عند التبديل = B-4 FSM (G4)؛ موجود حاليًا `CLEAR_WINNER_MARGIN = 5%` عند المُختار |
| §4.2 anti-flapping | ✅ فعلية | B-4 `FlapSuppressor` + معاملات §11.2 لكل صنف (G4/G5) |
| §4.3 معايرة الثقة | ✅ فعلية (عامل العدد فقط اليوم) | B-9 كاملًا (جودة×عدد×حداثة) مجدول G3 |
| §4.4 recovery hysteresis | ✅ فعلية | B-11 + B-6 `RevertGuard` (G5) |
| §6 الحالات F1–F4 | 🟡 مغطاة جزئيًا (اختبارات الاتجاه + الجولات الحية)؛ F5/F6 (عزل spike) جديدة ومقبولة | A-1/B-5 |
| §7.1 كلفة قناة التقارير | ✅ فعلية جديدة (أثبتنا التعافي حيًّا؛ الكلفة لم تُقس) | إضافة قيّمة تُدمج في D-3 |
| §7.2/§8 ضغط الطوابير المستدام | 🟡 منطق RT-2 مثبت؛ مصفوفة الضغط المستدامة فعلية جديدة | توسعة RT-2 |
| §9 الأداء | 🟡 benchmarks المحرك موجودة (criterion + stress)؛ قياس routing-overhead المخصص فعلية جديدة | توسعة D-3 |
| §10 soak | 🟡 24 ساعة **هي معيار خروج G3 أصلًا**؛ إقتراح 1h/6h إضافة تشغيلية مقبولة | G3 |
| §11 churn | ✅ فعلية وترتبط بعيوب مؤجلة **معروفة** (CORE-4 تسريب مهمة TX، CORE-9 نمو خريطة limiter) | سجل المؤجلات في Closure-Matrix |
| §12 failure injection | ✅ | D-4 (معتمدة v1.1 بمصفوفة سيناريو→بوابة)؛ مخطط artifact إضافة جيدة |
| §13 الساعات | 🟡 **محقق جزئيًا بالفعل**: كل الزمن الداخلي monotonic (`MonotonicTime`)، أمان الالتفاف u32 وحد الانزياح ≤1.5ms مثبتان باختبارات، والمقدّر offset-free لا يحتاج مزامنة؛ المتبقي: اختبارات الحواف (suspend/resume…) | G1 مسجل + إضافات P1 |
| §14 التزامن | ✅ فعلية (طبقة runtime: mutex لكل اتصال) | جديدة |
| §15 توسيع الحتمية | ✅ فعلية (الإثبات الحالي حالة واحدة) | توسعة D-2 |
| §16 replay | ✅ فعلية | إضافة تخدم B-10/RE-10 |
| §17 سجل القرارات | ✅ فعلية (الشريحة الحالية = scored records فقط) | B-10 كاملًا (G3) |
| §18 دورة حياة المسار | ✅ **مطابقة حرفيًا لـB-11** (نفس الحالات الست) | B-11 (G3/G5) |
| §19 سياسات لكل صنف | ✅ | B-12 |
| §20 الإعدادات | ✅ | A-7 (G3) |
| §21/§22 shadow/kill-switch | ✅ | B-2 + INV-15 (قاعدة P4: فحص سالب كل مرحلة) |
| §23 أمان ما قبل G4 | ✅ | معايير خروج G4 + B-6 |
| §27 السيناريوهات S01–S12 | ✅ إضافة قيّمة | معرفات ملموسة لكتالوج D-4 |
| §29/§30 harness وbaseline | ✅ | D-3 + امتداد المقارنة الثلاثية v1.1 |
| §33 الترتيب و§34 DoD | ✅ متسقة مع ترتيب ARDP | G3→G4 |

### C.3 التصحيحات الدقيقة المطلوبة عند قراءة الورقة

1. **§4.1 near-tie (دقة تصنيف):** السلوك الحالي للمُختار عند فارق ضئيل (0.910 مقابل 0.911) هو **`TIE_BREAK_LOWER_ID`** — أي اختيار حتمي بأدنى معرف، وليس `HOLD` كما يتوقع §4.1. هذا سليم ومقصود في وضع shadow (قرار قابل للتفسير بلا تذبذب عشوائي)، ومثبت باختبار `ties_break_to_the_lower_id`. متطلب «HOLD عند الفروق الضئيلة» صحيح لكنه **من متطلبات وقت التبديل** التي يوفرها B-4 FSM (جولات تأكيد + hold) عند G4 — فيجب قراءة §4.1 كمطلب G4 لا كوصف للسلوك الحالي.
2. **§13 (جزئية):** تفضيل monotonic semantics **محقق داخليًا منذ البداية** — كل العمليات الزمنية المحلية تعمل بـ`MonotonicTime`، والطابع السلكي مشتق منه، مع اختبارات الالتفاف والانزياح المثبتة. المتبقي فعليًا هو اختبارات الحواف عبر الأجهزة فقط.
3. **Gates A–E (§25):** يجب اعتمادها **كتوسيع تفصيلي لمعايير ARDP وليست مصنّفًا موازيًا**: Gate A+B+D ≈ معايير خروج G3 (سلامة القياس + سلامة المُختار + إثبات فصل الظل)، Gate C ≈ G3-soak/مقدمة G4، Gate E ≈ G4 نفسها. يُمنع إنتاج نظام بوابات منافس؛ المرجع الحاكم يبقى `docs/ADAPTIVE-ROUTING-DEVELOPMENT-PLAN.md` §6 وسجل §12.
4. **البنود المُجدولة أصلًا** (B-4/B-9/B-10/B-11/B-12/D-3/D-4/A-7/G3-24h/معايير G4) تُقرأ هنا كتأكيد وإضافة تفصيلية اختبارية فوق الخطة، لا كاكتشافات جديدة — وهذا استخدام صحيح ومفيد.

### C.4 التوافق مع بنية البروتوكول والمعمارية — متوافقة ✅

- **النواة النقية**: كل مقترحات الورقة تحافظ على فصل gtp-route النقي عن tokio/gtp-core (المحولات في gtp-route-tokio عند B-2).
- **لا تغيير سلكي**: توسيع freshness يتم عبر `GTPRP2` كرسالة تطبيق ReliableOrdered — مطابق لقاعدة §2.3.
- **الثوابت المحترمة**: INV-3 (قياس بعد المصادقة فقط)، INV-11 (نطاق واحد لكل PathStats)، INV-15 (ظل بلا تنفيذ)، INV-18 (لا حالة غير محدودة — درس RT-2).
- **أساس العينات لكل-حزمة** محفوظ في كل مقترحات القياس الجديدة.

### C.5 الحكم النهائي

**الورقة دقيقة وموثوقة**: كل ادعاءاتها الواقعية مؤكدة بالأدلة، وكل فجواتها حقيقية (مع كون بعضها مجدولًا أصلًا في خطة ARDP)، ولا تناقض معماريًا بنية البروتوكول. **تُعتمد كملحق تحقق تفصيلي (Validation Supplement) لخطة ARDP للبوابات G3/G4**، وأبرز إسهامين جديدين فيها: (1) كشف غياب حقل الزمن في `MeasurementReport` كفجوة freshness تشغيلية يجب سدها عند G3 عبر GTPRP2، و(2) مصفوفة السيناريوهات المضادة S01–S12 كمادة ملموسة لكتالوج D-4. تلتزم الورقة بقاعدة §12 (تُحدَّد حالة كل بند بالكود والاختبارات، لا بالنص).
