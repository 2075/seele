#!/usr/bin/env bash
# Pause the seele service, light each configured zone white in turn, then restart seele.
set -euo pipefail
cd "$(dirname "$0")/.."
SUDO=""; [ "$(id -u)" -eq 0 ] || SUDO="sudo"
$SUDO systemctl stop seele.service
trap '$SUDO systemctl start seele.service; echo "seele restarted"' EXIT
.venv/bin/seele identify --hold "${HOLD:-5}"
