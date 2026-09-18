# ملحق المواصفة GTP v1.1 → v1.2: ServerFinish وربط النسخة

> حالة: مسودة تنفيذية (المرحلة 3.1 من خطة البنود المفتوحة)
> يعدّل: `GTP_1_1_Comprehensive_Technical_Specification.md` §56 (Handshake)

## 1. الدافع

المصافحة الحالية (ClientHello → ServerHello → HandshakeFinish) غير متماثلة: العميل
يُثبت حيازته السرّ المشترك (`client_proof`) بينما الخادم لا يُثبت شيئاً للعميل، ورقم
النسخة في ClientHello لا يدخل في أي حساب. النتيجة: لا يمكن اكتشاف تخفيض النسخة أو
العبث بالرسائل غير المفتاحية، ولا يستطيع العميل التمييز بين خادم قبل الجلسة فعلاً
وبين حزمة ServerHello معاد بثّها.

## 2. التغييرات على السلك

### 2.1 إطار جديد: `ServerFinish` (نوع 0x0E)

```text
0x0E || server_proof[32]
```

- يُرسَل حصراً من الخادم، داخل حزمة **طويلة الترويسة**، بعد التحقق الناجح من
  `HandshakeFinish` (الكوكي + برهان العميل).
- يُعاد إرساله عند وصول نسخة معاد بثّها من `HandshakeFinish` نفس المادة (idempotent)،
  طالما اتصال الـCID حياً — دون إعادة تسجيل تعيد تهيئة حالة الاتصال.

### 2.2 تدفق المصافحة v1.2

```text
Client                          Server
  ------ ClientHello ---------->   (version = 0x00000002)
                                   version ≠ 2 → تجاهل صامت (رفض قاطع للنظراء v1.1)
  <---------- ServerHello ------
  ------ HandshakeFinish ------>   (cookie_echo + client_proof)
  <---------- ServerFinish -----   (server_proof)   ← الجديد
  [تأسيس مؤكد عند العميل فقط بعد تحقق البرهان]
```

### 2.3 ربط النسخة في النص (Transcript)

`HandshakeTranscript` يمتد بحقل `version: u32`:
- العميل يربط النسخة التي **أرسلها**؛ الخادم يربط النسخة التي **فكّها** من ClientHello.
- أي عبث بقيمة النسخة في العبور → عدم تطابق البرهانين → فشل مصنّف
  (`HandshakeFailed("transcript mismatch")`).

## 3. الاشتقاق

```text
confirmation_key = HKDF-SHA256(ikm = X25519_shared,
                               salt = "GTP_V1_1_CONFIRM_SALT",
                               info = "gtp/v1 handshake finished key")
server_proof     = HMAC-SHA256(confirmation_key,
                               "gtp-handshake-server-finish"
                               ‖ client_pk ‖ server_pk ‖ client_nonce ‖ server_nonce
                               ‖ cid_be8 ‖ version_be4)
```

(مطابق لـ`client_proof` عدا الملصق — نفس الدوال المنجزة في `gtp-crypto::handshake`.)

## 4. حدود الأمان المحدثة (صريحة)

- **يضيفه هذا الملحق**: حماية تخفيض النسخة؛ حماية انعكاس/إعادة بث برهان الخادم
  (ملصق مميز + نص كامل)؛ تأكيد قبول الخادم قبل "Established" عند العميل.
- **لا يضيفه**: مقاومة MITM كاملة. مع X25519 مجهولة الهوية، مهاجم وسيط مشارك شرعي
  في كل ساق ويملك سرّها، فيستطيع إنتاج server_proof صحيح لساقه. إغلاق هذه الفجوة
  يتطلب **مرساة ثقة** (PSK أو شهادات/مفاتيح معروفة سلفاً) تدخل في `ikm` — بند
  موصفي مستقل مقترح v1.3.
- التوافق: v1.2 يرفض v1.1 قاطعاً (تجاهل hello بلا إجابة) — لا نافذة ازدواج.

## 5. اختبارات القبول

1. ذهاب/إياب ترميز `ServerFinish`.
2. عبث بنسخة النص → فشل البرهان (وحدات crypto).
3. e2e: تدفق رباعي كامل، العميل "مؤكد" فقط بعد ServerFinish متحقق.
4. إعادة بث HandshakeFinish → إعادة ServerFinish دون إعادة تسجيل.
5. ServerHello بلا ServerFinish تالٍ → connect() يفشل بمهلة مصنّفة (وليس Established صامتاً).
