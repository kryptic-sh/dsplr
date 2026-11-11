#!/bin/bash
# Runs "$@" against a headless Hyprland that cannot touch the desktop session.
#
#   tests/e2e/headless-hyprland.sh cargo test -p dsplr-backend --test hyprland_e2e -- --ignored
#
# - Hyprland is nested in KWin's virtual backend: no window, no DRM. Hyprland
#   0.57 needs xdg_wm_base v6 of its parent; sway 1.12 and weston offer v5.
# - Both run in a private XDG_RUNTIME_DIR with `env -i`, so no socket of the
#   session is reachable, and even hyprctl cannot see the live instance.
# - LIBSEAT_BACKEND=seatd: Hyprland always tries its DRM backend first, and
#   libseat's logind backend would ask to take control of the user's active
#   session. seatd is not running, so the attempt fails without asking.
# - Hyprland finds its config in a private XDG_CONFIG_HOME, the way it finds
#   a user's, and the tests get the same XDG_CONFIG_HOME, so `dsplr save`
#   writes its monitors.lua there and never into the user's config.
# - KWin, Hyprland and the tests get a private HOME too, so nothing that falls
#   back to ~/.config reaches the user's. cargo and rustup keep their real
#   homes through CARGO_HOME and RUSTUP_HOME, pinned before HOME changes.
#
# Hyprland still needs a GPU render node to allocate buffers, so on a runner
# without one this fails at "Hyprland did not start" or with 0x0 outputs.
set -uo pipefail
# Records the X sockets and locks below :90, checked once the servers are up
# and at exit (x-guard.sh).
# shellcheck source=tests/e2e/x-guard.sh
source "$(dirname "${BASH_SOURCE[0]}")/x-guard.sh"

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home"
cleanup() {
  kill "${hypr_pid:-}" "${parent_pid:-}" 2>/dev/null
  wait 2>/dev/null
  # After the servers have exited, which is when they remove their sockets.
  x_guard_check || exit 1
}
trap cleanup EXIT

env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
  --socket wayland-0 --width 1920 --height 1080 > "$runtime/parent.log" 2>&1 &
parent_pid=$!
for _ in $(seq 100); do [ -S "$runtime/wayland-0" ] && break; sleep 0.1; done
if ! [ -S "$runtime/wayland-0" ]; then
  echo "KWin did not start" >&2
  cat "$runtime/parent.log" >&2
  exit 1
fi

mkdir -p "$runtime/config/hypr"
# No Xwayland: Hyprland's binds X display sockets as wlroots' does (reported,
# not read here; x-guard.sh), and no test needs X. KWin, the parent, starts
# none without `--xwayland` (`main_wayland.cpp`).
printf 'hl.config({ xwayland = { enabled = false } })\nhl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })\n' \
  > "$runtime/config/hypr/hyprland.lua"
env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
  XDG_CONFIG_HOME="$runtime/config" \
  HYPRLAND_NO_SD_NOTIFY=1 HYPRLAND_NO_CRASHREPORTER=1 LIBSEAT_BACKEND=seatd \
  Hyprland > "$runtime/hypr.log" 2>&1 &
hypr_pid=$!
for _ in $(seq 150); do
  ls "$runtime"/hypr/*/.socket.sock > /dev/null 2>&1 && break
  kill -0 "$hypr_pid" 2>/dev/null || break
  sleep 0.1
done
sig=$(ls "$runtime/hypr" 2>/dev/null | head -n 1)
if [ -z "$sig" ] || ! [ -S "$runtime/hypr/$sig/.socket.sock" ]; then
  echo "Hyprland did not start" >&2
  tail -n 30 "$runtime/hypr.log" >&2
  exit 1
fi

export XDG_RUNTIME_DIR="$runtime" HYPRLAND_INSTANCE_SIGNATURE="$sig" HOME="$home"
# dsplr keeps its revert point under XDG_STATE_HOME: a private one, so a test
# never writes the user's.
export XDG_STATE_HOME="$runtime/state"
# Hyprland's config, which `dsplr save` writes beside.
export XDG_CONFIG_HOME="$runtime/config"
# The e2e tests refuse to run without this, so they never reach a real
# Hyprland when started some other way.
export DSPLR_E2E_HYPRLAND="$runtime"
# No session bus: dsplr's GNOME detection asks the one the environment
# names, else the private runtime's, where there is none.
unset WAYLAND_DISPLAY DISPLAY SWAYSOCK NIRI_SOCKET HYPRLAND_CONFIG DBUS_SESSION_BUS_ADDRESS
# The servers are up and nothing below :90 was touched; else stop before any
# test.
x_guard_check || exit 1
"$@"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== Hyprland's log" >&2
  grep -v 'Got format' "$runtime/hypr/$sig/hyprland.log" | tail -n 40 >&2
fi
exit "$status"
