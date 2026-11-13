#!/usr/bin/env bash
# Install or refresh the OpenRGB server and seele as system services, then verify them.
# Works with or without sudo in front.
set -euo pipefail
cd "$(dirname "$0")/.."
SUDO=""; [ "$(id -u)" -eq 0 ] || SUDO="sudo"
OWNER="${SUDO_USER:-$USER}"

echo "==> stopping services"
$SUDO systemctl stop seele.service openrgb-server.service 2>/dev/null || true

echo "==> clearing stray OpenRGB servers on :6742"
$SUDO pkill -u "$OWNER" -x openrgb || true
for _ in $(seq 10); do ss -ltn 'sport = :6742' | grep -q LISTEN || break; sleep 1; done
if ss -ltn 'sport = :6742' | grep -q LISTEN; then echo "port 6742 still busy:"; ss -ltnp 'sport = :6742'; exit 1; fi

echo "==> installing units"
$SUDO install -m 644 deploy/openrgb-server.service deploy/seele.service /etc/systemd/system/
$SUDO systemctl daemon-reload
$SUDO systemctl reset-failed openrgb-server.service seele.service 2>/dev/null || true
$SUDO systemctl enable openrgb-server.service seele.service
$SUDO systemctl start openrgb-server.service seele.service

echo "==> verifying"
sleep 8
ok=1
grep -q -- '--server-host 127.0.0.1' /etc/systemd/system/openrgb-server.service || { echo "FAIL unit lacks localhost bind"; ok=0; }
ss -ltn 'sport = :6742' | grep -q '127.0.0.1:6742' || { echo "FAIL :6742 not bound to 127.0.0.1"; ss -ltn 'sport = :6742'; ok=0; }
for u in openrgb-server seele; do
  s=$(systemctl is-active $u.service || true); echo "$u: $s"; [ "$s" = active ] || ok=0
done
curl -s -m3 127.0.0.1:6743/state >/dev/null && echo "seele /state: ok" || { echo "FAIL seele /state unreachable"; ok=0; }
[ $ok = 1 ] && echo "ALL OK" || { echo "SOMETHING FAILED (see above)"; exit 1; }
