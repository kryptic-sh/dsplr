#!/bin/bash
# Runs "$@" against a headless sway that cannot touch the desktop session.
#
#   tests/e2e/headless-sway.sh cargo test -p dsplr-backend --test sway_e2e -- --ignored
#
# - wlroots' headless backend: no window, no DRM, no seat, and the pixman
#   renderer, so no GPU either. It starts with one output, HEADLESS-1; a test
#   makes more with `swaymsg create_output`.
# - sway runs in a private XDG_RUNTIME_DIR with `env -i`, so no socket of the
#   session is reachable, and dsplr's detection cannot find the live desktop.
set -uo pipefail

runtime=$(mktemp -d)
chmod 700 "$runtime"
cleanup() {
  kill "${sway_pid:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

# No bar, no wallpaper: nothing but the outputs.
printf 'swaybg_command -\n' > "$runtime/sway.conf"
env -i HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway --config "$runtime/sway.conf" > "$runtime/sway.log" 2>&1 &
sway_pid=$!
socket="" swaysock=""
for _ in $(seq 100); do
  socket=$(find "$runtime" -maxdepth 1 -type s -name 'wayland-[0-9]*' -printf '%f\n' | head -n 1)
  swaysock=$(find "$runtime" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit)
  [ -n "$socket" ] && [ -n "$swaysock" ] && break
  kill -0 "$sway_pid" 2>/dev/null || break
  sleep 0.1
done
if [ -z "$socket" ] || [ -z "$swaysock" ]; then
  echo "sway did not start" >&2
  tail -n 30 "$runtime/sway.log" >&2
  exit 1
fi

export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" SWAYSOCK="$swaysock"
# dsplr keeps its revert point under XDG_STATE_HOME: a private one, so a test
# never writes the user's.
export XDG_STATE_HOME="$runtime/state"
# The e2e tests refuse to run without this, so they never reach a real
# compositor when started some other way.
export DSPLR_E2E_SWAY="$runtime"
unset DISPLAY HYPRLAND_INSTANCE_SIGNATURE NIRI_SOCKET
"$@"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== sway's log" >&2
  tail -n 40 "$runtime/sway.log" >&2
fi
exit "$status"
