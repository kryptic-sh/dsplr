#!/bin/bash
# Runs inside CI's e2e VM as root, from cloud-init. /e2e is the host's share:
# `harness.sh` (one desktop's e2e harness), `tests/` (that desktop's e2e test
# binaries, one per package), `dsplr` (the binary the CLI's test runs),
# `packages` (what to install) and `env` (NAME=VALUE words the tests run
# with, maybe none; no value may hold a space). Installs them, runs each test
# under its own harness as an unprivileged user, leaves the logs and the exit
# status (0 only when every test passed) in /e2e/out and powers off.
#
# A step that leaves nothing to test with (the packages, the test user) ends
# the run there: its status says which ("pacman failed"), and `reason` holds
# why, with the tail of its log, which the CI step prints.
set -x
out=/e2e/out
read -r -a packages < /e2e/packages
test_env=()
[ -f /e2e/env ] && read -r -a test_env < /e2e/env

# Ends the run: `$1` as the status, and it with the tail of the log `$2`
# as the reason.
fail() {
  echo "$1" > "$out/status"
  # Only stdout: `set -x` traces to stderr, which goes to the console.
  { echo "$1"; [ -n "${2:-}" ] && tail -n 40 "$2"; } > "$out/reason"
  sync
  poweroff
  exit 1
}

# A mirror can drop the connection partway through a download, so each
# attempt runs the whole step again, after a longer wait each time.
attempts=4
for attempt in $(seq "$attempts"); do
  {
    echo "== attempt $attempt of $attempts"
    pacman -Sy --noconfirm archlinux-keyring &&
      pacman -Syu --noconfirm --needed seatd "${packages[@]}"
  } >> "$out/pacman.log" 2>&1 && break
  [ "$attempt" -eq "$attempts" ] && fail "pacman failed" "$out/pacman.log"
  sleep $((attempt * 15))
done

# The VM is disposable, so here a compositor may own the VM's own seat:
# seatd lets Hyprland's DRM backend open the virtio GPU, which is how
# Hyprland's own CI runs it. On a desktop seatd is not running, and the
# harness's LIBSEAT_BACKEND=seatd then fails without touching logind.
systemctl start seatd > "$out/seatd.log" 2>&1 || fail "seatd did not start" "$out/seatd.log"
useradd -m -G video,render,seat t > "$out/useradd.log" 2>&1 ||
  fail "useradd failed" "$out/useradd.log"
ls -l /dev/dri > "$out/dri.txt" 2>&1

cp -r /e2e/harness.sh /e2e/dsplr /e2e/tests /home/t/
chown -R t: /home/t/harness.sh /home/t/dsplr /home/t/tests
status=0
for test in /home/t/tests/*; do
  name=$(basename "$test")
  su t -c "cd && env ${test_env[*]} DSPLR_BIN=/home/t/dsplr ./harness.sh ./tests/$name --ignored --test-threads=1" \
    > "$out/test-$name.log" 2>&1 || status=1
done
echo "$status" > "$out/status"
sync
poweroff
