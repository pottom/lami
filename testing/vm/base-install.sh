#!/usr/bin/env bash
# The manual part of an Arch install, carried out unattended in the test VM.
#
# Run from the live medium (the harness copies it there). It follows the
# ordinary documented procedure -- partition, subvolumes, pacstrap, chroot,
# systemd-boot -- because the point of the exercise is to check that the
# documented procedure plus `lami apply` produces a working machine.
#
# Where it must differ from a physical install, the reason is in a comment.
# Those comments are the only places the VM is not a faithful rehearsal.
set -euo pipefail

DISK="${DISK:-/dev/vda}"   # virtio, so no `p` between disk and partition number
P1="${DISK}1"
P2="${DISK}2"
VM_HOSTNAME="${VM_HOSTNAME:-pippin}"
VM_USER="${VM_USER:-arch}"
VM_PASSWORD="${VM_PASSWORD:-arch}"
TIMEZONE="${TIMEZONE:-UTC}"

echo "== partitions =="
sgdisk --zap-all "$DISK"
sgdisk -n1:0:+1G -t1:ef00 -c1:EFI -n2:0:0 -t2:8300 -c2:root "$DISK"
partprobe "$DISK"; sleep 1

mkfs.fat -F32 "$P1"
mkfs.btrfs -f -L arch "$P2"

echo "== subvolumes =="
# @log and @pkg are separate so they stay out of snapshots: the package cache
# is large, and logs are the one thing you want to survive a rollback.
# @snapshots is separate so the root snapshot does not contain itself.
mount "$P2" /mnt
for sv in @ @home @snapshots @log @pkg; do
    btrfs subvolume create "/mnt/$sv"
done
umount /mnt

echo "== mount =="
OPTS="noatime,compress=zstd:1"
mount -o "$OPTS,subvol=@" "$P2" /mnt
mount --mkdir -o "$OPTS,subvol=@home"      "$P2" /mnt/home
mount --mkdir -o "$OPTS,subvol=@snapshots" "$P2" /mnt/.snapshots
mount --mkdir -o "$OPTS,subvol=@log"       "$P2" /mnt/var/log
mount --mkdir -o "$OPTS,subvol=@pkg"       "$P2" /mnt/var/cache/pacman/pkg
mount --mkdir "$P1" /mnt/boot
findmnt /mnt

echo "== pacstrap =="
# The minimum. Everything else is the config's job -- which is the thing under
# test. No microcode package: a guest has none to load.
pacstrap -K /mnt \
    base base-devel linux linux-firmware \
    btrfs-progs \
    networkmanager sudo git nano vim man-db man-pages texinfo \
    terminus-font usbutils pciutils

genfstab -U /mnt >> /mnt/etc/fstab

echo "== chroot =="
cat > /mnt/root/chroot.sh <<CHROOT
set -euo pipefail
VM_HOSTNAME="$VM_HOSTNAME"
VM_USER="$VM_USER"
VM_PASSWORD="$VM_PASSWORD"
TIMEZONE="$TIMEZONE"
P2="$P2"
CHROOT
cat >> /mnt/root/chroot.sh <<'CHROOT'
ln -sf "/usr/share/zoneinfo/$TIMEZONE" /etc/localtime
hwclock --systohc

sed -i 's/^#en_US.UTF-8 UTF-8/en_US.UTF-8 UTF-8/' /etc/locale.gen
locale-gen
printf 'LANG=en_US.UTF-8\n' > /etc/locale.conf
printf 'KEYMAP=us\nFONT=ter-120b\n' > /etc/vconsole.conf

echo "$VM_HOSTNAME" > /etc/hostname
cat > /etc/hosts <<X
# Static table lookup for hostnames.
# See hosts(5) for details.
127.0.0.1        localhost
::1              localhost
127.0.1.1	 $VM_HOSTNAME.localdomain $VM_HOSTNAME
X

echo "root:$VM_PASSWORD" | chpasswd
useradd -m -G wheel -s /bin/bash "$VM_USER"
echo "$VM_USER:$VM_PASSWORD" | chpasswd

# A drop-in, never the main sudoers file. No dot in the name: sudo silently
# skips any drop-in whose filename contains one.
printf '%s ALL=(ALL:ALL) NOPASSWD: ALL\n' "$VM_USER" > "/etc/sudoers.d/10-$VM_USER"
chmod 440 "/etc/sudoers.d/10-$VM_USER"
visudo -c

bootctl install
UUID=$(blkid -s UUID -o value "$P2")
# rootflags=subvol=@ is not belt-and-braces: the initramfs mounts the root
# from the kernel command line, before /etc/fstab is read, and subvol= cannot
# be changed by a remount. Without it the default subvolume is mounted, where
# there is no /sbin/init.
#
# console=ttyS0 is the VM's own addition: it is how the harness reaches the
# installed system, since nothing has installed sshd at this point.
cat > /boot/loader/entries/arch.conf <<X
title   Arch Linux
linux   /vmlinuz-linux
initrd  /initramfs-linux.img
options root=UUID=$UUID rootflags=subvol=@ rw console=tty0 console=ttyS0,115200
X
cat > /boot/loader/loader.conf <<'X'
default arch.conf
timeout 3
console-mode max
editor  yes
X

systemctl enable NetworkManager
CHROOT

arch-chroot /mnt /bin/bash /root/chroot.sh
rm -f /mnt/root/chroot.sh

echo "== done =="
umount -R /mnt
