#!/usr/bin/env bash
# Build and install lami to /usr/local/bin.
#
# Kept as a script rather than `cargo install` because /usr/local/bin needs
# root, and because a stale binary there once caused real confusion: capture
# and prune existed in the source but not in the installed copy.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"

cargo build --release
sudo install -Dm755 target/release/lami /usr/local/bin/lami

echo
echo "installed: $(command -v lami)"
lami --version
echo "commands:  $(lami --help | sed -n '/Commands:/,/^$/p' | awk 'NR>1 && NF {printf "%s ", $1}')"
