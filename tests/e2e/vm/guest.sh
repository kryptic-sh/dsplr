#!/bin/bash
# Runs inside CI's e2e VM as root, from cloud-init. /e2e is the host's share:
# `harness.sh` and `test` (one desktop's e2e harness and test binary) and
# `packages` (what to install). Installs them, runs the test under the harness
# as an unprivileged user, leaves the logs and the exit status in /e2e/out and
# powers off.
set -x
out=/e2e/out
read -r -a packages < /e2e/packages
{
  pacman -Sy --noconfirm archlinux-keyring
  pacman -Syu --noconfirm --needed seatd "${packages[@]}"
} > "$out/pacman.log" 2>&1

# The VM is disposable, so here a compositor may own the VM's own seat:
# seatd lets Hyprland's DRM backend open the virtio GPU, which is how
# Hyprland's own CI runs it. On a desktop seatd is not running, and the
# harness's LIBSEAT_BACKEND=seatd then fails without touching logind.
systemctl start seatd
useradd -m -G video,render,seat t
ls -l /dev/dri > "$out/dri.txt" 2>&1

cp /e2e/harness.sh /e2e/test /home/t/
chown t: /home/t/harness.sh /home/t/test
su t -c "cd && ./harness.sh ./test --ignored --test-threads=1" > "$out/test.log" 2>&1
echo $? > "$out/status"
sync
poweroff
