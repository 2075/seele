#!/usr/bin/env bash
# Outage drill: briefly break parts of research-stack and watch seele's severity (needs seele running).
#   egress-open stopped → expect "degraded" (amber)
#   edge stopped        → expect "outage"   (slow red breathing everywhere)
# Every container is started again at the end, even on Ctrl-C.
set -uo pipefail
P=research-stack
watch_for() {  # seconds label
  for i in $(seq "$1"); do
    curl -s -m2 127.0.0.1:6743/state | python3 -c "import json,sys;d=json.load(sys.stdin);s=d['signals'];print(f\"  [$2 +${i}s] severity={d.get('severity')!s:9} gw={s.get('gateway_down',0):.2f} bad={s.get('containers_bad',0):.2f} health={d['health']}\")" 2>/dev/null || echo "  [$2 +${i}s] seele unreachable"
    sleep 1
  done
}
restore() { docker start $P-egress-open-1 $P-edge-1 >/dev/null 2>&1; echo "restored"; }
trap restore EXIT
echo "== baseline";                 watch_for 4 base
echo "== stop egress-open";          docker stop $P-egress-open-1 >/dev/null; watch_for 12 egress-down
echo "== start egress-open";         docker start $P-egress-open-1 >/dev/null; watch_for 12 egress-up
echo "== stop edge (gateway off)";   docker stop $P-edge-1 >/dev/null; watch_for 12 edge-down
echo "== start edge";                docker start $P-edge-1 >/dev/null; watch_for 15 edge-up
