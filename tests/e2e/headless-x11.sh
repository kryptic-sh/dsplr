#!/bin/bash
# Runs "$@" against an Xvfb with RandR that cannot touch the desktop session.
#
#   tests/e2e/headless-x11.sh cargo test -p dsplr-backend --test x11_e2e -- --ignored
#
# - Xvfb on a display number of its own, the first in :90-:120 nothing has;
#   no TCP. The
#   user's DISPLAY is never used: the tests get only the harness's, and
#   WAYLAND_DISPLAY is unset, as on an X11 desktop. Xvfb has one CRTC and one
#   output, `screen`, whose largest size is the one it starts at.
# - `-noreset`: an X server resets when its last client disconnects, which
#   drops modes created with RandR; dsplr connects afresh for each read.
# - A private XDG_RUNTIME_DIR, XDG_CONFIG_HOME, XDG_STATE_HOME and HOME, so
#   nothing reaches the user's ~/.config or ~/.local/state. cargo and rustup
#   keep their real homes through CARGO_HOME and RUSTUP_HOME, pinned before
#   HOME changes.
set -uo pipefail

# Records the X sockets and locks below :90, which it never takes; checked
# once Xvfb is up and at exit.
# shellcheck source=tests/e2e/x-guard.sh
source "$(dirname "${BASH_SOURCE[0]}")/x-guard.sh"
first=$x_guard_first

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home"
cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${xvfb_pid:-}" 2>/dev/null
  wait 2>/dev/null
  # After Xvfb has exited, which is when it removes its own socket.
  x_guard_check || exit 1
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# A display number nothing has: no lock, whatever process it names (Xvfb
# takes a lock whose pid it cannot see, in another PID namespace or really
# dead, as free, and would then replace, and on exit delete, the socket of a
# server still listening there), no socket file, and no socket bound to its
# name, as a file or in the abstract namespace (/proc/net/unix lists both).
# Never `-displayfd` on its own: it starts at :0, which is how a desktop's
# Xwayland lost its socket once (x-guard.sh).
bound=$(cat /proc/net/unix)
number=""
for candidate in $(seq "$first" 120); do
  socket="/tmp/.X11-unix/X$candidate"
  if [ ! -e "/tmp/.X${candidate}-lock" ] && [ ! -e "$socket" ] &&
    ! grep -qE "(@)?${socket}\$" <<< "$bound"; then
    number="$candidate"
    break
  fi
done
if [ -z "$number" ] || [ "$number" -lt "$first" ]; then
  echo "no free X display number in :$first-:120" >&2
  exit 1
fi

# The number given, and `-displayfd` to say when it listens: Xvfb writes the
# number to fd 3 once it does. Another server taking the number first makes
# Xvfb exit, which the wait sees.
env -i PATH="$PATH" HOME="$home" \
  Xvfb ":$number" -displayfd 3 -screen 0 1920x1080x24 -nolisten tcp -noreset \
  3> "$runtime/display" > "$runtime/xvfb.log" 2>&1 &
xvfb_pid=$!
ready=""
for _ in $(seq 100); do
  ready=$(tr -dc '0-9' < "$runtime/display" 2>/dev/null)
  [ -n "$ready" ] && break
  kill -0 "$xvfb_pid" 2>/dev/null || break
  sleep 0.1
done
if [ "$ready" != "$number" ] || ! kill -0 "$xvfb_pid" 2>/dev/null; then
  echo "Xvfb did not start on :$number (it reported :${ready:-nothing})" >&2
  tail -n 30 "$runtime/xvfb.log" >&2
  exit 1
fi
# Listening, and nothing below :$first touched: else stop before any test.
x_guard_check || exit 1

export DISPLAY=":$number" XDG_RUNTIME_DIR="$runtime" HOME="$home"
export XDG_CONFIG_HOME="$runtime/config" XDG_STATE_HOME="$runtime/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
# The e2e tests refuse to run without these, so they never reach a real X
# server when started some other way.
export DSPLR_E2E_X11="$runtime" DSPLR_E2E_X11_DISPLAY="$DISPLAY"
# An X11 desktop: no Wayland compositor and no session bus to find.
unset WAYLAND_DISPLAY SWAYSOCK HYPRLAND_INSTANCE_SIGNATURE NIRI_SOCKET \
  DBUS_SESSION_BUS_ADDRESS XAUTHORITY
# In a session and process group of its own, so cleanup can end all of it.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== Xvfb's log" >&2
  tail -n 40 "$runtime/xvfb.log" >&2
fi
exit "$status"
