#!/bin/bash
# Runs "$@" against a KWin that cannot touch the desktop session.
#
#   tests/e2e/headless-kwin.sh cargo test -p dsplr-backend --test kwin_e2e -- --ignored
#
# - KWin's virtual backend (`kwin_wayland --virtual`): no window, no DRM and
#   no seat (its session is a no-op one). `--output-count` makes that many
#   outputs, Virtual-0 and on, each `--width` by `--height` at 60 Hz, with that
#   one mode, no EDID and no capability but being turned off (KWin 6.8's
#   `src/main_wayland.cpp` and `src/backends/virtual/`). KWin still renders
#   with OpenGL, so it needs EGL: Mesa, on a GPU or in software.
# - KWin runs in a private XDG_RUNTIME_DIR with `env -i`, so no socket of the
#   session is reachable, and dsplr's detection cannot find the live desktop.
#   No Xwayland (it starts one only with `--xwayland`), no lock screen, no
#   global shortcuts.
# - KWin saves every configuration it applies to `kwinoutputconfig.json` in
#   Qt's config location, which is $XDG_CONFIG_HOME: a private one, shared
#   with the tests, which read it there. KWin and the tests get a private HOME
#   too, so nothing that falls back to ~/.config reaches the user's. cargo
#   and rustup keep their real homes through CARGO_HOME and RUSTUP_HOME,
#   pinned before HOME changes.
set -uo pipefail

# The facts the tests rest on were read in KWin 6.8.2's source; say which
# KWin this is, so a failure after an update shows it.
# `--version` builds KWin's application object first, so it gets a time limit.
echo "== $(timeout 10 kwin_wayland --version 2>/dev/null || echo 'kwin_wayland --version said nothing')" >&2

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home" "$runtime/config"
cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${kwin_pid:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# The number of outputs the tests expect; they check it.
outputs=3
env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  XDG_CONFIG_HOME="$runtime/config" \
  kwin_wayland --virtual --no-lockscreen --no-global-shortcuts \
  --socket wayland-0 --width 1920 --height 1080 --output-count "$outputs" \
  > "$runtime/kwin.log" 2>&1 &
kwin_pid=$!
for _ in $(seq 100); do
  [ -S "$runtime/wayland-0" ] && break
  kill -0 "$kwin_pid" 2>/dev/null || break
  sleep 0.1
done
if ! [ -S "$runtime/wayland-0" ]; then
  echo "KWin did not start" >&2
  tail -n 30 "$runtime/kwin.log" >&2
  exit 1
fi

export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 HOME="$home"
# KWin's config location, where it writes kwinoutputconfig.json.
export XDG_CONFIG_HOME="$runtime/config"
# dsplr keeps its revert point under XDG_STATE_HOME: a private one, so a test
# never writes the user's.
export XDG_STATE_HOME="$runtime/state"
export DSPLR_E2E_KWIN_OUTPUTS="$outputs"
# The e2e tests refuse to run without this, so they never reach a real
# compositor when started some other way.
export DSPLR_E2E_KWIN="$runtime"
unset DISPLAY HYPRLAND_INSTANCE_SIGNATURE SWAYSOCK NIRI_SOCKET
# In a session and process group of its own, so cleanup can end all of it.
# A background job of a script is no group leader, so setsid runs it in
# place and its pid names the group.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== KWin's log" >&2
  tail -n 40 "$runtime/kwin.log" >&2
fi
exit "$status"
