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
lami render ~/.config/fish/config.fish   # tilde or absolute, either works
lami render --layer rice --list    # only what one layer writes
lami render --out ./preview        # a whole tree you can diff yourself
```

The `--out` tree mirrors absolute paths, so:

```sh
lami render --out ./preview
diff -ru ./preview/etc /etc 2>/dev/null | less
```

`--layer` answers "what does this layer actually put on the machine?" without
reading the layer file and following every `from=` by hand. It is checked
against the layers *this host* enables, so asking about one it does not have
is an error rather than an empty list.

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

### Groups

```kdl
groups {
    libvirt     // manage VMs without a password prompt for every action
    wireshark   // capture without running the whole GUI as root
}
```

Whose groups: yours — the user running lami, which is the only actor it has.

Three things worth knowing:

- **lami never creates a group.** The package that needs one creates it, with
  the right gid. A declared group that does not exist is reported as a
  problem, not quietly invented.
- **A new membership takes effect at the next login.** The session that ran
  `apply` still does not have it. lami says so, because otherwise the first
  thing that fails afterwards looks like the step not having worked.
- **`prune` only removes what lami added.** A group you joined by hand never
  reached the state file, so it is never a candidate.

Which leaves the memberships nothing accounts for. `lami diff --undeclared`
lists those too, primary group excluded:

```
a member of, but declared by no layer:
  i2c
```

That one is real: it is left over from a manual step on the machine this was
written on, and the layer that would have wanted it says in a comment that the
membership is not needed at all.

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

Switches are written `on` / `off`:

```kdl
// hosts/frodo.kdl
gpu   "intel"
class "desktop"
ddc   on        // external monitor brightness over DDC/CI
```

`off` is worth writing even though leaving the line out would also disable it,
because it documents itself: a missing line does not say whether you decided
against it or simply forgot.

### Parameters: say what the layer needs

A layer declares what it expects the host to tell it. This is the contract
between the two files, and it is worth writing out because without it they can
only agree by convention:

```kdl
params {
    gpu   "which vendor driver to install"  one-of="intel amd nvidia"
    class "what kind of machine this is"    one-of="desktop laptop vm"
    ddc   "external monitor brightness over DDC/CI" default=off
    monitors "Hyprland monitor lines, one per output" list=#true
}
```

The description is not optional. It is what somebody reading the host file
sees when they ask what a line means:

```
$ lami show
parameters:
  class        desktop   core: what kind of machine this is
  cpu_threads  4         default, core: how many jobs makepkg may run
  gpu          intel     gui: which vendor driver to install
```

What it buys, all of it things that used to pass in silence:

- **A required parameter a host omits is an error.** No `default=` means no
  default: a machine that does not say which GPU it has stops, rather than
  installing the wrong driver.
- **`one-of` catches a typo** — and says what was allowed, quoting the line.
- **`when` may only test a declared parameter.** `when gpu="intle"` and
  `when hostname="frodo"` used to do exactly what a correct condition that
  happens not to hold does: nothing, quietly. Now the first is an error and
  the second works, because `hostname` is always available.
- **`list=#true` refuses a bare string.** KDL cannot tell one string from a
  list of one, so `{% for m in monitors %}` over the scalar form produced
  nothing at all.
- **`lami check` reports a parameter nothing reads.** Not an error — a template
  may read it, or a layer this host does not enable may declare it — but not
  invisible either.

A parameter is declared in the layer that introduces it. A layer that `needs`
that one and also reads it does not declare it again.

### A new machine

```sh
lami init bree --layers core,tools,gui
```

The layers are asked what they need to know, so the file arrives already
listing it:

```kdl
description "TODO: what this machine is"

layers "core" "tools" "gui"

// Required. Every one of these has to be answered before
// `lami diff` will run: there is no default profile here.

// which vendor driver to install (gui)
// one of: intel amd nvidia
gpu ""

// Optional: these have defaults, shown here commented out.

// external monitor brightness over DDC/CI (gui)
// ddc off
```

`--like frodo` fills the answers in from an existing host, for a machine much
like one you already have. Copying the file by hand instead means inheriting
its answers along with any parameter that has since stopped mattering.

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

#### Whole directories

A config directory with a dozen files in it does not need a dozen lines:

```kdl
dir "~/.config/fish" from="files/home/.config/fish"
```

Every file under the source is declared as if you had written it out by hand,
keeping its relative path. `lami why` and `capture` work on each of them
individually, so nothing is lost by the shorthand.

#### Suffixes carry meaning

Two suffixes on a *source* file change what happens to it, and both are
stripped from the target path:

| Source | Target | What happens |
|---|---|---|
| `files/hyprland.conf` | `hyprland.conf` | copied verbatim |
| `files/hyprland.conf.tmpl` | `hyprland.conf` | rendered with this host's parameters |
| `files/ssh-config.age` | `ssh-config` | decrypted, and written `0600` |

**Only `.tmpl` files are rendered.** A static file containing `{{` -- a shell
script, a CSS file, a Jinja template you are managing as data -- would
otherwise break, and `capture` would write the rendered output back over the
template and destroy it. Making it a suffix means the decision is visible in
the filename rather than inferred from the content.

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

### Services

A bare name means enabled, because that is what a declaration almost always
means. Say otherwise when you mean otherwise:

```kdl
services {
    greetd
    sshd
    bluetooth         disabled    // actively off, not merely unmentioned
    systemd-networkd  masked      // cannot start even as a dependency
}

user-services {
    pipewire.socket
    wireplumber
}
```

`disabled` is worth writing even though leaving the line out would also leave
the unit off: it says the choice was made rather than forgotten, and lami then
actively turns it off instead of ignoring it.

User units go through `systemctl --user -M <user>@`, which routes via
systemd-machined and therefore works whether or not the user has a live
session.

The `.service` suffix is inferred; `.timer`, `.socket` and the rest are kept as
written.

**lami never starts or stops a service.** Enabling is a statement about the
next boot; deciding something should be running right now is yours to make.

### Unit files

Write one like any other file. Anything under a systemd unit directory implies
a `daemon-reload`, so it cannot be forgotten — and forgetting it is a nasty
failure, because the unit silently keeps running its old definition:

```kdl
file "/etc/systemd/system/backup.timer" from="files/backup.timer"
file "~/.config/systemd/user/sync.service" from="files/sync.service"

services { backup.timer }
```

A drop-in counts as belonging to its unit:
`/etc/systemd/system/foo.service.d/override.conf` is `foo.service`.

If a unit should come back up after lami rewrites its own file, say so:

```kdl
services {
    my-daemon  restart-on-change
}
```

That is narrow on purpose — it restarts only when *that unit's* file or drop-in
changes. For anything else, write the hook out, so the consequence is visible:

```kdl
on-change "/etc/greetd/config.toml" {
    run "systemctl restart greetd"    // this ends your session
}
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

### One-off steps

Some things happen once and cannot be described as a state: hardware detection
that writes a file only it knows the contents of, an interactive installer, a
step that only works after something else has run at least once.

```kdl
migration "sensors-detect" from="scripts/sensors-detect.sh"
    because="writes HWMON_MODULES into /etc/conf.d/lm_sensors -- nothing else knows it"
```

Declared in the layer it belongs to, so a machine only gets the one-offs its
own layers bring. It is keyed by the name: moving or editing the script does
not re-run it, and giving it a new name is how you ask for it to happen again.

```
$ lami why sensors-detect
sensors-detect  (migration)
  declared:   layers/gui/layer.kdl:74
  applies:    layer 'gui'
  script:     layers/gui/scripts/sensors-detect.sh
  because:    writes HWMON_MODULES into /etc/conf.d/lm_sensors
  status:     already run on this machine
```

They run last, as root, with the layer's directory as the working directory
and `LAMI_HOST`, `LAMI_USER`, `LAMI_HOME` set. Write them to be safe to run
twice anyway: the record protects them, but a lost state file should not be a
disaster.

### Is this file already managed?

```sh
lami why ~/.config/fish/config.fish     # or the absolute path; both work
lami why /etc/fstab
lami render --list                      # every managed path, in one list
```

```
~/.config/fish/config.fish  (file)
  declared:   layers/tools/layer.kdl:34
  applies:    layer 'tools'
  content:    layers/tools/files/home/.config/fish/config.fish
  permissions: 644 pottom:pottom  (under the user's home)

/etc/fstab  (not managed by lami)
  the file exists, but no layer this host enables declares it
```

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

Two details of that order are worth knowing, because both were found by
installing a config into an empty machine and watching it fail:

- **`/etc/pacman.conf` is written first**, before packages, if a layer manages
  it. It decides which repositories exist, so applying it after the package
  phase means the first run installs from a repository list the config has not
  applied yet. It is the only file treated this way.
- **The machine is read again after packages are installed.** A unit whose
  package did not exist a moment ago cannot be reported as disabled, so
  otherwise every service installed by that run would be left alone and a
  second apply would be needed.

**`apply` never removes anything.** A typo'd layer name cannot uninstall your
desktop.

---

## 6. Capture: the other direction

You changed something on the machine and want it in the repo.

```sh
lami capture                                   # what is on offer
lami capture --file /etc/pacman.conf --dry-run
lami capture --file /etc/pacman.conf
lami capture --layer tools --package cowsay
lami capture --layer dev --package podman,podman-compose   # several at once
lami capture --layer dev --all                 # everything no layer declares
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

That is the whole interface. There is no attribute to remember, so there is no
way to commit a secret in the clear by forgetting one.

### Setting it up, once

```sh
mkdir -p ~/.config/age && chmod 700 ~/.config/age
age-keygen -o ~/.config/age/lami.txt
chmod 600 ~/.config/age/lami.txt
age-keygen -y ~/.config/age/lami.txt      # the public key, for config.kdl
```

```kdl
// config.kdl, at the root of the config directory
age {
    identity  "~/.config/age/lami.txt"    // may be repeated
    recipient "age1ncuc0xf8vd938aq..."    // may be repeated
}
```

Both lists take more than one line. Two identities and two recipients is the
normal arrangement: a YubiKey for everyday use and a file key kept somewhere
safe, so a key left in the other room does not mean an unreadable config.

**Keep the key outside the config directory.** lami refuses one inside it, and
follows symlinks to decide — `~/.config/lami` is commonly a symlink to the
repo, which makes this an easy mistake with a permanent consequence:

```
× the age identity is inside the config repository
 ╰── the next `git push` would publish your private key
```

An identity file readable by anyone else is refused too, the way ssh refuses
one.

### Encrypting something

```sh
age -r "$(age-keygen -y ~/.config/age/lami.txt)" \
    -o layers/net/files/home/.ssh/config.age ~/.ssh/config
```

Then declare it, and everything else works as usual: `lami diff` decrypts to
compare, `lami render` prints the plaintext, `lami apply` writes it.

### What it does for you

- **A decrypted file is always `0600`**, wherever it lands. The encryption is
  the statement that it is secret; the mode follows from that rather than from
  where the file happens to go.
- **`capture` re-encrypts** rather than writing the live file through. Without
  that, pulling a change back would put the secret into the repo in the clear,
  under a name ending in `.age` — the last place anybody would look for a
  leak. It prints no content either: a terminal is scrollback, and a diff of
  ciphertext says nothing anyway, since age picks a fresh file key every time.
- **More than one recipient** is how a second machine or a YubiKey is added:
  append its public key and re-encrypt. No key is ever copied between
  machines.

Getting the first key onto a new machine is the one step nothing can automate —
a YubiKey, a password manager, or a USB stick.

---

## 9. The repo, and a second machine

Your config is a git repository, and lami knows how to find it, update it and
send it back. On a new machine that is one command:

```sh
lami clone git@github.com:you/lami-config.git
```

That clones it, remembers where it went, and tells you straight away whether
this machine is described:

```
config repo
  git clone git@github.com:you/lami-config.git
  /home/you/.config/lami
  recorded in /home/you/.config/lami.kdl

  this host (sam) is described. Next:

    lami diff
    sudo lami apply
```

### Where the repo lives

By default the working copy goes to `~/.config/lami`. If you would rather keep
it with your other projects, say so:

```sh
lami clone git@github.com:you/lami-config.git --path ~/Projects/lami-config
```

Either way the answer is written to `~/.config/lami.kdl`, which is the one
file lami reads before it reads anything else:

```kdl
// Where lami finds your config repo. Written by `lami clone`;
// edit it freely, or override with --repo / --config-dir.
repo "git@github.com:you/lami-config.git"
path "~/Projects/lami-config"
```

Both lines are optional, and both have a command line equivalent:

| Where it comes from | Wins over |
|---|---|
| `--config-dir <path>` (or `LAMI_CONFIG_DIR`) | everything |
| `path` in `~/.config/lami.kdl` | the default |
| `~/.config/lami` | — |

`--repo <url>` (or `LAMI_REPO`) is the same for the remote. If the working
copy is missing and a URL is known, lami clones it before doing anything else,
so a machine with nothing on it can go straight to:

```sh
lami --repo git@github.com:you/lami-config.git diff
```

Unlike `lami clone`, that is a one-off: nothing is recorded.

### Keeping it in sync

```sh
lami pull            # fast-forward to the remote
lami push            # commit everything and send it
lami push -m "add the nvidia desktop"
```

`pull` is `--ff-only`: a config repo should not grow a merge commit behind your
back, and a diverged history is something you want to see in git rather than
have a tool resolve. Uncommitted work is left where it is, and reported.

Neither command parses the config first. That is deliberate — a config that
does not load is exactly when you most want to pull the fix.

Both refuse to run under `sudo`: git needs your ssh agent and credential
helper, which do not survive it, and anything git wrote would end up owned by
root.

### Adding the machine

If the hostname is not in `hosts/`, lami stops with an error rather than
guessing. There is no default profile: a typo'd hostname or a fresh VM must not
quietly receive somebody else's configuration.

1. Write `hosts/<name>.kdl` with its layers and parameters.
2. `lami diff` to see what it would do.
3. `sudo lami apply`.

---

## Colour

Output is coloured when it goes to a terminal, and plain otherwise — so piping
into `grep`, `less -F` or a file gives clean text without asking.

Colour carries meaning rather than decoration: green adds, yellow changes, red
removes, cyan runs a command. The sigils (`+ ~ - >`) say the same thing, so
nothing is lost without it.

To turn it off explicitly:

```sh
lami diff --no-color
NO_COLOR=1 lami diff        # the de-facto standard, honoured everywhere
```

## Everyday loop

```sh
lami diff          # what drifted
lami capture       # pull it back into the repo
lami push          # send it

# on the other machine
lami pull
lami diff
sudo lami apply
```
