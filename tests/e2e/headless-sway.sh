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
# - sway finds its config in a private XDG_CONFIG_HOME, the way it finds a
#   user's, and that config includes `outputs` beside it, which is where
#   `dsplr save` writes. The tests get the same XDG_CONFIG_HOME, and sway and
#   the tests a private HOME, so nothing reaches the user's ~/.config. cargo
#   and rustup keep their real homes through CARGO_HOME and RUSTUP_HOME,
#   pinned before HOME changes.
set -uo pipefail

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home"
cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it, such as a second sway a killed test left running.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${sway_pid:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# No bar, no wallpaper: nothing but the outputs, and those `dsplr save` writes.
mkdir -p "$runtime/config/sway"
printf 'swaybg_command -\ninclude outputs\n' > "$runtime/config/sway/config"
env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  XDG_CONFIG_HOME="$runtime/config" \
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway > "$runtime/sway.log" 2>&1 &
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

export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" SWAYSOCK="$swaysock" HOME="$home"
# sway's config, which `dsplr save` writes beside.
export XDG_CONFIG_HOME="$runtime/config"
# dsplr keeps its revert point under XDG_STATE_HOME: a private one, so a test
# never writes the user's.
export XDG_STATE_HOME="$runtime/state"
# The e2e tests refuse to run without this, so they never reach a real
# compositor when started some other way.
export DSPLR_E2E_SWAY="$runtime"
unset DISPLAY HYPRLAND_INSTANCE_SIGNATURE NIRI_SOCKET
# In a session and process group of its own, so cleanup can end all of it.
# A background job of a script is no group leader, so setsid runs it in
# place and its pid names the group.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== sway's log" >&2
  tail -n 40 "$runtime/sway.log" >&2
fi
exit "$status"
