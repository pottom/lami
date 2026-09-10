# The VM test

An Arch install from nothing, in a headless QEMU guest, to answer one question
the unit tests cannot: does a manual base install plus `lami apply` actually
produce the machine the config describes?

It has already paid for itself. Every one of these was found here and nowhere
else:

- `apply` computed its plan once, up front, so no service whose package that
  same run installed ever got enabled — a fresh machine needed two runs.
- `/etc/pacman.conf` was written after packages, so the first run installed
  from a repository list the config had not applied yet. With `[multilib]`
  declared but its database never downloaded, pacman refuses the whole
  transaction.
- `pacman -S --needed` cannot claim a package that is already present as a
  dependency, so `apply` reported the same change forever.
- A layer that enables a service without declaring the package providing it
  passed in silence.

## Running it

```sh
./setup.sh                 # ISO, disk image, firmware vars, a throwaway ssh key
./serve.sh &               # serves prepare.sh to the guest on 10.0.2.2:8000
./run-vm.sh iso            # boot the install medium
```

The medium boots with archiso's `script=` hook pointed at `prepare.sh`, which
authorises the key and starts sshd — the minimum needed to drive the rest from
here. Then:

```sh
./vm-ssh 'cat > /root/base-install.sh' < base-install.sh
./vm-ssh 'bash /root/base-install.sh'    # partition, pacstrap, chroot, bootloader
pkill -f 'qemu-system-x86_64'
./run-vm.sh disk                         # boot what was installed
```

From here there is no ssh into the guest — nothing has installed it — so the
console is the way in, exactly as it would be for a person at the machine:

```sh
./vmcon login root <password>
./vmcon run "curl -fsS http://10.0.2.2:8000/bootstrap.sh -o /tmp/b.sh"
./vmcon run "su - <user> -c 'bash /tmp/b.sh'"
./vmcon run "su - <user> -c 'sudo lami apply'"
```

And to see the screen, without a window or any image tooling:

```sh
./shot greeter        # -> run/greeter.png
./qmon "sendkey ret"  # QEMU's monitor: keystrokes, screendumps, anything
```

## The pieces

| | |
|---|---|
| `setup.sh` | downloads the ISO, extracts its kernel and initramfs, makes the disk |
| `run-vm.sh` | `iso` or `disk`; headless, serial on a socket, monitor on a socket |
| `serve.sh` | http on 10.0.2.2 for the guest to fetch from |
| `prepare.sh` | runs on the install medium via archiso's `script=` hook |
| `base-install.sh` | the manual install, unattended |
| `vmcon` | log in and run commands on the serial console |
| `shot` | screenshot to PNG, standard library only |
| `qmon` | raw QEMU monitor commands |

`run/` holds everything downloaded or generated and is gitignored.

## Why the kernel is passed directly

`run-vm.sh` boots the ISO with `-kernel` and `-initrd` rather than letting the
medium's own bootloader run. That is the only way to add a serial console and
the `script=` hook to archiso's command line without rebuilding the image. The
ISO identifies itself to its initramfs by a UUID that changes with every
monthly release, so `setup.sh` reads it out of the image instead of hardcoding
it.

## Deviations from a physical install

Three, all in `base-install.sh` and all commented there:

- no microcode package, because a guest has none to load
- `console=ttyS0` on the kernel command line, because that is how the harness
  reaches the installed system
- partitioning with `sgdisk` instead of `cfdisk`, for the same layout
