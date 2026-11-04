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
#
# Hyprland still needs a GPU render node to allocate buffers, so on a runner
# without one this fails at "Hyprland did not start" or with 0x0 outputs.
set -uo pipefail

runtime=$(mktemp -d)
chmod 700 "$runtime"
cleanup() {
  kill "${hypr_pid:-}" "${parent_pid:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

env -i HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
  --socket wayland-0 --width 1920 --height 1080 > "$runtime/parent.log" 2>&1 &
parent_pid=$!
for _ in $(seq 100); do [ -S "$runtime/wayland-0" ] && break; sleep 0.1; done
if ! [ -S "$runtime/wayland-0" ]; then
  echo "KWin did not start" >&2
  cat "$runtime/parent.log" >&2
  exit 1
fi

printf 'hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })\n' \
  > "$runtime/hyprland.lua"
env -i HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
  HYPRLAND_NO_SD_NOTIFY=1 HYPRLAND_NO_CRASHREPORTER=1 LIBSEAT_BACKEND=seatd \
  Hyprland --config "$runtime/hyprland.lua" > "$runtime/hypr.log" 2>&1 &
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

export XDG_RUNTIME_DIR="$runtime" HYPRLAND_INSTANCE_SIGNATURE="$sig"
# The e2e tests refuse to run without this, so they never reach a real
# Hyprland when started some other way.
export DSPLR_E2E_HYPRLAND="$runtime"
unset WAYLAND_DISPLAY DISPLAY SWAYSOCK NIRI_SOCKET
"$@"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== Hyprland's log" >&2
  grep -v 'Got format' "$runtime/hypr/$sig/hyprland.log" | tail -n 40 >&2
fi
exit "$status"
