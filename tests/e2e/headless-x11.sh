#!/bin/bash
# Runs "$@" against an X server with RandR that cannot touch the desktop
# session: Xvfb, or with DSPLR_E2E_X11_SERVER=xorg Xorg's dummy driver.
#
#   tests/e2e/headless-x11.sh cargo test -p dsplr-backend --test x11_e2e -- --ignored
#   DSPLR_E2E_X11_SERVER=xorg tests/e2e/headless-x11.sh \
#     cargo test -p dsplr-backend --test xorg_e2e -- --ignored
#
# - The server on a display number of its own, the first in :90-:120 nothing
#   has; no TCP. The user's DISPLAY is never used: the tests get only the
#   harness's, and WAYLAND_DISPLAY is unset, as on an X11 desktop.
# - Xvfb has one CRTC and one output, `screen`, whose largest size is the one
#   it starts at.
# - Xorg (`xf86-video-dummy` on Arch) runs unprivileged as the server binary
#   (DSPLR_E2E_XORG, else /usr/lib/Xorg or /usr/lib/xorg/Xorg; never
#   Xorg.wrap, which refuses anyone not on a console), only with
#   DSPLR_E2E_XORG_MAY_TAKE_SESSION=1: an Xorg that reached systemd-logind
#   would ask it for the session its process is in and take control of it,
#   its VT and its GPU (xorg-server 21.1.26, `systemd-logind.c`). CI sets it,
#   in a VM; on a desktop it is the user's call. Two things keep Xorg from
#   either, each enough on its own, and its log is checked for both:
#   - DBUS_SYSTEM_BUS_ADDRESS names a socket that does not exist, so it never
#     reaches the system bus (`dbus-core.c`), the only way it reaches logind;
#     checked first with dbus-send, which must fail with the same address;
#   - `-seat` names a seat no device is on, so the platform probe skips every
#     /dev/dri card (`config_udev_odev_probe`: the seat's udev tag, then
#     `check_seat`) and no console or VT is opened (non-seat0).
#   Its config is its own: `dsplr-xorg.conf`, a stub, and
#   `xorg.conf.d/00-dsplr.conf` in the run's directory, each found there, so
#   the fallbacks to /etc/X11/xorg.conf and /etc/X11/xorg.conf.d are never
#   reached. /usr/share/X11/xorg.conf.d is read whatever is given (its
#   InputClass rules then match no device). No input devices: hotplugging
#   is left on (AutoAddDevices and AutoEnableDevices), as turning either off
#   makes Xorg force a default `mouse` and `kbd` device, which open the
#   devices directly whatever the seat (`checkCoreInputDevices`), and the
#   udev devices it would hotplug are none, as none is on its seat. No GPU,
#   no GLX. DSPLR_E2E_XORG_MODULES, when set, is its module path, for a server
#   unpacked somewhere other than the system. The dummy driver has sixteen
#   outputs: DUMMY0 on, the others disconnected with no modes. The harness
#   adds CVT's 1280x720 to DUMMY1 and DUMMY2, so xrandr can turn them on,
#   which makes them connected: a monitor plugged in. Turned off again they
#   stay connected (dummy 0.4.1), so there is no unplugging.
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

server="${DSPLR_E2E_X11_SERVER:-xvfb}"
case "$server" in
  xvfb) ;;
  xorg)
    if [ "${DSPLR_E2E_XORG_MAY_TAKE_SESSION:-}" != 1 ]; then
      echo "DSPLR_E2E_X11_SERVER=xorg runs Xorg unprivileged, which could take control of a login session and its VT if this harness's guards failed; set DSPLR_E2E_XORG_MAY_TAKE_SESSION=1 to run it (CI does, in a VM)" >&2
      exit 1
    fi
    xorg="${DSPLR_E2E_XORG:-}"
    if [ -z "$xorg" ]; then
      for candidate in /usr/lib/Xorg /usr/lib/xorg/Xorg; do
        if [ -x "$candidate" ]; then
          xorg="$candidate"
          break
        fi
      done
    fi
    if [ -z "$xorg" ] || [ ! -x "$xorg" ] || ! command -v xrandr > /dev/null ||
      ! command -v dbus-send > /dev/null; then
      echo "DSPLR_E2E_X11_SERVER=xorg needs Xorg's server binary, its dummy driver, xrandr and dbus-send" >&2
      exit 1
    fi
    ;;
  *)
    echo "DSPLR_E2E_X11_SERVER is xvfb or xorg, not $server" >&2
    exit 1
    ;;
esac

runtime=$(mktemp -d)
chmod 700 "$runtime"
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
home="$runtime/home"
mkdir "$home"
# Whether Xorg's log and stderr say it stayed off the system bus, so off
# logind, off every /dev/dri card, and off every input device; says what it
# found when not.
xorg_log_check() {
  local said
  said=$(cat "$runtime/xvfb.log" "$runtime/xorg.stderr" 2> /dev/null)
  if ! grep -q "dbus-core: error connecting to system bus" <<< "$said"; then
    echo "!! Xorg's log does not say it failed to reach the system bus" >&2
    return 1
  fi
  if grep -nE "systemd-logind|/dev/dri|Platform probe|Adding drm device|LoadModule: \"(mouse|kbd|evdev|libinput)\"|Adding input device|Using input driver" \
    <<< "$said" >&2; then
    echo "!! Xorg's log mentions logind, a DRM device or an input device (above)" >&2
    return 1
  fi
}

cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${xvfb_pid:-}" 2>/dev/null
  wait 2>/dev/null
  # After Xvfb has exited, which is when it removes its own socket.
  x_guard_check || exit 1
  if [ "${server:-}" = xorg ] && [ -n "${xvfb_pid:-}" ]; then
    xorg_log_check || exit 1
  fi
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

# The number given, and `-displayfd` to say when it listens: the server writes
# the number to fd 3 once it does. Another server taking the number first
# makes it exit, which the wait sees.
if [ "$server" = xorg ]; then
  # Never on the system bus, so never to logind: checked with a libdbus
  # client first, asking the bus for its id, which any client of the real
  # bus may, so it answers if the variable were ignored.
  no_bus="unix:path=$runtime/no-system-bus"
  if DBUS_SYSTEM_BUS_ADDRESS="$no_bus" dbus-send --system --print-reply \
    --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.GetId \
    > /dev/null 2>&1; then
    echo "!! dbus-send reached a system bus with DBUS_SYSTEM_BUS_ADDRESS=$no_bus; not starting Xorg" >&2
    exit 1
  fi
  mkdir "$runtime/xorg.conf.d"
  echo "# dsplr's e2e: the config is xorg.conf.d/00-dsplr.conf" > "$runtime/dsplr-xorg.conf"
  cat > "$runtime/xorg.conf.d/00-dsplr.conf" << 'CONF'
Section "Device"
    Identifier "dummy"
    Driver "dummy"
    VideoRam 256000
EndSection
Section "Monitor"
    Identifier "monitor"
    HorizSync 5.0-1000.0
    VertRefresh 5.0-200.0
EndSection
Section "Screen"
    Identifier "screen"
    Device "dummy"
    Monitor "monitor"
    DefaultDepth 24
    SubSection "Display"
        Depth 24
        Virtual 4096 2160
    EndSubSection
EndSection
Section "Module"
    Disable "glx"
EndSection
Section "ServerFlags"
    Option "AutoAddDevices" "true"
    Option "AutoEnableDevices" "true"
    Option "AutoAddGPU" "false"
EndSection
CONF
  modules=()
  [ -n "${DSPLR_E2E_XORG_MODULES:-}" ] && modules=(-modulepath "$DSPLR_E2E_XORG_MODULES")
  # Not root, Xorg takes config paths relative to its working directory.
  (cd "$runtime" && exec env -i PATH="$PATH" HOME="$home" \
    DBUS_SYSTEM_BUS_ADDRESS="$no_bus" \
    "$xorg" ":$number" -displayfd 3 -seat seat-dsplr-e2e \
    -config dsplr-xorg.conf -configdir xorg.conf.d \
    "${modules[@]}" -logfile "$runtime/xvfb.log" -nolisten tcp -noreset \
    3> "$runtime/display" > "$runtime/xorg.stderr" 2>&1) &
else
  env -i PATH="$PATH" HOME="$home" \
    Xvfb ":$number" -displayfd 3 -screen 0 1920x1080x24 -nolisten tcp -noreset \
    3> "$runtime/display" > "$runtime/xvfb.log" 2>&1 &
fi
xvfb_pid=$!
ready=""
for _ in $(seq 100); do
  ready=$(tr -dc '0-9' < "$runtime/display" 2>/dev/null)
  [ -n "$ready" ] && break
  kill -0 "$xvfb_pid" 2>/dev/null || break
  sleep 0.1
done
if [ "$ready" != "$number" ] || ! kill -0 "$xvfb_pid" 2>/dev/null; then
  echo "$server did not start on :$number (it reported :${ready:-nothing})" >&2
  tail -n 30 "$runtime/xvfb.log" "$runtime/xorg.stderr" >&2 2> /dev/null
  exit 1
fi
if [ "$server" = xorg ]; then
  xorg_log_check || exit 1
  # CVT's 1280x720 at 60 Hz, as `cvt 1280 720 60` prints it, on the outputs
  # the tests plug in.
  if ! (DISPLAY=":$number" xrandr --newmode 1280x720_60.00 74.50 1280 1344 1472 1664 \
    720 723 728 748 -hsync +vsync &&
    DISPLAY=":$number" xrandr --addmode DUMMY1 1280x720_60.00 &&
    DISPLAY=":$number" xrandr --addmode DUMMY2 1280x720_60.00); then
    echo "xrandr could not add a mode to DUMMY1 and DUMMY2" >&2
    exit 1
  fi
fi
# Listening, and nothing below :$first touched: else stop before any test.
x_guard_check || exit 1

export DISPLAY=":$number" XDG_RUNTIME_DIR="$runtime" HOME="$home"
export XDG_CONFIG_HOME="$runtime/config" XDG_STATE_HOME="$runtime/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_STATE_HOME"
# The e2e tests refuse to run without these, so they never reach a real X
# server when started some other way.
export DSPLR_E2E_X11="$runtime" DSPLR_E2E_X11_DISPLAY="$DISPLAY"
export DSPLR_E2E_X11_SERVER="$server"
# An X11 desktop: no Wayland compositor and no session bus to find.
unset WAYLAND_DISPLAY SWAYSOCK HYPRLAND_INSTANCE_SIGNATURE NIRI_SOCKET \
  DBUS_SESSION_BUS_ADDRESS XAUTHORITY
# In a session and process group of its own, so cleanup can end all of it.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== $server's log" >&2
  tail -n 40 "$runtime/xvfb.log" >&2
fi
exit "$status"
