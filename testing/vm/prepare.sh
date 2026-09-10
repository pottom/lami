#!/usr/bin/env bash
# Run by archiso's `script=` hook on the install medium. It does the minimum
# needed to drive the rest from the host over ssh: authorise a key and start
# sshd. Everything the test is actually about happens later.
exec > /dev/ttyS0 2>&1
set -x

install -d -m 700 /root/.ssh
curl -fsS "http://10.0.2.2:${HTTP_PORT:-8000}/authorized_keys" -o /root/.ssh/authorized_keys
chmod 600 /root/.ssh/authorized_keys

mkdir -p /etc/ssh/sshd_config.d
printf 'PermitRootLogin prohibit-password\n' > /etc/ssh/sshd_config.d/99-vmtest.conf
systemctl restart sshd || systemctl start sshd

ip -brief addr
echo "=== LAMI-VM-READY ==="
