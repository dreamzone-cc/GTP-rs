#!/usr/bin/env bash
# Deploys the LATEST repository HEAD to the verification VPS and restarts the
# two long-running servers with their established configuration:
#   UDP 7777 — anonymous server (tools/simulation compatibility)
#   UDP 7778 — identified v1.3 server (fixed test identity seed; the public
#              key it prints is the one clients pin — see
#              docs/WAN_BASELINE_VERIFICATION_AR.md)
#
# Usage: scripts/deploy_vps.sh [commit-ish]   (default: HEAD)
#
# The binary is stamped with the git short SHA (GTP_BUILD_SHA), so the last
# verification step compares local and remote stamps and fails loudly on drift.
set -euo pipefail

VPS_USER="${VPS_USER:-ubuntu}"
VPS_HOST="${VPS_HOST:-92.222.80.200}"
VPS_PASS="${VPS_PASS:?set VPS_PASS or export it before running}"
REPO="$(git rev-parse --show-toplevel)"
REF="${1:-HEAD}"
SHA="$(git -C "$REPO" rev-parse --short "$REF")"

echo "==> building release for $SHA"
cd "$REPO"
PKG_VER="$(sed -n '/\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1 | tr -d ' "')"
GTP_BUILD_SHA="gtp-${PKG_VER:-0.2.0}-$SHA" ~/.rustup/toolchains/1.85.0-x86_64-unknown-linux-gnu/bin/cargo \
  build --release -p gtp-cli --offline

echo "==> local stamp: $(./target/release/gtp-cli --version)"

echo "==> uploading to $VPS_USER@$VPS_HOST"
sshpass -p "$VPS_PASS" scp -o StrictHostKeyChecking=accept-new \
  ./target/release/gtp-cli "$VPS_USER@$VPS_HOST":~/gtp/gtp-cli.new

echo "==> restarting servers (config preserved; remote block runs as root via sudo -S)"
printf '%s\n' "$VPS_PASS" | sshpass -p "$VPS_PASS" ssh -o StrictHostKeyChecking=accept-new "$VPS_USER@$VPS_HOST" 'sudo -S bash -s' <<'REMOTE'
  set -e
  ME=$$
  for sig in TERM KILL; do
    # exclude our own shell: pgrep -f would otherwise match the script text
    pids=$(pgrep -f "gtp/gtp-cli net-server" | grep -v "^${ME}$" || true)
    [ -z "$pids" ] && break
    kill -$sig $pids 2>/dev/null || true
    sleep 1
  done
  chmod +x /home/ubuntu/gtp/gtp-cli.new
  mv /home/ubuntu/gtp/gtp-cli.new /home/ubuntu/gtp/gtp-cli
  chown ubuntu:ubuntu /home/ubuntu/gtp/gtp-cli
  for port_log in "7777 server.log" "7778 server-anchored.log"; do
    set -- $port_log
    port=$1; log=$2
    if [ "$port" = "7778" ]; then
      extra="--identity-seed a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00"
    else
      extra=""
    fi
    su ubuntu -c "nohup /home/ubuntu/gtp/gtp-cli net-server --bind 0.0.0.0:${port} ${extra} > /home/ubuntu/gtp/${log} 2>&1 &"
  done
  sleep 1
  ss -ulnp | grep -E ":7777|:7778"
  echo REMOTE_OK
REMOTE

echo "==> verifying version sync"
REMOTE_STAMP="$(sshpass -p "$VPS_PASS" ssh "$VPS_USER@$VPS_HOST" '~/gtp/gtp-cli --version')"
LOCAL_STAMP="$(./target/release/gtp-cli --version)"
echo "local : $LOCAL_STAMP"
echo "remote: $REMOTE_STAMP"
if [ "$LOCAL_STAMP" != "$REMOTE_STAMP" ]; then
  echo "✗ VERSION DRIFT — remote is not running $SHA" >&2
  exit 1
fi
echo "✓ VPS in sync at $LOCAL_STAMP"
