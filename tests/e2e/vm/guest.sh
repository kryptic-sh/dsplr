#!/bin/bash
# Runs inside CI's e2e VM as root, from cloud-init. /e2e is the host's share:
# `harness.sh` (one desktop's e2e harness), `tests/` (that desktop's e2e test
# binaries, one per package), `dsplr` (the binary the CLI's test runs) and
# `packages` (what to install). Installs them, runs each test under its own
# harness as an unprivileged user, leaves the logs and the exit status (0 only
# when every test passed) in /e2e/out and powers off.
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

cp -r /e2e/harness.sh /e2e/dsplr /e2e/tests /home/t/
chown -R t: /home/t/harness.sh /home/t/dsplr /home/t/tests
status=0
for test in /home/t/tests/*; do
  name=$(basename "$test")
  su t -c "cd && DSPLR_BIN=/home/t/dsplr ./harness.sh ./tests/$name --ignored --test-threads=1" \
    > "$out/test-$name.log" 2>&1 || status=1
done
echo "$status" > "$out/status"
sync
poweroff
