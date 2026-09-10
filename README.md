# lami

Layered, declarative system configuration for Arch Linux — packages, `/etc`,
systemd units and dotfiles through **one** tool and **one** config.

> **Status: early development.** Packages, `/etc` files, services and hooks
> are managed end to end: `diff`, `apply`, `capture`, `prune`. Dotfiles are
> next.

## Why

If you keep several Arch machines alike, today you have to glue together three
tools: one for packages, one for `/etc`, one for dotfiles. Each has its own
config language, its own notion of "what changed", and none of them has a good
answer to the question that matters most in practice: **how do you pull back a
change you made live on the machine?**

lami puts all of it behind one model. Everything is a **resource** — a package,
a file, a service — and every resource has the same lifecycle: resolve, diff,
apply, **capture**.

## The config

One layer is one directory with one file, readable top to bottom:

```kdl
// layers/gui/layer.kdl
description "A working Hyprland desktop"
needs "core"

packages {
    hyprland
    greetd
    firefox     // needed for work SSO, do not swap for chromium
}

services {
    greetd
    power-profiles-daemon    // hard dependency of caelestia-shell
}

when gpu="nvidia" {
    packages { nvidia-open; nvidia-utils; egl-wayland }
}
```

**AUR packages have no separate block.** They live in the same `packages` list;
lami reads pacman's sync database to tell where a package comes from, so you do
not have to keep track of it while writing config.

A host says which layers it gets, and with what parameters:

```kdl
// hosts/frodo.kdl
description "Desktop"

layers "core" "tools" "gui" "rice"

gpu   "intel"
ucode "intel"
class "desktop"
ddc   on            // external monitor brightness over DDC/CI
```

The same `gui` layer runs on an Intel iGPU and on an RTX 5080 — only the `gpu`
parameter differs.

Switches are spelled `on` / `off`. `off` is worth having even though omitting
the line would also disable the feature, because **it documents itself**: a
missing line does not tell you whether the choice was considered or forgotten.

## Files

A layer can declare files, either taken verbatim from the layer directory or
written inline. Inline content is a Jinja2 template — the same shape an HTML
templating engine uses — with every host parameter available as a variable:

```kdl
file "/etc/pacman.conf" from="files/pacman.conf"

file "/etc/makepkg.conf.d/99-local.conf" {
    text """
    MAKEFLAGS="-j{{ cpu_threads }}"
    """
}
```

KDL's triple-quoted strings dedent automatically, so the indentation that keeps
the config readable never reaches the rendered file. Rendered files always end
with a newline.

### Permissions

Ownership and mode are inferred from the path, so only the exceptions are
written down:

| Path | Default |
|---|---|
| `/etc/sudoers.d/**` | `root:root 0440` — sudo silently ignores anything else |
| `/usr/local/bin/**`, `/usr/local/sbin/**` | `root:root 0755` |
| anything else under `/` | `root:root 0644` |
| `~/.ssh/**`, `~/.gnupg/**` | user, `0600` |
| `~/.local/bin/**` | user, `0755` — scripts on PATH are meant to run |
| anything else under `~` | user, `0644` |

The rules are deliberately few; inference you cannot recite from memory is
worse than none, because you end up looking it up anyway. When a path rule
cannot guess, say so outright:

```kdl
file "/etc/snapper/configs/root" from="files/snapper-root" mode="0640"
```

Modes are written as **strings**: a bare `0640` would be read as decimal.

`lami why` always prints which rule applied, so it never has to be guessed.

### Hooks

Some files are not enough on their own — the initramfs has to be regenerated
after its config changes:

```kdl
on-change "/etc/mkinitcpio.conf" {
    run "mkinitcpio -P"
}
```

A hook only runs when one of the files it watches actually changes.

Use `cpu_threads`, not `cpu-threads`: parameter names reach templates verbatim,
and a hyphen would be read as subtraction.

## Rendering

Nothing is installed — you just look at what *would* be:

```sh
lami render                    # every managed file, with headers
lami render --list             # just the paths, and where they are declared
lami render /etc/hostname      # one file, raw and pipeable
lami render --out ./preview    # a directory tree mirroring the target paths
```

The `--out` tree mirrors absolute paths, so it can be compared with the live
system using your own tools:

```sh
lami render --out ./preview
diff -ru ./preview/etc /etc 2>/dev/null | less
```

## Provenance

Every resource can say where it came from and why this host gets it:

```
$ lami why nvidia-open
nvidia-open  (package)
  declared:   layers/gui/layer.kdl:24
  applies:    layer 'gui' is in sam's layers list
  condition:  gpu=nvidia (this host: gpu = nvidia)
```

## Applying

```sh
sudo lami apply --dry-run    # show the plan and stop
sudo lami apply              # install, write, enable, run hooks
```

In that order: a service cannot be enabled before its package exists, and a
hook exists to react to a file that has just changed.

**`apply` never removes anything.** A typo'd layer name or a half-finished
config must not be able to uninstall your desktop. Removal is a separate
command with its own confirmation.

Root is required, because it writes to `/etc` and enables units — but only for
`apply`. Every read-only command deliberately runs unprivileged.

Packages available from a repository are installed by pacman as root. AUR
packages are installed by dropping to the invoking user, because every AUR
helper refuses to run as root — rightly, since it builds untrusted PKGBUILDs.
The user, home directory and group all come from passwd rather than the
environment: under `sudo`, `$HOME` may still be the caller's, and the build
cache would land in the wrong place owned by the wrong user.

Files are written atomically: a temporary file in the **target's own
directory**, then a rename over the target. The directory matters — on a stock
Arch btrfs layout `@` and `@home` are separate filesystems as far as
`rename(2)` is concerned, so staging in `/tmp` and renaming into `$HOME` fails
with `EXDEV` on most Arch installs.

Ownership and mode are set on the file descriptor, never on a path: this is a
root process writing into directories an unprivileged user controls, and
between a `stat` and an `open` a path can be swapped for a symlink to
`/etc/shadow`. Ownership is set before mode, since `chown` clears the setuid
and setgid bits.

## Capturing

The other direction: something changed on the machine and should end up in the
repo.

```sh
lami capture                                       # what could be captured
lami capture --package cowsay --layer tools --dry-run
lami capture --package cowsay --layer tools
```

This edits the config **you** hand-wrote, which is why the format has to
round-trip: capture must not reformat your file or eat the comment explaining
why a package is there. Verified against a real config — adding one package
produced exactly one added line and zero deleted ones, with every end-of-line
comment intact.

## Secrets

A source file whose name ends in `.age` is decrypted on the way out:

```kdl
file "~/.ssh/config" from="files/ssh-config.age"
```

That is the whole interface. There is no attribute to remember, so there is no
way to commit a secret in the clear by forgetting one.

Point lami at your key in `config.kdl` at the root of the config directory:

```kdl
age {
    identity  "~/.config/lami/identity.txt"
    recipient "age1..."          // may be repeated
}
```

`age` is called as a program rather than linked as a library — the same
reasoning as pacman, and it means a YubiKey works through
`age-plugin-yubikey` without lami knowing anything about smartcards.

**A decrypted file is never world-readable.** Whatever the path rules would
have said, content that was kept encrypted at rest is written `0600`; the
encryption is the statement that it is secret, so the mode follows from that
rather than from where it happens to land. An explicit `mode=` still wins, for
the daemon that has to read its own secret as another user.

Getting the first key onto a new machine is the one step that cannot be
automated — by definition. lami says so plainly rather than failing obscurely.

## Pruning

`apply` never removes anything, so removal is a separate command:

```sh
lami prune              # show what is stale; this is the default
sudo lami prune --force # actually remove, after confirming
```

Only things a **previous `apply` recorded as managed** are ever candidates. A
package you installed by hand is never offered, because lami never claimed it —
that distinction is the entire reason there is a state file at
`/var/lib/lami/state.json`.

Removal has to be asked for twice: once by choosing the command, once by saying
`--force`. A tool that deletes because of a mistyped subcommand is not one to
trust with root.

Services are **disabled, not stopped**. Stopping a display manager out from
under a running session because a layer was edited would be indefensible.

## Diffing

`lami diff` compares the declared state against the machine and changes
nothing:

```
$ lami diff
host: frodo

  ~ file     /etc/pacman.conf  [core]
  + package  ripgrep  [tools]
  + service  greetd.service  [gui]  (now: disabled)

3 change(s). Nothing has been applied.
```

`--undeclared` additionally lists explicitly installed packages that no layer
declares. `apply` never removes those — that is what `prune` is for, and
`capture` will offer to file them into a layer.

Packages are compared against `pacman -Qqe`, not `pacman -Qq`, on purpose: a
package present only as a dependency counts as missing, because an orphan sweep
will take it away as soon as whatever pulled it in disappears.

## Checking

```
$ lami check
packages:
  from repos   29
  not in repos 3

'paru' will fetch these from the AUR:
  caelestia-shell
  ...
```

Without an AUR helper this exits with an error before anything is installed,
and tells you how to fix it.

## Guide

[docs/guide.md](docs/guide.md) walks through it in about fifteen minutes,
without writing anything until you ask.

## Trying it out

```sh
./install.sh            # build and install to /usr/local/bin

# or without installing:
cargo build
./target/debug/lami --config-dir examples/minimal list
./target/debug/lami --config-dir examples/minimal --host frodo show
./target/debug/lami --config-dir examples/minimal --host frodo check
./target/debug/lami --config-dir examples/minimal --host sam why nvidia-open
```

The config directory defaults to `$XDG_CONFIG_HOME/lami`; `--config-dir` or
`LAMI_CONFIG_DIR` overrides it.

## Design principles

- **The config comes first.** Permissions follow from the path, the `.service`
  suffix is inferred, short content lives inline, conditions read as sentences.
- **Arch only, on purpose.** pacman, the AUR, systemd and mkinitcpio are built
  in rather than hidden behind an abstraction.
- **Call, don't link.** lami does not link libalpm: when pacman changes its ABI,
  the tool you need to repair the system should not be the casualty.
- **`apply` never removes.** Removal is a separate command with its own
  confirmation.

## License

MIT
