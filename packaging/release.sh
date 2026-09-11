#!/usr/bin/env bash
# Cut a release: version bump, tag, checksum, .SRCINFO.
#
#   ./release.sh 0.4.0
#
# Refuses a dirty tree or a failing test suite. A tag is the one thing here
# that cannot be taken back.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ver="${1:-}"
[[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "usage: $0 <major.minor.patch>" >&2; exit 1; }

git diff --quiet && git diff --cached --quiet || {
    echo "working tree is dirty; commit first" >&2; exit 1
}
git rev-parse "v$ver" >/dev/null 2>&1 && { echo "v$ver already exists" >&2; exit 1; }

echo "== tests"
cargo test --quiet
cargo clippy --all-targets -- -D warnings
cargo fmt --check

echo "== version"
sed -i "s/^version = \".*\"$/version = \"$ver\"/" Cargo.toml
sed -i "s/^pkgver=.*$/pkgver=$ver/" packaging/PKGBUILD
cargo build --release >/dev/null        # refresh Cargo.lock
git add -A
git commit -qm "Release $ver"
git tag -a "v$ver" -m "lami $ver"

echo "== checksum"
# The tag has to be on GitHub before its tarball can be hashed.
echo "   push the tag, then press enter:  git push && git push --tags"
read -r _
cd packaging
rm -f "lami-$ver.tar.gz"
curl -fsSL -o "lami-$ver.tar.gz" \
    "https://github.com/pottom/lami/archive/refs/tags/v$ver.tar.gz"
sum="$(sha256sum "lami-$ver.tar.gz" | cut -d' ' -f1)"
sed -i "s/^sha256sums=(.*)$/sha256sums=('$sum')/" PKGBUILD
makepkg --printsrcinfo > .SRCINFO
rm -f "lami-$ver.tar.gz"

cd ..
git add packaging/PKGBUILD packaging/.SRCINFO
git commit -qm "packaging: $ver checksum"

echo
echo "done. next:"
echo "  git push"
echo "  packaging/aur.sh"
