#!/usr/bin/env bash
# Serve prepare.sh and the ssh key to the guest, which reaches the host at
# 10.0.2.2 under QEMU's user-mode networking.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
cp -f prepare.sh base-install.sh run/
cd run
exec python3 -m http.server "${HTTP_PORT:-8000}" --bind 0.0.0.0
