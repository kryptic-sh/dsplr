#!/bin/bash
# Runs "$@" against a niri that cannot touch the desktop session.
#
#   tests/e2e/headless-niri.sh cargo test -p dsplr-backend --test niri_e2e -- --ignored
#
# - niri has no headless backend of its own, so it runs nested (winit) in a
#   headless sway: wlroots' headless backend and the pixman renderer, so no
#   window, no DRM and no seat. niri renders through EGL, which Mesa gives it
#   in software (llvmpipe). Its one output is `winit`, the size of its window;
#   a nested niri ignores a mode, a transform and adaptive sync for it, so the
#   tests change its scale and position.
# - Both run in a private XDG_RUNTIME_DIR with `env -i`, so no socket of the
#   session is reachable, and dsplr's detection cannot find the live desktop.
# - niri finds its config in a private XDG_CONFIG_HOME, the way it finds a
#   user's, and that config includes `outputs.kdl` beside it, which is where
#   `dsplr save` writes (optional, so niri starts before the first save). The
#   tests get the same XDG_CONFIG_HOME, and sway, niri and the tests a private
#   HOME, so nothing reaches the user's ~/.config. cargo and rustup keep their
#   real homes through CARGO_HOME and RUSTUP_HOME, pinned before HOME changes.
# - The tests learn niri's sockets as its own clients do: niri runs a command
#   at startup with its environment, which writes WAYLAND_DISPLAY and
#   NIRI_SOCKET to a file.
set -uo pipefail
# Records the X sockets and locks below :90, checked once the servers are up
# and at exit (x-guard.sh).
# shellcheck source=tests/e2e/x-guard.sh
source "$(dirname "${BASH_SOURCE[0]}")/x-guard.sh"

# The facts the tests rest on (the first node wins, writing applies, the
# answers to an apply) were checked on niri 26.04; say which niri this is, so
# a failure after an update shows it.
echo "== $(niri --version)" >&2

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home"
cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it, such as a second niri a killed test left running.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${niri_pid:-}" "${sway_pid:-}" 2>/dev/null
  wait 2>/dev/null
  # After the servers have exited, which is when they remove their sockets.
  x_guard_check || exit 1
}
trap cleanup EXIT
trap 'exit 130' INT TERM

mkdir -p "$runtime/config/sway" "$runtime/config/niri"
# Each niri's window floats, so a second niri a test starts does not halve
# the first's window, which is its output.
# No Xwayland: wlroots binds X display sockets at startup even when it starts
# Xwayland lazily, taking a display whose lock names a pid it cannot see
# (x-guard.sh), and no test needs X.
# niri's own X sockets (its xwayland-satellite integration, where that is
# installed; niri 26.04 skips any display with a lock, `pick_x11_display`)
# are turned off too.
printf 'swaybg_command -\nxwayland disable\nfor_window [all] floating enable\n' > "$runtime/config/sway/config"
printf 'xwayland-satellite {\n    off\n}\ninclude optional=true "outputs.kdl"\n' > "$runtime/config/niri/config.kdl"
env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  XDG_CONFIG_HOME="$runtime/config" \
  WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
  sway > "$runtime/sway.log" 2>&1 &
sway_pid=$!
parent="" swaysock=""
for _ in $(seq 100); do
  parent=$(find "$runtime" -maxdepth 1 -type s -name 'wayland-[0-9]*' -printf '%f\n' | head -n 1)
  swaysock=$(find "$runtime" -maxdepth 1 -name 'sway-ipc.*.sock' -print -quit)
  [ -n "$parent" ] && [ -n "$swaysock" ] && break
  kill -0 "$sway_pid" 2>/dev/null || break
  sleep 0.1
done
if [ -z "$parent" ] || [ -z "$swaysock" ]; then
  echo "sway did not start" >&2
  tail -n 30 "$runtime/sway.log" >&2
  exit 1
fi

env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  XDG_CONFIG_HOME="$runtime/config" WAYLAND_DISPLAY="$parent" \
  LIBGL_ALWAYS_SOFTWARE=1 \
  niri -- sh -c 'printf "%s\n%s\n" "$WAYLAND_DISPLAY" "$NIRI_SOCKET" > "$XDG_RUNTIME_DIR/niri.env.tmp" && mv "$XDG_RUNTIME_DIR/niri.env.tmp" "$XDG_RUNTIME_DIR/niri.env"' \
  > "$runtime/niri.log" 2>&1 &
niri_pid=$!
for _ in $(seq 100); do
  [ -s "$runtime/niri.env" ] && break
  kill -0 "$niri_pid" 2>/dev/null || break
  sleep 0.1
done
{ read -r display && read -r niri_socket; } < "$runtime/niri.env" 2>/dev/null
if [ -z "${display:-}" ] || ! [ -S "${niri_socket:-}" ]; then
  echo "niri did not start" >&2
  tail -n 30 "$runtime/niri.log" >&2
  exit 1
fi

# The servers are up and nothing below :90 was touched; else stop before any
# test.
x_guard_check || exit 1
export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$display" NIRI_SOCKET="$niri_socket" HOME="$home"
# niri's config, which `dsplr save` writes beside.
export XDG_CONFIG_HOME="$runtime/config"
# dsplr keeps its revert point under XDG_STATE_HOME: a private one, so a test
# never writes the user's.
export XDG_STATE_HOME="$runtime/state"
# The headless sway niri runs in, for a second niri a test starts, and its
# IPC socket, which a compositor nested in sway would inherit.
export DSPLR_E2E_NIRI_PARENT="$parent" DSPLR_E2E_NIRI_PARENT_SWAYSOCK="$swaysock"
# The e2e tests refuse to run without this, so they never reach a real
# compositor when started some other way.
export DSPLR_E2E_NIRI="$runtime"
# No session bus: dsplr's GNOME detection asks the one the environment
# names, else the private runtime's, where there is none.
unset DISPLAY HYPRLAND_INSTANCE_SIGNATURE SWAYSOCK NIRI_CONFIG DBUS_SESSION_BUS_ADDRESS
# In a session and process group of its own, so cleanup can end all of it.
# A background job of a script is no group leader, so setsid runs it in
# place and its pid names the group.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== niri's log" >&2
  grep -v DEBUG "$runtime/niri.log" | tail -n 40 >&2
fi
exit "$status"
