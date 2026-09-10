# Getting started with lami

lami keeps one or more Arch machines described in one git repository, and can
move changes in both directions: from the repo onto a machine, and from a
machine back into the repo.

This guide takes about fifteen minutes and never writes to your system until
you explicitly ask it to.

---

## 1. The idea, in one page

Everything lami manages is a **resource**: a package, a file, or a service.

Resources are grouped into **layers**. A layer is one directory with one file:

```
layers/gui/
  layer.kdl       what this layer installs, writes and enables
  files/          file contents it copies
```

A **host** picks which layers it gets, and sets parameters:

```
hosts/
  frodo.kdl       gpu "intel",  layers "core" "gui"
  sam.kdl         gpu "nvidia", layers "core" "gui" "dev"
```

The same layer runs on both machines. What differs is the parameters, and the
layer says what to do about them.

Five commands do everything:

| Command | What it does | Writes anything? |
|---|---|---|
| `lami show` | this host's resolved profile | no |
| `lami diff` | what differs from the config | no |
| `lami apply` | bring the machine in line | **yes** |
| `lami capture` | pull a change back into the repo | **yes, to the repo** |
| `lami prune` | remove what is no longer declared | **yes**, and asks twice |

---

## 2. Look around without changing anything

Everything in this section is read-only.

```sh
lami list        # which hosts and layers exist
lami show        # what this machine resolves to
lami diff        # what differs right now
```

`lami why` answers the question every config eventually raises — *why is this
here?*

```sh
lami why firefox
```

```
firefox  (package)
  declared:   layers/gui/layer.kdl:12
  applies:    layer 'gui' is in frodo's layers list
```

It works for files too, and tells you where their permissions came from:

```sh
lami why /etc/sudoers.d/10-wheel
```

```
  permissions: 440 root:root  (a sudoers drop-in, which sudo requires to be 0440)
```

---

## 3. See the files before they exist

```sh
lami render --list                 # every managed path, and its mode
lami render /etc/hostname          # one file, exactly as it would be written
lami render --out ./preview        # a whole tree you can diff yourself
```

The `--out` tree mirrors absolute paths, so:

```sh
lami render --out ./preview
diff -ru ./preview/etc /etc 2>/dev/null | less
```

---

## 4. Write a layer

A layer file reads top to bottom. Nothing is required except a description.

```kdl
// layers/tools/layer.kdl
description "Command line tools"
needs "core"

packages {
    ripgrep
    fd
    bat      // a comment here survives everything lami does to this file
}

services {
    bluetooth
}
```

Package names go in one list whether they come from a repository or the AUR —
lami works that out from pacman's database, so you do not have to track it.
`lami check` confirms it and tells you if an AUR helper is missing.

### Conditions

Machines differ. Say so where it belongs, in the layer:

```kdl
when gpu="intel" {
    packages { intel-media-driver; vulkan-intel }
}

when gpu="nvidia" {
    packages { nvidia-open; nvidia-utils }
}

when class="laptop" {
    packages { brightnessctl }
}
```

`gpu`, `class` and the rest are whatever you put in the host file. Switches are
written `on` / `off`:

```kdl
// hosts/frodo.kdl
gpu   "intel"
class "desktop"
ddc   on        // external monitor brightness over DDC/CI
```

`off` is worth writing even though leaving the line out would also disable it,
because it documents itself: a missing line does not say whether you decided
against it or simply forgot.

### Files

Two ways, and the choice matters later:

```kdl
// From a file in the layer directory. Can be pulled back with `capture`.
file "/etc/pacman.conf" from="files/pacman.conf"

// Inline, and templated. Cannot be pulled back -- rendering is not reversible.
file "/etc/makepkg.conf.d/99-local.conf" {
    text """
    MAKEFLAGS="-j{{ cpu_threads }}"
    """
}
```

Use `from=` for anything you might tweak in place on a machine, and inline
`text` for anything genuinely generated. Triple-quoted strings dedent
automatically, so the indentation keeping your config readable never reaches
the file.

Ownership and mode come from the path, so you only write down exceptions:

| Path | Default |
|---|---|
| `/etc/sudoers.d/**` | `root:root 0440` |
| `/usr/local/{bin,sbin}/**` | `root:root 0755` |
| anything else under `/` | `root:root 0644` |
| `~/.ssh/**`, `~/.gnupg/**` | you, `0600` |
| `~/.local/bin/**` | you, `0755` |
| anything else under `~` | you, `0644` |

When no rule could guess, say it outright — and note that modes are **strings**,
because a bare `0640` would be read as a decimal number:

```kdl
file "/etc/snapper/configs/root" from="files/snapper-root" mode="0640"
```

### Hooks

Some files need something to happen after they change:

```kdl
on-change "/etc/mkinitcpio.conf" {
    run "mkinitcpio -P"
}
```

It runs only when that file actually changes.

---

## 5. Apply

Always look first:

```sh
lami apply --dry-run
```

Then:

```sh
sudo lami apply
```

In order: packages, files, services, hooks. Root is needed because it writes to
`/etc` — but only for `apply`; every read-only command runs unprivileged.

**`apply` never removes anything.** A typo'd layer name cannot uninstall your
desktop.

---

## 6. Capture: the other direction

You changed something on the machine and want it in the repo.

```sh
lami capture                                   # what is on offer
lami capture --file /etc/pacman.conf --dry-run
lami capture --file /etc/pacman.conf
lami capture --package cowsay --layer tools
```

This edits the config **you** wrote, so it never reformats your file or eats a
comment. Check with `git diff` before committing — adding a package should be
exactly one added line.

Only `from=` files can be captured. An inline `text` block may contain
`{{ ... }}`, and there is no way to tell which part of the result came from a
template, so lami says so instead of guessing.

---

## 7. Prune: removing things

`apply` never removes, so this is separate — and deliberately awkward:

```sh
lami prune                # shows what is stale; this is the default
sudo lami prune --force   # actually remove, after confirming
```

Only things a previous `apply` recorded as managed are candidates. Something
you installed by hand is never offered, because lami never claimed it.

Services are disabled, never stopped.

---

## 8. Secrets

Name an encrypted source `*.age` and it is decrypted on the way out:

```kdl
file "~/.ssh/config" from="files/ssh-config.age"
```

Point lami at your key once, in `config.kdl` at the root of your config
directory:

```kdl
age {
    identity  "~/.config/lami/identity.txt"
    recipient "age1..."
}
```

A decrypted file is always written `0600`, wherever it lands.

Getting the first key onto a new machine is the one step nothing can automate —
a YubiKey, a password manager, or a USB stick.

---

## 9. A second machine

1. Write `hosts/<name>.kdl` with its layers and parameters.
2. Get the repo onto the machine and point `~/.config/lami` at it.
3. `lami diff` to see what it would do.
4. `sudo lami apply`.

If the hostname is not in `hosts/`, lami stops with an error rather than
guessing. There is no default profile: a typo'd hostname or a fresh VM must not
quietly receive somebody else's configuration.

---

## Everyday loop

```sh
lami diff          # what drifted
lami capture       # pull it back
git add -A && git commit && git push

# on the other machine
git pull
lami diff
sudo lami apply
```
