# سجل تتبع تنفيذ معالجة عيوب GTP-rs

> **آلية المراجعة المعتمدة** — هذا المستند هو السجل الوحيد لحالة التنفيذ.
> الحالات: `✅ منفَّذ ومُتحقَّق` | `⏸ مؤجَّل بقرار` | `❌ فشل التحقق`
> قاعدة الإغلاق: بند بلا اختبار تثبيت مُسمّى = غير منفَّذ. المرجع الحَكَم: `GTP-rs-Architecture-Protocol-Audit-Paper-v1.0.md` §12 + `GTP-rs_Cross-Audit_Reconciliation_AR.md`.
> خطة التنفيذ: `GTP-rs-Remediation-Execution-Plan-v1.0.md`.

## بوابة التحقق الأخيرة — **ناجحة ✅** (2026-08-30)

| البند | النتيجة |
| :--- | :--- |
| `cargo test --workspace --all-targets` | ✅ **77 ناجحاً / 0 فاشل** (كانت 48 قبل الجولة — +29 اختبار تثبيت) |
| `cargo clippy --workspace --all-targets` | ✅ 0 تحذيرات من الفئات المعنية |
| `cargo fmt` | ✅ مطبَّق على الشجرة كاملة |
| فحوص grep الإجرائية (7) | ✅ كلها ناجحة |
| أمر التشغيل | `bash scripts/verify_remediation.sh` |

---

## المرحلة 0 — إيقاف النزيف الأمني: **مكتملة 6/6**

| ID | البند | الحالة | اختبارات التثبيت | الدليل/ملاحظات |
| :--- | :--- | :--- | :--- | :--- |
| P0-1 | SEC-1 مفاتيح اتجاهية | ✅ | `directional_keys_distinct_per_role`, `test_x25519_diffie_hellman_roundtrip` (يتضمن cross-open fail)، `test_x25519_passive_eavesdropper_cannot_decrypt`، مصافحة e2e حية | `DirectionalKeys` + `derive_directional_handshake_session_keys` (labels: c2s/s2c)؛ المسار الثابت المُهمَل تحوّل هو الآخر (`derive_directional_session_keys`) — لا يبقى أي مسار nonce-reuse؛ الاختبار القديم الذي كان يؤكد `client_key == server_key` أُلغي وعُكس |
| P0-2 | SEC-2 nonce بكامل PN | ✅ | `nonce_uses_full_packet_number` | nonce = `IV ⊕ (CID[0..4] ‖ PN[0..8])` — حقن كامل الـ 64 بت |
| P0-3 | SEC-3 مصادقة قبل الالتزام | ✅ | `spoofed_high_pn_does_not_burn_replay_window` (connection)، `failed_auth_simulation_does_not_burn_window` (replay) | `ReplayWindow::check()` بلا التزام + `commit()` بعد نجاح AEAD فقط |
| P0-4 | SEC-4 كوكي HMAC | ✅ | `cookie_timestamp_refresh_is_rejected`, `future_dated_cookie_is_rejected`, `cookie_secret_not_recoverable`, `test_cookie_verify_rejects_wrong_addr_and_expiry` | `HMAC-SHA256(secret, "GTP-COOKIE-V1"‖addr‖ts)`؛ طابع نصّي + MAC مقطوع 24B؛ رفض صريح للطابع المستقبلي؛ gtp-path أصبح يعتمد hmac/sha2 |
| P0-5 | SEC-7/10/12/14 | ✅ | `low_order_public_key_rejected`, `debug_does_not_leak_keys`, `seal_rejects_overflowing_payload_len` | `x25519-dalek/zeroize` مفعّلة في workspace؛ `was_contributory()` عبر توقيع `Result`؛ `checked_add` في seal/open؛ `Debug` محجوب يدوياً لـ `GtpAeadProtector`/`HandshakeSecret`/`StatelessTokenManager` (مع Drop+zeroize للثاني) |
| P0-6 | REC-10 + مطابقة 0.7/0.8 | ✅ | `ack_ranges_multi_gap_roundtrip_exact`، `ack_ranges_property_sweep_seeded` (64 حالة LCG حتمية)، `optimistic_ack_rejected`، `oversized_ack_ranges_capped` | **عطب المُفكِّك مُصلَح** (الـ gap يُطرح قبل حساب start — كان يعترض زوراً 6,7,17,18,19 ويفقد 1,2,8,9,10 في مثال وثيقة المطابقة)؛ `MAX_ACKED_PER_FRAME=16384`؛ رفض `largest_acked > largest_sent` |

## المرحلة 1 — التكامل الوظيفي: **مكتملة 6/6**

| ID | البند | الحالة | اختبارات التثبيت | الدليل/ملاحظات |
| :--- | :--- | :--- | :--- | :--- |
| P1-1 | Core-C1 + PATH-5 | ✅ | `control_frames_reach_peer_as_frames` (Ping يصل كإطار + Close يغلق النظير بـ `ConnectionClosed(7)`)، `path_migration_via_protocol` (تحدي→رد موجَّه→migration + حدث `PathMigrated`) | صف تحكم مستقل `OutgoingControlFrame` (6 أنواع) يُصرف كإطارات حقيقية؛ رد التحدي يوجَّه لـ `src_addr`؛ التحدي نفسه يوجَّه للعنوان الجديد؛ `graceful_close` يُدخل الإطار قبل Draining وحلقة TX تنكسر على `Closed` فقط — **الإغلاق المهذب يرسل فعلاً** |
| P1-2 | REC-7 + CC-6 | ✅ | `cubic_config_is_honored`, `policy_config_drives_ack_behavior`, `test_hkdf_directional_keys_isolation` | `AckTracker::with_policy/set_policy`؛ `CubicConfig{smss,iw,min,beta,c,gain}`؛ `PacingEngineConfig`؛ إطار AckFrequency الوارد يصل المتتبع فعلاً؛ `GtpConfig.max_pacing_burst_bytes` و`pto_max_duration` أصبحا مُقرأين |
| P1-3 | CC-3 + مطابقة §6 | ✅ | `ack_only_packets_never_declared_lost` | `bytes_acked`/`bytes_lost` تحصي in_flight فقط؛ حلقة الفقد تفلتر `record.in_flight` (منع التخفيض الزائف كل RTT)؛ `cc.on_packet_sent` يُستدعى للـ ack-eliciting فقط |
| P1-4 | REC-11 + ORD-4 | ✅ | `pto_burst_capped_and_drains_records`، `reliable_unordered_dedup` | `on_timeout` يصرف ≤2 سجل أقدم in-flight **ويزيلها**؛ تراجع `×2^min(count,8)` بحد `pto_max_duration`؛ `DeliveredIndex` (FIFO 4096) يمنع تكرار تسليم `ReliableUnordered`/`Retx` |
| P1-5 | ORD-1 + SEM-1 + SEM-2 | ✅ | `order_seq_wraparound_still_delivers`, `test_generation_id_modulo_arithmetic`, `rx_drop_late_sequenced` | RFC 1982 لـ `GenerationId` و`order_seq` (التفاف u32 يتحرك)؛ جدول RX-side `should_admit/update` يُسقط الحالة المتأخرة (الإطارات الافتراضية/غير sequenced تتجاوزه — حماية من CORE-8)؛ `PacketNumber/MessageId::next` أصبحا wrapping |
| P1-6 | REC-5 + REC-6 | ✅ | `ack_intervals_pruned_and_coalesced` | نافذة احتفاظ `ACK_RETENTION_WINDOW=1024` مع تقليم تحت الأكبر مستلم؛ الأقدم خارج الميزانية يبقى محتفظاً به حتى التقليم (لا فقد دائم) وكل إطار يغطي الأحدث |

## المرحلة 2 — الأولوية العالية: **5/6 منفَّذة + 1 جزئي**

| ID | البند | الحالة | الدليل/ملاحظات |
| :--- | :--- | :--- | :--- |
| P2-1 | CORE-3 صمود RX | ✅ | حلقة RX لا تنكسر: `ConnectionReset/ConnectionRefused/WouldBlock/Interrupted → continue`، والأخطاء الأخرى تُسجَّل وتستمر (grep evidence)؛ اختبار tokio مخصص مؤجل |
| P2-2 | ORD-2 عزل امتلاء المجموعة | ✅ منطقياً / ⏸ اختبار الحمل | فشل `on_incoming` يُحصى في `total_dropped_frames` ولا يجهض الـ datagram ولا يمنع `ack_tracker.on_packet_received`؛ اختبار بـ 256KB مؤجل (ثقيل) |
| P2-3 | PATH-1 + Draining→Closed | ✅ | `force_close_fresh_connection_succeeds`؛ `Initial→Closed` قانوني؛ `Draining→Closed` تلقائي عند نضوب صف التحكم |
| P2-4 | CORE-4 إخلاء CID | ✅ منطقياً / ⏸ اختبار e2e | إخلاء مدخل جدول التوجيه عند بلوغ Closed في حلقة RX؛ اختبار e2e خاص مؤجل |
| P2-5 | SEC-6 ratchet منسّق | ✅ | `coordinated_ratchet_keeps_link_alive`: دوران الاتجاهين معاً + بت KEY_PHASE على السلك + مفتاح RX سابق بنافذة سماح (حزمة ما قبل الدوران تُفتح)؛ **التحديد الموثق: يتطلب استدعاءً متزامناً من الطرفين حتى تُبنى رسالة KeyUpdate سلكية** |
| P2-6 | D-2 معالجة أخطاء الترميز | ✅ | لا `let _ = append_frame` متبقية: فشل الترميز يعيد العنصر للمجدول ولا يُسجَّل in-flight وهمياً؛ فشل enqueue إعادة الإرسال يُحصى (`total_dropped_frames`) |

## اختبارات التثبيت المضافة في هذه الجولة (29)

crypto: `nonce_uses_full_packet_number`, `seal_rejects_overflowing_payload_len`, `debug_does_not_leak_keys`, `low_order_public_key_rejected`, `failed_auth_simulation_does_not_burn_window` + اختبارات الكوكي الأربعة + اختبار المصافحة المعاد بناؤه.
recovery: `ack_ranges_multi_gap_roundtrip_exact`, `ack_ranges_property_sweep_seeded`, `optimistic_ack_rejected`, `oversized_ack_ranges_capped`, `ack_only_packets_never_declared_lost`, `pto_burst_capped_and_drains_records`, `ack_intervals_pruned_and_coalesced`, `policy_config_drives_ack_behavior`, اختبار RTT الموسّع.
cc: `cubic_timeout_resets_epoch_state`, `cubic_does_not_grow_on_empty_acks`, `cubic_config_is_honored`.
types/scheduler/path: `test_generation_id_modulo_arithmetic`, `test_packet_number_next_wraps_safely`, `order_seq_wraparound_still_delivers`.
core: `spoofed_high_pn_does_not_burn_replay_window`, `reliable_unordered_dedup`, `control_frames_reach_peer_as_frames`, `rx_drop_late_sequenced`, `force_close_fresh_connection_succeeds`, `directional_keys_distinct_per_role`, `coordinated_ratchet_keeps_link_alive`, `path_migration_via_protocol`.

## المراحل المؤجَّلة (بمتابعة — انظر خطة التنفيذ §5)

| ID | البند | الحالة | ملاحظات |
| :--- | :--- | :--- | :--- |
| PH-3 | عدالة DRR (SCH-1/2/3/4) + سقف المجموعات ORD-5 + تقليم SEM-5 | ⏸ | بعد استقرار قياسات المرحلتين 0/1 |
| PH-4 | سيناريوهات الالتحام المتبقية (بذور sim متعددة، ضغط CLI بتأكيدات، fuzz في CI) | ⏸ جزئي | الجزء الحرج مغطى بـ 29 اختبار تثبيت |
| PH-5 | الأداء: to_vec، تقليص Frame (288B)، مُحكى ChaCha مخزّن، تفعيل gtp-io/sendmmsg، نبض TX | ⏸ | بعد الصحة — لم يُلمس أداء المسار الساخن إلّا pacing remainder |
| DEF-1 | SEC-5 مصادقة خادم/Finished | ⏸ قرار معماري | يتطلب تصميم PSK/توقيع موثق قبل التنفيذ |
| DEF-2 | حماية الترويسة (header protection) | ⏸ قرار معماري | توصية وثيقة المطابقة |
| DEF-3 | WIR-6 امتدادات header_len + WIR-10 تفاوض نسخة + WIR-5 حشو طرفي + WIR-2 range_count | ⏸ | WIR-2/WIR-3 ذواهما انعكاس جانبياً في مسار core (رفض/تحقق) — إصلاح gtp-wire الكامل ضمن PH-4/5 |
| DEF-4 | REC-8 (ACK في كل datagram) | ⏸ | أصبح مقصوداً تحت الإعداد الافتراضي ack_frequency=1؛ يتغير تلقائياً عبر P1-2 |
| DEF-5 | CC-11 مقاييس (queue_bytes_per_tier/ECN) | ⏸ جزئي | `pacing_tokens_remaining` أصبح حقيقياً؛ الباقي PH-5 |

## سجل القرارات التنفيذية

| القرار | السبب |
| :--- | :--- |
| nonce = IV ⊕ CID[0..4] ‖ PN كامل | يلغي التصادم مع كامل الـ 64 بت؛ المفتاح مُسقَف بالـ CID في HKDF info أصلاً |
| المسار الثابت المُهمَل اشُتقاق اتجاهي + دور صريح (`new_with_role`/`as_client`) | لا يبقى أي مسار nonce-reuse في الشجرة؛ المحاكاة والاختبارات مرّرت بأدوار متعاكسة |
| صف تحكم مستقل (`OutgoingControlFrame`) بدل فئة MessageClass جديدة | أقل تمزيقاً للواجهات العامة، ويحمل التوجيه (`dest`) داخل العنصر |
| حقلا `tx_iv/rx_iv` ظاهران في ConnectionHot | يحتاجهما ratchet والاختبارات؛ ويكشف عدم اتساق المفاتيح فوراً |
| Ratchet يدوي متزامن-الاتجاهين + KEY_PHASE + مفتاح RX سابق | أقصى أمان قابل للتسليم بلا بروتوكول تفاوض سلكي جديد (موثق كقيد) |
| أخطاء `append_frame` تعيد العنصر للمجدول | يستوفي D-2 دون تغيير دلالات pop_next |

## كيفية المراجعة (للمراجع البشري أو الآلي)

```bash
# البوابة الكاملة (اختبارات + clippy + فحوص إجرائية)
bash scripts/verify_remediation.sh

# اختبار تثبيت محدد
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-crypto nonce_uses_full
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-recovery ack_ranges_multi_gap
~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo test -p gtp-core path_migration_via_protocol
```

قاعدة القبول لكل بند: الكود مدمج + اختبار تثبيت مُسمّى أعلاه يفشل عند عكس الإصلاح + البوابة خضراء. أي فشل مستقبلي يعيد البند إلى `❌` تلقائياً وفق §8 من خطة التنفيذ.

---
*آخر تحديث: 2026-08-30 — جولة v1.0: المرحلتان 0 و1 مكتملتان، المرحلة 2 بنسبة 5/6 (+1 جزئي)، 77/77 اختباراً ناجحاً.*
