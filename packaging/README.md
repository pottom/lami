# Packaging

`PKGBUILD` builds a release tag from GitHub.

```sh
cd packaging
makepkg -si
```

## Why `depends=()`

lami calls pacman, systemctl and age as programs rather than linking them, so
the package has no shared library dependency at all:

```
$ ldd /usr/bin/lami
    libgcc_s.so.1, libm.so.6, libc.so.6
```

That is deliberate. paru links libalpm, and when pacman 7.1 moved to libalpm 16
in December 2025 paru would not compile for five weeks -- leaving people
without the tool they would use to fix it. A configuration manager should not
be able to break that way.

## Releasing

```sh
# bump the version in Cargo.toml and PKGBUILD, then
git tag -a v0.1.0 -m "..."
git push --tags
cd packaging && makepkg -g          # real checksums instead of SKIP
```
