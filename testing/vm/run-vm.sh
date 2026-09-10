#!/usr/bin/env bash
# Boot the test VM.
#
#   ./run-vm.sh iso     the install medium, with archiso's `script=` hook
#                       pointed at the local http server
#   ./run-vm.sh disk    what has been installed on the virtual disk
#
# Headless: the console is a unix socket (./vmcon) and the screen is read with
# QEMU's own screendump (./shot). Nothing needs a window.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/run"

[[ -e arch.iso ]] || { echo "run ../setup.sh first" >&2; exit 1; }
UUID_ARG="$(cat iso-uuid)"

common=(
    -machine q35,accel=kvm -cpu host -smp "${VCPUS:-4}" -m "${MEMORY:-8192}"
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd
    -drive if=pflash,format=raw,file=OVMF_VARS.fd
    -drive file=disk.qcow2,if=virtio,format=qcow2,cache=writeback
    -netdev "user,id=n0,hostfwd=tcp:127.0.0.1:${SSH_PORT:-2222}-:22"
    -device virtio-net-pci,netdev=n0
    -display none
    # A socket, not a file: after the install there is no ssh into the guest
    # until the config puts it there, and the console is what a person sitting
    # at the machine would have. logfile= keeps the transcript too.
    -chardev "socket,id=con0,path=serial.sock,server=on,wait=off,logfile=serial.log"
    -serial chardev:con0
    -monitor unix:monitor.sock,server,nowait
    -daemonize
)

case "${1:-iso}" in
  iso)
    exec qemu-system-x86_64 "${common[@]}" \
        -cdrom arch.iso \
        -kernel arch/boot/x86_64/vmlinuz-linux \
        -initrd arch/boot/x86_64/initramfs-linux.img \
        -append "archisobasedir=arch $UUID_ARG console=tty0 console=ttyS0,115200 script=http://10.0.2.2:${HTTP_PORT:-8000}/prepare.sh"
    ;;
  disk)
    exec qemu-system-x86_64 "${common[@]}"
    ;;
  *)
    echo "usage: $0 [iso|disk]" >&2; exit 1 ;;
esac
