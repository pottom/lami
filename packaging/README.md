# Packaging

`PKGBUILD` builds a release tag from GitHub, and is the same file that is
published to the AUR. `.SRCINFO` is generated from it and kept alongside so a
change to one without the other shows up in review.

```sh
cd packaging
makepkg -si
```

## What it depends on

`glibc` and `libgcc` — the C runtime, and nothing else:

```
$ pacman -Qi lami | grep Depends
Depends On      : glibc  libgcc
```

That is the point. lami calls pacman, systemctl, git and age as programs
rather than linking them, so no ABI bump in any of those can break it. paru
links libalpm, and when pacman 7.1 moved to libalpm 16 in December 2025 paru
would not compile for five weeks — leaving people without the tool they would
use to fix it. A configuration manager should not be able to break that way.

`paru`, `yay` and `age` are optional dependencies: an AUR helper is only
needed if a layer declares AUR packages, and `age` only for encrypted sources.
lami says plainly which one is missing rather than failing obscurely.

## Releasing

```sh
./release.sh 0.4.0      # version bump, tag, checksum, .SRCINFO
git push && git push --tags
./aur.sh                # copy into the AUR clone and push
```

`release.sh` refuses to run with a dirty tree or failing tests, because a tag
is the one thing that cannot be taken back.

## The AUR package

**Not published yet, and currently not possible.** As of September 2026 the
AUR has paused new account registration while it deals with a wave of
automated account creation:

> New account registration is temporarily closed. […] There's no manual
> registration queue, and we will not be able to respond to requests for new
> accounts during this time. — aur.archlinux.org, HTTP 503

There is nothing to retry and nothing to ask for; it is announced on
`aur-general` and the Arch news feed when it reopens. Everything on this side
is ready and waiting: the name `lami` is free (checked against both the AUR
and the official repositories), the recipe builds and lints clean, and
`aur.sh` needs only a clone to push into.

Nothing is blocked by it in the meantime — `base/bootstrap.sh` builds from
this directory, which is how every machine has been set up so far. The AUR
would save a `git clone` and a `makepkg`, no more.

Once it reopens, installing lami on any Arch machine is one line:

```sh
paru -S lami
```

Until then a fresh machine builds it from this directory, which is what
`base/bootstrap.sh` in the config repo does.

### One-time setup

The AUR needs an account and an SSH key, and only the maintainer can do this
part:

1. Register at <https://aur.archlinux.org/register> — closed as of September
   2026, see above.
2. Add a public key under *My Account → SSH Public Key*. A dedicated key is
   worth it — this one can only push packages:

   ```sh
   ssh-keygen -t ed25519 -f ~/.ssh/aur -C "aur"
   cat >> ~/.ssh/config <<'EOF'
   Host aur.archlinux.org
       User aur
       IdentityFile ~/.ssh/aur
       IdentitiesOnly yes
   EOF
   ```

3. Clone the (empty) package repo. The name has to be free — `lami` was, as of
   this writing:

   ```sh
   git clone ssh://aur@aur.archlinux.org/lami.git ~/Projects/lami-aur
   ```

After that `./aur.sh` does every subsequent release.

### What the AUR repo contains

Only `PKGBUILD` and `.SRCINFO`. No source, no build output — the AUR is a
recipe index, not a package host, and the tarball is fetched from GitHub at
build time with the checksum pinned here.

### Checking before publishing

```sh
makepkg -f                                    # builds and runs the test suite
namcap PKGBUILD                               # lints the recipe
namcap lami-*-x86_64.pkg.tar.zst              # lints what it produced
```

Both namcap runs should be silent. `makepkg` runs `cargo test` in `check()`,
so a release that does not pass its own tests cannot be built at all.
