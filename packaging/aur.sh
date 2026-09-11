#!/usr/bin/env bash
# Copy PKGBUILD and .SRCINFO into the AUR clone and push.
#
#   ./aur.sh [path-to-aur-clone]     default ~/Projects/lami-aur
#
# The AUR repo holds nothing else: it is a recipe index, not a package host.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
aur="${1:-$HOME/Projects/lami-aur}"

[[ -d "$aur/.git" ]] || {
    cat >&2 <<EOF
No AUR clone at $aur.

  git clone ssh://aur@aur.archlinux.org/lami.git $aur

See README.md for the one-time account and SSH key setup.
EOF
    exit 1
}

# Generated from the PKGBUILD, so a mismatch means one was edited alone.
( cd "$here" && makepkg --printsrcinfo > .SRCINFO )

cp "$here/PKGBUILD" "$here/.SRCINFO" "$aur/"
cd "$aur"

ver="$(sed -n 's/^pkgver=//p' PKGBUILD)"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
    echo "nothing changed"
    exit 0
fi
git --no-pager diff --cached --stat
git commit -qm "lami $ver"
echo
echo "committed. review with 'git -C $aur show', then:"
echo "  git -C $aur push"
