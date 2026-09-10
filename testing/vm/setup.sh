#!/usr/bin/env bash
# Fetch what the test VM needs. Everything lands in run/, which is gitignored.
#
#   ./setup.sh            latest Arch ISO
#   ISO=/path/to.iso ./setup.sh    an ISO you already have
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
mkdir -p run
cd run

MIRROR="${MIRROR:-https://geo.mirror.pkgbuild.com/iso/latest}"

if [[ -n "${ISO:-}" ]]; then
    ln -sf "$ISO" arch.iso
elif [[ ! -e arch.iso ]]; then
    name="$(curl -fsS "$MIRROR/" | grep -oE 'archlinux-[0-9.]+-x86_64\.iso' | head -1)"
    [[ -n "$name" ]] || { echo "cannot find an ISO at $MIRROR" >&2; exit 1; }
    echo "== $name"
    curl -# -o "$name" "$MIRROR/$name"
    ln -sf "$name" arch.iso
fi

# The kernel and initramfs are passed to QEMU directly, because that is the
# only way to add a serial console and archiso's `script=` hook to the command
# line without rebuilding the image.
echo "== kernel and initramfs"
bsdtar -xf arch.iso arch/boot/x86_64/vmlinuz-linux arch/boot/x86_64/initramfs-linux.img

# The ISO identifies itself to its own initramfs by this UUID; read it from the
# image rather than hardcoding a value that changes every month.
bsdtar -xOf arch.iso loader/entries/01-archiso-linux.conf \
    | grep -oE 'archisosearchuuid=[^ ]+' > iso-uuid
cat iso-uuid

echo "== disk and firmware"
[[ -e disk.qcow2 ]] || qemu-img create -f qcow2 disk.qcow2 "${DISK_SIZE:-24G}"
cp -f /usr/share/edk2/x64/OVMF_VARS.4m.fd OVMF_VARS.fd
chmod u+w OVMF_VARS.fd

# A throwaway key: the guest is reinstalled from scratch, so it never gets to
# be a host you trust with a real one.
[[ -e key ]] || ssh-keygen -q -t ed25519 -N "" -C "lami-vm-test" -f key
cp -f key.pub authorized_keys

echo
echo "ready. next:  ./run-vm.sh iso"
