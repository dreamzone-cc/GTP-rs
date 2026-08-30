#!/usr/bin/env bash
# بوابة التحقق الشاملة لمعالجة عيوب GTP-rs
# المرجع: GTP-rs-Remediation-Execution-Plan-v1.0.md §8
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# بيئة rustup معطلة محلياً (argv[0] مشوّه) — استخدم الأداة مباشرة أو تجاوزها بمتغير البيئة
CARGO_BIN="${CARGO_BIN:-$HOME/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo}"

cd "$ROOT"
FAILED=0

banner() { printf '\n\033[1;34m== %s ==\033[0m\n' "$1"; }

banner "1/4 fmt (تقرير)"
"$CARGO_BIN" fmt --all -- --check 2>&1 | head -20 || true

banner "2/4 clippy (تقرير — غير قاطع في هذه الجولة)"
"$CARGO_BIN" clippy --workspace --all-targets 2>&1 | tail -30 || true

banner "3/4 الاختبارات (بوابة قاطعة)"
if "$CARGO_BIN" test --workspace --all-targets 2>&1 | tee /tmp/gtp_test_output.log | grep -E "^test result"; then
    if grep -qE "test result: FAILED|error\[|^error:" /tmp/gtp_test_output.log; then
        echo "❌ اختبارات فاشلة"; FAILED=1
    else
        PASS=$(grep -oE "^test result: ok\. [0-9]+ passed" /tmp/gtp_test_output.log | grep -oE "[0-9]+" | awk '{s+=$1} END {print s}')
        echo "✅ الاختبارات ناجحة — إجمالي الناجحين: $PASS"
    fi
else
    echo "❌ فشل بناء/تشغيل الاختبارات"; FAILED=1
fi

banner "4/4 فحوص grep إجرائية"
check_absent() { # $1=pattern $2=scope $3=desc
    if grep -rn "$1" "$2" >/dev/null 2>&1; then
        echo "❌ موجود وما يجب ألا يكون: $3"; grep -rn "$1" "$2" | head -3; FAILED=1
    else
        echo "✅ $3"
    fi
}
check_present() { # $1=pattern $2=scope $3=desc
    if grep -rn "$1" "$2" >/dev/null 2>&1; then
        echo "✅ $3"
    else
        echo "❌ مفقود: $3"; FAILED=1
    fi
}

check_absent "assert_eq!(client_key, server_key)" "crates/gtp-crypto" "لا اختبار يؤكد تساوي مفاتيح الاتجاهين (SEC-1)"
check_present "assert_ne!" "crates/gtp-crypto/src" "اختبار عدم تساوي مفاتيح الاتجاهين موجود (SEC-1)"
check_present "GTP-COOKIE-V1" "crates/gtp-path/src" "الكوكي HMAC بالتسمية الجديدة (SEC-4)"
check_present "was_contributory" "crates/gtp-crypto/src" "فحص contributory لـ X25519 (SEC-12)"
check_present "checked_add" "crates/gtp-crypto/src" "فحوص أطوال محفوظة (SEC-10)"
check_present "in_flight" "crates/gtp-recovery/src/loss_detector.rs" "فلترة الفقد على in_flight (P1-3)"
check_present "ConnectionReset" "crates/gtp-runtime-tokio/src" "صمود RX أمام إعادة ضبط الاتصال (P2-1)"

printf '\n'
if [ "$FAILED" -eq 0 ]; then
    printf "\033[1;32mالبوابة: ناجحة ✅\033[0m\n"
    exit 0
else
    printf "\033[1;31mالبوابة: فاشلة ❌ — راجع البنود أعلاه ومستند التتبع\033[0m\n"
    exit 1
fi
