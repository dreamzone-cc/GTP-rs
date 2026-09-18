# GTP Protocol Engineering Audit — ocr (open-code-review) Custom Prompt & Usage Guide

> دليل تقني شامل لتسليط أداة [open-code-review](https://github.com/alibaba/open-code-review) (`ocr`)
> على مشروع GTP-rs **كتدقيق هندسي لبروتوكول اتصال شبكي** وليس مراجعة كود عامة.
> المستند مكتفي بذاته: كل ما يلزم للتثبيت والتشغيل والصيانة موجود هنا.
>
> - المشروع الهدف: `/home/ggonlinux/GTP` (Rust workspace، 14 crate في `crates/`)
> - إصدار الأداة المُختبر: `ocr` v1.12.5 (مثبت عالمياً عبر npm)
> - تاريخ الإعداد: 2026-09-18

---

## Table of Contents

1. [الفكرة الأساسية: نقطتا الحقن في محرك المراجعة](#1-الفكرة-الأساسية-نقطتا-الحقن-في-محرك-المراجعة)
2. [المتطلبات الأساسية والتحقق منها](#2-المتطلبات-الأساسية-والتحقق-منها)
3. [معمارية الإعداد المخصص (3 مكونات)](#3-معمارية-الإعداد-المخصص-3-مكونات)
4. [خطوات التثبيت](#4-خطوات-التثبيت)
5. [المكوّن 1: ملف القواعد rule.json (كامل جاهز)](#5-المكوّن-1-ملف-القواعد-rulejson-كامل-جاهز)
6. [المكوّن 2: ملف الخلفية audit-brief.md (كامل جاهز)](#6-المكوّن-2-ملف-الخلفية-audit-briefmd-كامل-جاهز)
7. [المكوّن 3: أوامر التشغيل (كل السيناريوهات)](#7-المكوّن-3-أوامر-التشغيل-كل-السيناريوهات)
8. [الاستدعاء من ZCode](#8-الاستدعاء-من-zcode)
9. [خريطة تغطية نقاط التدقيق (15 بنداً)](#9-خريطة-تغطية-نقاط-التدقيق-15-بنداً)
10. [مبادئ الصياغة المستخدمة (ولماذا تعمل)](#10-مبادئ-الصياغة-المستخدمة-ولماذا-تعمل)
11. [أفضل الممارسات التشغيلية](#11-أفضل-الممارسات-التشغيلية)
12. [استكشاف الأخطاء وإصلاحها](#12-استكشاف-الأخطاء-وإصلاحها)
13. [الصيانة والترقيات](#13-الصيانة-والترقيات)

---

## 1. الفكرة الأساسية: نقطتا الحقن في محرك المراجعة

كل ما يُمرَّر إلى `ocr` يصل إلى النموذج عبر موضعين محددين داخل قالب المراجعة الرئيسي
(`main_task_user.md` في محرك الأداة):

```
### Requirement Background (Optional)   ← هنا يذهب --background / --background-file
{{requirement_background}}

### Review Checklist                     ← هنا يذهب rule.json (أو القاعدة النظامية)
{{system_rule}}
```

قاعدة الصياغة الذهبية الناتجة عن ذلك:

| | `--background-file` (الخلفية) | `.opencodereview/rule.json` (القواعد) |
|---|---|---|
| **دورها** | *من أنت وماذا تراجع* — مهمة التدقيق، هوية البروتوكول، أين المواصفات | *ماذا تتحقق* — قائمة فحص إلزامية لكل ملف |
| **الطبيعة** | سياق سردي موجز | أوامر فحص حتمية قابلة للتنفيذ |
| **النطاق** | تُحقن في **كل** مجموعة مراجعة | تُطابَق **لكل ملف** حسب نمط المسار |
| **البقاء** | لكل تشغيلة (flag سطر الأوامر) | دائمة في المستودع وتُدار معه عبر git |

**آلية مطابقة القواعد** (مهمة للترتيب):

- طبقات الحل بالأولوية: `--rule` (custom) ← `<repo>/.opencodereview/rule.json` (project) ← `~/.opencodereview/rule.json` (global) ← القواعد النظامية المدمجة.
- داخل كل طبقة: **أول قاعدة يطابق نمط مسارها الملفَ هي التي تُطبَّق** — لذلك تُرتَّب القواعد من الأخصص إلى الأعم.
- `"merge_system_rule": true` تجعل قاعدتك تُدمَج **مع** القاعدة النظامية للغة (Rust) بدل استبدالها — القاعدة النظامية لـ Rust ممتازة وتغطي cancellation safety والأقفال عبر `.await` والـ `unsafe` وأخطاء `unwrap`، فالدمج يضيف الطبقة البروتوكولية فوقها.

---

## 2. المتطلبات الأساسية والتحقق منها

| المتطلب | أمر التحقق | الحالة المرجعية على هذا الجهاز |
|---|---|---|
| Git ≥ 2.41 | `git --version` | متوفر |
| أداة `ocr` مثبتة عالمياً | `ocr --version` | v1.12.5 عبر `npm i -g @alibaba-group/open-code-review` |
| مزوّد LLM مُعدّ وي_WORK | `ocr llm test` | z-ai-coding / glm-5.3 — ناجح |
| المستودع الهدف git repo | `git -C /home/ggonlinux/GTP status` | نعم |

> وضع بديل: **Delegation Mode** (`ocr delegate ...`) لا يحتاج مزوّد LLM خاص بالأداة —
> الوكيل المضيف (ZCode) ينفّذ المراجعة بنموذجه و ocr تقدّم الهندسة الحتمية فقط
> (اختيار الملفات وحلّ القواعد). انظر `skills/open-code-review-delegate` في توثيق الأداة.

---

## 3. معمارية الإعداد المخصص (3 مكونات)

```
/home/ggonlinux/GTP/
├── .opencodereview/
│   ├── rule.json          ← المكوّن 1: قواعد الفحص لكل crate (قائمة تدقيق هندسية)
│   └── audit-brief.md     ← المكوّن 2: موجز مهمة التدقيق + خرائط المواصفات
└── docs/
    └── OCR_PROTOCOL_AUDIT_GUIDE.md   ← هذا الدليل (المكوّن 3: التعليمات)
```

- **rule.json** يعيد تعريف "قائمة الفحص" لكل ملف حسب طبقته البروتوكولية
  (wire / core / recovery / path / runtime / cc / crypto / io / route / ...).
- **audit-brief.md** يعيد تعريف "مهمة النموذج": أنت مدقق بروتوكول شبكات،
  المدخلات معادية، المواصفات هنا — يقرأها بنفسه عبر أدوات البحث عند الحاجة.
- **أوامر التشغيل** تربط المكونين بالتشغيلة وتتحكم بالنطاق والجهد والمخرجات.

---

## 4. خطوات التثبيت

```bash
cd /home/ggonlinux/GTP

# 1) أنشئ مجلد الإعداد
mkdir -p .opencodereview

# 2) احفظ محتوى القسم 5 في:   .opencodereview/rule.json
# 3) احفظ محتوى القسم 6 في:   .opencodereview/audit-brief.md

# 4) تحقق من صلاة JSON ومطابقة القواعد (مجاني — بلا LLM):
ocr rules check crates/gtp-core/src/session.rs   # يتوقع: قاعدة gtp-core
ocr rules check crates/gtp-wire/src/frame.rs     # يتوقع: قاعدة gtp-wire
ocr rules check crates/gtp-cli/src/main.rs       # يتوقع: قاعدة supporting crates

# 5) معاينة نطاق المراجعة (مجاني — بلا LLM):
ocr review --preview
```

> ملاحظة: `.opencodereview/` يمكن إدراجه في git ليستفيد منه الفريق كله والـ CI.
> `.gitignore` الحالي لا يستثنيه، فسيظهر كملف جديد غير متتبَّع حتى تقرر.

---

## 5. المكوّن 1: ملف القواعد rule.json (كامل جاهز)

احفظ كما هو في `GTP/.opencodereview/rule.json`. **الترتيب مقصود من الأخصص إلى الأعم**
(أول قاعدة تطابق تفوز):

```json
{
  "rules": [
    {
      "path": "crates/gtp-core/**",
      "rule": "You are auditing the GTP protocol CORE (handshake, sessions, state machines). Treat this file as protocol-critical infrastructure, not application code. Enforce:\n\n#### Protocol State Machine\n- Every state transition must be triggered by a defined event (message received, timer fired, error); flag transitions reachable by undefined events or unreachable states.\n- Flag states that can be exited by multiple paths with inconsistent cleanup (session state torn down on one path but leaked on another).\n- Flag missing handling for out-of-order, duplicated, or stale messages in ANY state (a message valid in state A arriving during state B must be rejected or ignored by explicit design, never fall through).\n- Enum-based states must make illegal states unrepresentable; flag bool-flag combinations that encode hidden states.\n\n#### Handshake and Session Lifecycle\n- Verify handshake completes atomically: no path where a session is considered established while peer identity/crypto parameters are unverified.\n- Flag session IDs, nonces, or tokens generated without proper randomness or reused across sessions.\n- Every session creation path must have a matching teardown on ALL error paths (early returns, timeouts, cancelled futures).\n- Flag session state not bounded — unauthenticated peers must not be able to create unbounded session entries (DoS).\n\n#### Data Flow Correctness\n- Trace sender/receiver agreement: every message constructed here must be consumed symmetrically by the peer logic; flag write-side fields the read-side never validates.\n- Flag ordering assumptions not enforced (in-flight window, sequence handling, queue bounds).",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-wire/**",
      "rule": "You are auditing the GTP WIRE FORMAT layer (packet encoding/decoding). This is a network-facing parser: assume every input is hostile. Enforce:\n\n#### Packet Parsing Robustness\n- Every length field, offset, or count read from a packet MUST be bounds-checked against the remaining buffer BEFORE use; flag any slice indexing, sub-slice, or read relying on a packet-supplied length.\n- Flag integer parsing with truncating casts (u64->u32, i32->usize) or arithmetic that can overflow/wrap on attacker-controlled values.\n- Unknown versions, unknown message types, reserved flags, and malformed variants must be rejected with typed errors, never guessed or skipped silently.\n- Flag parse functions that partially mutate output state before failing (parser must fail atomically).\n\n#### Encode/Decode Symmetry\n- Every encode must round-trip through decode; flag asymmetries (fields serialized but never parsed, defaults assumed on decode that encode never writes).\n- Flag endianness assumptions not explicit (big/little) on multi-byte fields.\n\n#### Specification Compliance\n- Field orders, sizes, alignment, and padding must match the GTP specification (see GTP_1_1_Comprehensive_Technical_Specification.md); flag any deviation from documented frame layouts, even if tests pass.\n- Maximum packet/message sizes must be enforced on both encode and decode paths.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-recovery/**",
      "rule": "You are auditing the GTP FAILURE RECOVERY layer (timeouts, retries, reconnection, loss recovery). Enforce:\n\n#### Timeout and Retry Semantics\n- Every network operation must have an explicit timeout; flag unbounded waits on channels, sockets, or futures.\n- Retry loops MUST have: bounded attempts OR exponential backoff with jitter, cancellation propagation, and terminal failure distinction (retryable vs permanent error); flag infinite hot retry loops.\n- Flag timeout values without justification relative to RTT assumptions documented in the spec.\n\n#### Reconnection Correctness\n- Reconnection must re-validate peer identity and re-key (flag paths resuming a stale session as trusted).\n- Flag connection state shared between old and new connections where a delayed packet from the old connection corrupts the new one.\n- In-flight work at disconnect must be either cancelled with cleanup or explicitly re-queued; flag silently dropped work.\n\n#### Recovery State Integrity\n- Recovery procedures must not assume the failure model they are recovering from (e.g., recovery code itself doing unreliably-ordered operations); flag recovery paths that can themselves fail without recourse.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-path/**",
      "rule": "You are auditing the GTP PATH MANAGEMENT layer (multi-path, path migration, path health). Enforce:\n\n#### Path Migration\n- Migration must be atomic from the session's perspective: no window where a session is bound to zero paths or double-bound to old and new paths simultaneously.\n- In-flight data at migration must have defined ownership (follow old path, redirect, or drop+retransmit); flag data loss or duplication windows.\n- Old-path resources (sockets, buffers, timers) must be released on every migration branch, including failed migrations.\n- Path identity/health metrics used for migration decisions must not be racy or stale enough to cause flapping; flag migrations triggered by unverified single samples.\n\n#### Multi-Path Invariants\n- Send scheduling across paths must respect ordering constraints the protocol guarantees; flag reordering introduced by path switching.\n- Path failure detection and failover must be bounded in time (explicit heartbeats/timeouts, not passive detection only).",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-runtime-tokio/**",
      "rule": "You are auditing the GTP ASYNC RUNTIME layer (tokio integration). This is the concurrency substrate of the protocol. Enforce:\n\n#### Races and Deadlocks\n- Flag check-then-act sequences on shared session/connection state not protected by a single lock or atomic ordering.\n- Flag lock hierarchy violations: acquiring two locks in different orders on different paths (deadlock); flag any lock held across .await.\n- Flag select!/race! branches where losing branches' side effects or spawned work leak (cancellation safety of every branch).\n- Shared mutation via Arc<Mutex>/Rc<RefCell> must have a single clear owner pattern; flag scattered mutation points.\n\n#### Task Lifecycle\n- Every spawned task must have defined shutdown (JoinHandle observed, cancellation channel, or detached-by-design documented); flag fire-and-forget tasks owning resources.\n- Flag blocking calls (std::fs, std::net, thread::sleep, CPU-heavy loops) inside async contexts.\n- Channel senders/receivers must handle peer-closed errors; flag sends that ignore a dead receiver losing data.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-scheduler/**",
      "rule": "You are auditing the GTP SCHEDULER (packet scheduling across paths/timers). Enforce:\n\n- Scheduling decisions must be deterministic given the same inputs, or explicitly documented as timing-dependent; flag decisions reading unsynchronized time/clock state.\n- Timer wheels/intervals must handle: missed ticks, delayed firing, and cancellation without leaking entries.\n- Priority/fairness logic must be bounded — flag starvation scenarios where one flow/path class can starve others indefinitely.\n- Queue bounds must be enforced with defined overflow policy (drop-oldest, reject, backpressure); flag unbounded queues on network input.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-cc/**",
      "rule": "You are auditing the GTP CONGESTION CONTROL implementation. Enforce:\n\n- Every signal used (RTT samples, loss signals, ECN, ACK counts) must be validated for staleness and plausibility before driving cwnd changes; flag division by zero on smoothed RTT/interval and integer underflow on window shrink.\n- Mode transitions (slow start -> congestion avoidance -> recovery) must be complete: no state where multiple modes are simultaneously active or where recovery exit forgets inflated state.\n- Flag window arithmetic that can overflow, go negative via unsigned wrap, or exceed documented limits.\n- Pacing/bursting must be enforced as designed; flag bursts exceeding the spec's defined burst tolerance.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-crypto/**",
      "rule": "You are auditing the GTP CRYPTO layer. Enforce with zero tolerance:\n\n- Flag ANY hand-rolled cryptographic primitive, mode, or padding (use audited crates: ring/rustcrypto per project deps).\n- Nonces/IVs must be unique per key with a defined uniqueness strategy; flag random nonces with birthday-risk counters not enforced, or counter nonces without wrap handling.\n- Flag decryption/authentication order errors (decrypt-before-verify), missing AEAD tag checks, or MAC comparison not constant-time.\n- Key material: no logging, no Debug/Display on keys, zeroization on drop where the project's spec requires it, no keys derived from low-entropy inputs.\n- Handshake crypto: replay protection (challenge/transcript binding), downgrade prevention, and forward-secrecy requirements per the spec must be verifiable in code.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-io/**",
      "rule": "You are auditing the GTP I/O layer (sockets, buffers, epoll integration). Enforce:\n\n- Buffer management: every read/write path must handle partial reads/writes and WouldBlock explicitly; flag assume-full-read patterns.\n- Flag per-packet allocations in the hot path where a pool/reuse strategy is the documented design.\n- Resource cleanup: every socket/timer registration must be deregistered on all exit paths including panics and cancellation.\n- FD limits: flag unbounded socket/connection creation without backpressure or limits.",
      "merge_system_rule": true
    },
    {
      "path": "crates/gtp-route/**",
      "rule": "You are auditing the GTP ADAPTIVE ROUTING engine. Enforce:\n\n- Routing decisions must be based on validated, fresh metrics; flag decisions on stale/missing measurements silently defaulting to unsafe paths.\n- Route changes must propagate atomically to forwarding state; flag windows with inconsistent route/forwarding views.\n- Flag routing oscillation risks: hysteresis/threshold logic must prevent flip-flopping between routes on metric noise.\n- Fallback route must exist and be tested for every primary-route failure mode.",
      "merge_system_rule": true
    },
    {
      "path": "crates/{gtp-types,gtp,gtp-cli,gtp-sim}/**",
      "rule": "You are auditing GTP supporting crates (types, facade, CLI, simulator). Enforce:\n\n- gtp-types: protocol constants/limits must match the specification exactly (frame sizes, header lengths, version numbers); flag magic numbers duplicated instead of using these types.\n- gtp-sim: simulation MUST faithfully model the failure modes the protocol claims to handle (reordering, duplication, loss, partition); flag simulators that cannot produce these conditions — they give false confidence.\n- Tests: every protocol behavior changed or touched in this diff must have a test covering it, INCLUDING at minimum: malformed input, boundary values (empty/max), duplicate/out-of-order delivery, and cancellation mid-operation. Flag happy-path-only tests for protocol logic.",
      "merge_system_rule": true
    },
    {
      "path": "**/*.rs",
      "rule": "PROTOCOL AUDIT MODE — this codebase implements GTP, a networking protocol (gaming VPN with adaptive routing). Review it as a protocol engineering audit, not generic code review:\n\n- Verify changes against the protocol's documented contracts (specs live in repo root: GTP_1_1_Comprehensive_Technical_Specification.md and GTP-rs-Technical-Specification.md — read them via search tools when the diff touches protocol-visible behavior).\n- Every change to message handling, session logic, or timing alters externally-visible protocol behavior: flag changes that deviate from spec-defined behavior even when internally consistent.\n- Error handling: network input errors must be typed and distinguish malformed (peer bug/attack) from transient (retryable) from fatal.\n- Resources (memory, FDs, timers, tasks) must be bounded and released on all paths — a protocol server must survive adversarial peers.\n- Tests must cover edge cases: empty payloads, maximum sizes, zero-length fields, malformed truncations, and concurrent session operations.",
      "merge_system_rule": true
    }
  ],
  "exclude": ["**/*.zip", "target/**", "docs/**", "fixes/**", "GTP_All_Markdown_Files/**"]
}
```

**ملاحظات على الملف:**

1. `merge_system_rule: true` في كل قاعدة: تُدمَج قاعدة Rust النظامية الممتازة
   (تغطي الـ Error Handling وRace Conditions العامة وunsafe وunwrap) **مع** طبقتك البروتوكولية.
2. آخر قاعدة `**/*.rs` شبكة أمان لأي ملف Rust خارج الـ crates المسماة.
3. حقل `exclude` يستبعد الضغوط والهدف البنائي والمستندات من نطاق المراجعة — عدّله حسب الحاجة
   (أنماط بأسلوب gitignore).
4. ترتيب القواعد جزء من التصميم: لا تُعدِل ترتيب الأخصص-قبل-الأعم.

---

## 6. المكوّن 2: ملف الخلفية audit-brief.md (كامل جاهز)

احفظ كما هو في `GTP/.opencodereview/audit-brief.md`.

> ⚠️ **تحذير مهم**: الخلفية تُحقن في **كل** مجموعة مراجعة، وميزانية التوكنات للمطالبة
> (200k افتراضياً) مشتركة مع الـ diffs — **لا تمرر المواصفة الكاملة (98KB) كخلفية**.
> الصيغة الصحيحة: موجز تنفيذي + مؤشرات للمواصفات، والوكيل يقرأها بنفسه عبر أدوات البحث.

```markdown
# GTP Protocol Engineering Audit

## Mission
You are performing a NETWORKING PROTOCOL ENGINEERING AUDIT, not a generic
code review. GTP-rs is a Rust workspace (14 crates) implementing a gaming
VPN transport protocol with adaptive multi-path routing: reliable sessions
over unordered UDP-like paths, congestion control per path, path migration,
and crypto-protected handshakes.

## Authoritative references (read via your search tools when the diff
touches protocol-visible behavior)
- Wire format, frame layouts, limits: GTP_1_1_Comprehensive_Technical_Specification.md (repo root)
- Architecture and layering: GTP-rs-Technical-Specification.md
- Adaptive routing design: GTP_Adaptive_Routing_Technical_Paper.md

## Audit posture
- Peer inputs are hostile: every packet is attacker-crafted until validated.
- Protocol invariants (ordering, uniqueness, state legality) outrank local
  code elegance; a locally-clean change that breaks a protocol invariant is
  a critical finding.
- Severity guide: spec violation or memory/DoS-reachable flaw = critical;
  race/deadlock/leak on any path = high; fragile-but-correct = medium.
```

---

## 7. المكوّن 3: أوامر التشغيل (كل السيناريوهات)

جميع الأوامر تُنفَّذ من جذر المستودع: `cd /home/ggonlinux/GTP`

### 7.1 ما قبل التشغيل (مجاني — بلا LLM)

```bash
# التحقق من القاعدة المطبقة على ملف معين
ocr rules check crates/gtp-core/src/session.rs

# معاينة الملفات التي ستُراجع
ocr review --preview
```

### 7.2 التدقيق الكامل لتغييرات مساحة العمل (staged + unstaged + untracked)

```bash
ocr review --audience agent \
  --background-file .opencodereview/audit-brief.md \
  --effort high \
  --concurrency 1 \
  --output /tmp/gtp-audit.txt
```

> **سياسة الحساب المشترك على هذا الجهاز**: ZCode وOCR يتشاركان حساب Z.ai واحداً،
> ولذلك مرور OCR عبر بوابة محلية (`z-ai-gated` ← `127.0.0.1:8788`، خدمة `zai-gate`)
> تفرض `max_active_requests = 1` وتعطي أولوية جلسات ZCode التفاعلية، مع تراجع أُسي
> تلقائي عند 429/1302 واحترام `Retry-After`. **`--concurrency 1` إلزامي دائماً هنا**
> حتى لا تتراكم اتصالات منتظرة. الحالة: `curl -s http://127.0.0.1:8788/__gate/status`،
> والسجلات: `~/.zai-gate/gatekeeper.log` (بلا أي مفاتيح).

### 7.3 تدقيق نطاق فرع (merge-base ضد main)

```bash
ocr review --audience agent \
  --background-file .opencodereview/audit-brief.md \
  --from main --to feature-branch \
  --effort high \
  --output /tmp/gtp-audit.txt
```

### 7.4 تدقيق التزام واحد

```bash
ocr review --audience agent \
  --background-file .opencodereview/audit-brief.md \
  --commit abc123 \
  --output /tmp/gtp-audit.txt
```

### 7.5 تدقيق ملفات كاملة بلا diff (للتدقيق الاستقصائي لـ crate حساس)

```bash
ocr scan --path crates/gtp-wire \
  --background-file .opencodereview/audit-brief.md

ocr scan --path crates/gtp-core
```

### 7.6 استئناف مراجعة انقطعت

```bash
ocr session list                                        # اعثر على المعرف
ocr review --from main --to feature-branch \
  --resume <session-id>                                 # نفس الهدف الأصلي
```

### 7.7 قاعدة مؤقتة لتشغيلة واحدة (دون إنشاء rule.json)

```bash
ocr review --audience agent \
  --rule /path/to/some-rule.json \
  -b "GTP protocol audit: verify wire-format bounds checks and state machine legality"
```

### شرح الأعلام الأساسية

| العَلَم | الوظيفة |
|---|---|
| `--audience agent` | **إلزامي عند الاستدعاء الآلي/من وكيل** — ملخص نهائي فقط دون واجهة تقدم |
| `--background-file <md>` | الخلفية من ملف Markdown (أولوية أعلى من `--background`) |
| `-b / --background "text"` | خلفية قصيرة inline |
| `--effort low\|medium\|high` | الجهد: 1/2/3 جولات — استخدم high للتدقيق الهندسي |
| `--output <path>` | حفظ النتيجة كاملة في ملف (يمنع البتر) — اقرأه كاملاً |
| `--format text\|json\|sarif` | صيغة المخرجات (json آلي، sarif لتكامل مسح الكود) |
| `--preview` | معاينة النطاق دون LLM |
| `--exclude '<patterns>'` | استبعاد أنماط إضافية (تُدمج مع exclude في rule.json) |
| `--concurrency <n>` | تخفيض التوازي عند حدود المعدل (الافتراضي 8) |
| `--max-tokens-budget <n>` | سقف إجمالي للتوكنات في التشغيلة |
| `--timeout <min>` | مهلة كل مجموعة × عدد الجولات |
| `--resume <id>` | استئناف مراجعة نطاق/التزام انقطعت |
| `--repo <path>` | تشغيل من خارج المستودع |

---

## 8. الاستدعاء من ZCode

مهارتا ocr مثبتتان على نطاق المستخدم في `~/.zcode/skills/`
(`open-code-review` و `open-code-review-delegate`) — تعملان في أي جلسة جديدة
وفي أي مشروع. طلاقة الاستدعاء:

```text
"راجع تغييراتي في GTP كتدقيق بروتوكول — استخدم الخلفية .opencodereview/audit-brief.md"
"راجع هذا الفرع ضد main كتدقيق بروتوكول GTP بالجهد العالي واخرج النتيجة لملف"
"امسح crates/gtp-wire مسحاً كاملاً كتدقيق بروتوكول"
```

الوكيل سيبني الأمر الصحيح تلقائياً (`ocr review --audience agent --background-file ...`).
كما أن ملفات `rule.json` و `audit-brief.md` تُلتقط تلقائياً من `.opencodereview/`
لأنها في مواضع الحل الافتراضية — لا حاجة لتمرير `--rule`.

**وضع التوفير (Delegation)**: اطلب من ZCode استخدام مهارة `open-code-review-delegate`
لينفّذ المراجعة بنموذجه الخاص مع `ocr delegate preview` / `ocr delegate rule` —
بلا استهلاك حصة API منفصلة، مناسب للفحوص السريعة المتكررة.

---

## 9. خريطة تغطية نقاط التدقيق (15 بنداً)

| نقطة الفحص | أين تغطت |
|---|---|
| صحة تصميم وتنفيذ البروتوكول | قاعدة `**/*.rs` العامة + `gtp-core` + الخلفية |
| توافق التنفيذ مع المواصفات والوثائق | قاعدة `**/*.rs` (أمر قراءة المواصفات) + `gtp-wire` (Specification Compliance) |
| تحليل مسارات الاتصال وتدفق البيانات | `gtp-core` (Data Flow Correctness) + `gtp-route` |
| صحة Handshake وSession Management | `gtp-core` (Handshake and Session Lifecycle) |
| اكتشاف مشاكل State Machine | `gtp-core` (Protocol State Machine) |
| التحقق من معالجة الحزم والرسائل | `gtp-wire` (Packet Parsing Robustness + Encode/Decode Symmetry) |
| حالات الفشل وTimeout وRetry | `gtp-recovery` (Timeout and Retry Semantics) |
| فحص التزامن وAsync | `gtp-runtime-tokio` + قاعدة Rust النظامية المدمجة |
| Race Conditions وDeadlocks | `gtp-runtime-tokio` (Races and Deadlocks) |
| إدارة الاتصالات وإعادة الاتصال | `gtp-recovery` (Reconnection Correctness) |
| Path Migration | `gtp-path` (Path Migration + Multi-Path Invariants) |
| أخطاء الذاكرة والموارد | `gtp-io` + قاعدة `**/*.rs` (bounded/released) |
| الجوانب الأمنية ومقاومة سوء الاستخدام | `gtp-crypto` + `gtp-wire` (مدخلات معادية) + الخلفية |
| صحة Error Handling | قاعدة Rust النظامية المدمجة عبر `merge_system_rule` |
| الاختبارات والتغطية والحالات الحدية | `gtp-sim`/`gtp-types` (قسم Tests) + ذيل القاعدة العامة |
| (إضافة) ازدحام وتوجيه تكيفي | `gtp-cc` + `gtp-route` — جوهر هوية المشروع |

---

## 10. مبادئ الصياغة المستخدمة (ولماذا تعمل)

1. **إعادة تعريف الهوية في الموضعين**: الخلفية تقول "أنت مدقق بروتوكول شبكات"،
   وكل قاعدة تبدأ بـ "You are auditing the GTP ___ layer" — النموذج يتبنى منظور
   المدقق الهندسي لا المراجع العام.
2. **قابلية تنفيذ لا وصفية**: كل بند شرط قابل للفحص
   ("MUST be bounds-checked BEFORE use") وليس نصيحة فضفاضة.
3. **فرضية العدوانية**: "assume every input is hostile" — يحوّل فحص المتانة
   من بند إضافي إلى افتراض أساسي، وهذا جوهر تدقيق بروتوكولات الشبكات.
4. **التخصص المكاني**: كل crate يحصل فقط على فحوص طبقته الوظيفية —
   لا يُغرق فاحص الـ wire بتفاصيل الـ scheduler.
5. **ربط المواصفة بالكود عبر الخرائط لا اللصق**: المواصفة (98KB) أكبر من أن
   تُحقن؛ نمرر مؤشراتها ونأمر بالرجوع إليها عند المساس بسلوك مرئي للبروتوكول.
6. **سلم شدّة واضح** في الخلفية (spec violation = critical) يمنع تسطيح الخطورة.
7. **الدمج مع النظامي لا الاستبدال**: `merge_system_rule: true` يبقي قائمة Rust
   القوية (cancellation safety، الأقفال عبر await، unsafe) ويضيف فوقها.

---

## 11. أفضل الممارسات التشغيلية

- **ابدأ دائماً بـ `--preview`** لمعرفة النطاق قبل إنفاق توكنات.
- **استخدم `--output` دائماً** واقرأ الملف كاملاً — لا تمرر عبر `head`/`tail`
  (يسقط تعليقات الجزء الأول).
- **`--effort high` للتدقيق الجاد** (3 جولات، مهلة فعليّة = timeout × 3)،
  و`low` للفحص السريع أثناء التطوير.
- **المخرجات المهيكلة**: `--format json` للمعالجة الآلية، `--format sarif`
  لتكامل GitHub Code Scanning.
- **مراجعات كبيرة؟** ضع `--max-tokens-budget`؛ و`--concurrency 1` ثابتة على هذا
  الجهاز بموجب سياسة الحساب المشترك (البوابة تسلسل الطلبات على أي حال).
- **انقطعت مراجعة نطاق؟** `ocr session list` ثم `--resume <id>` —
  لا تُعِد من الصفر (استئناف وضع مساحة العمل غير مدعوم).
- **تصفح النتائج بصرياً**: `ocr viewer` يفتح واجهة ويب لسجل الجلسات.
- **لغة التعليقات**: تتبع إعداد `language` في إعداد ocr (الافتراضي English).

---

## 12. استكشاف الأخطاء وإصلاحها

| العَرَض | السبب والمعالجة |
|---|---|
| `ocr: command not found` | `npm install -g @alibaba-group/open-code-review` |
| فشل اتصال LLM | `ocr llm test` للتشخيص؛ ثم `ocr config provider` / `ocr config model` |
| قاعدة لا تُطبَّق على ملف | `ocr rules check <file>` — تحقق من نمط المسار وترتيب القواعد (الأخصص أولاً) وطبقة أعلى أولوية (custom ← project ← global) |
| `unknown flag: --output` | إصدار أقدم من 1.10.0 — `npm i -g @alibaba-group/open-code-review@latest` |
| استهلاك توكنات مرتفع | راجع حجم audit-brief.md، استخدم `--exclude`، وخفض `--effort` |
| حدود معدل المزوّد | `--concurrency 4` أو أقل |
| مراجعة "عامة" رغم القواعد | تأكد أن rule.json في `<repo>/.opencodereview/` وصيغة JSON سليمة (`python3 -m json.tool`) |

---

## 13. الصيانة والترقيات

- **ترقية الأداة**: `npm i -g @alibaba-group/open-code-review@latest`
  ثم `ocr --version` — ملفات `.opencodereview/` محلية للمستودع ولا تتأثر.
- **إضافة crate جديد إلى الـ workspace**: أضف قاعدة بنمط مساره **قبل** قاعدة
  `**/*.rs` العامة، مع `merge_system_rule: true`.
- **تغيير المواصفة**: حدّث مؤشرات المراجع في `audit-brief.md` إذا تغيرت أسماء
  ملفات المواصفات في جذر المستودع.
- **التدقيق المؤسسي لاحقاً**: المستودع الرسمي يوفر `action.yml` (GitHub Actions
  قابل لإعادة الاستخدام) وأمثلة جاهزة في `examples/` لـ GitLab وBitbucket
  وGerrit — أسرار التشغيل: `OCR_LLM_URL` و`OCR_LLM_AUTH_TOKEN`، ومتغيرات:
  `OCR_LLM_MODEL` و`OCR_LLM_USE_ANTHROPIC`. ملف `rule.json` نفسه يُستخدم كما هو في CI.

---

## مراجع

- المستودع الرسمي: <https://github.com/alibaba/open-code-review>
- التوثيق الكامل: <https://open-codereview.ai/docs>
- مرجع CLI الكامل: <https://open-codereview.ai/docs/cli-reference>
- قواعد المراجعة والتخصيص: <https://open-codereview.ai/docs/review-rules>
- وضع التفويض: <https://open-codereview.ai/docs/delegate>
- مواصفة GTP: `GTP_1_1_Comprehensive_Technical_Specification.md` (جذر المستودع)
