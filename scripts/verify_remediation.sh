#!/usr/bin/env bash
# Comprehensive verification gate for the GTP-rs remediation effort.
# Reference: GTP-rs-Remediation-Execution-Plan-v1.0.md §8
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Resolve cargo in this order (the local rustup shims are broken — mangled
# argv[0] — so a plain `cargo` may not work):
#   1. explicit CARGO_BIN override,
#   2. the toolchain pinned by rust-toolchain.toml, installed under ~/.rustup,
#   3. whatever `cargo` resolves on PATH (if it actually runs),
#   4. the newest toolchain present under ~/.rustup.
PINNED="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$ROOT/rust-toolchain.toml" 2>/dev/null | tr -d '[:space:]')"
CARGO_BIN="${CARGO_BIN:-}"
if [ -z "$CARGO_BIN" ] && [ -n "$PINNED" ]; then
    candidate="$HOME/.rustup/toolchains/${PINNED}-x86_64-unknown-linux-gnu/bin/cargo"
    [ -x "$candidate" ] && CARGO_BIN="$candidate"
fi
if [ -z "$CARGO_BIN" ]; then
    if command -v cargo >/dev/null 2>&1 && cargo --version >/dev/null 2>&1; then
        CARGO_BIN="$(command -v cargo)"
    else
        CARGO_BIN="$(ls -1 "$HOME"/.rustup/toolchains/*/bin/cargo 2>/dev/null | sort -V | tail -1)"
    fi
fi
if [ -z "$CARGO_BIN" ]; then
    echo "❌ no usable cargo found (set CARGO_BIN explicitly)"; exit 1
fi
echo "using cargo: $CARGO_BIN ($("$CARGO_BIN" --version 2>/dev/null))"

cd "$ROOT"
FAILED=0

banner() { printf '\n\033[1;34m== %s ==\033[0m\n' "$1"; }

banner "1/4 fmt (report)"
"$CARGO_BIN" fmt --all -- --check 2>&1 | head -20 || true

banner "2/4 clippy (report — non-blocking this round)"
"$CARGO_BIN" clippy --workspace --all-targets 2>&1 | tail -30 || true

banner "3/4 tests (hard gate)"
if "$CARGO_BIN" test --workspace --all-targets 2>&1 | tee /tmp/gtp_test_output.log | grep -E "^test result"; then
    if grep -qE "test result: FAILED|error\[|^error:" /tmp/gtp_test_output.log; then
        echo "❌ failing tests"; FAILED=1
    else
        PASS=$(grep -oE "^test result: ok\. [0-9]+ passed" /tmp/gtp_test_output.log | grep -oE "[0-9]+" | awk '{s+=$1} END {print s}')
        echo "✅ tests green — total passed: $PASS"
    fi
else
    echo "❌ test build/run failed"; FAILED=1
fi

banner "4/4 procedural grep checks"
check_absent() { # $1=pattern $2=scope $3=desc
    if grep -rn "$1" "$2" >/dev/null 2>&1; then
        echo "❌ present but must be absent: $3"; grep -rn "$1" "$2" | head -3; FAILED=1
    else
        echo "✅ $3"
    fi
}
check_present() { # $1=pattern $2=scope $3=desc
    if grep -rn "$1" "$2" >/dev/null 2>&1; then
        echo "✅ $3"
    else
        echo "❌ missing: $3"; FAILED=1
    fi
}

check_absent "assert_eq!(client_key, server_key)" "crates/gtp-crypto" "no test asserts directional key equality (SEC-1)"
check_present "assert_ne!" "crates/gtp-crypto/src" "directional key inequality test exists (SEC-1)"
check_present "GTP-COOKIE-V1" "crates/gtp-path/src" "HMAC cookie with the new label (SEC-4)"
check_present "was_contributory" "crates/gtp-crypto/src" "X25519 contributory check (SEC-12)"
check_present "checked_add" "crates/gtp-crypto/src" "overflow-safe length checks (SEC-10)"
check_present "in_flight" "crates/gtp-recovery/src/loss_detector.rs" "loss filtering on in_flight (P1-3)"
check_present "ConnectionReset" "crates/gtp-runtime-tokio/src" "RX resilience against connection reset (P2-1)"
# INV-18 (G1): every header field carried on the wire has a real consumer
# outside its defining crate — no dead wire surface. The timestamp check is
# the G1 regression guard: it failed before 5ed2430 (the field was written
# at TX and never read at RX) and must never regress.
check_present "timestamp_micros" "crates/gtp-core/src/connection.rs" "wire timestamp consumed in the core RX path (INV-18, RE-1)"
check_present "owd.on_packet" "crates/gtp-core/src/connection.rs" "OwdEstimator wired into RX (INV-18, A-1)"
check_present "owd_var" "crates/gtp-core/src/control/metrics.rs" "one-way delay surfaced in telemetry (INV-18, A-2)"

printf '\n'
if [ "$FAILED" -eq 0 ]; then
    printf "\033[1;32mGATE: PASSED ✅\033[0m\n"
    exit 0
else
    printf "\033[1;31mGATE: FAILED ❌ — review the items above and the tracker document\033[0m\n"
    exit 1
fi
