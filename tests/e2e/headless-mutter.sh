#!/bin/bash
# Runs "$@" against a headless Mutter that cannot touch the desktop session.
#
#   tests/e2e/headless-mutter.sh cargo test -p dsplr-backend --test mutter_e2e -- --ignored --test-threads=1
#
# - Mutter's headless backend (`mutter --headless`): no window, no DRM, no
#   seat and no logind session (`meta_backend_native_create_launcher` takes
#   none in that mode, Mutter 51.1). `--virtual-monitor WxH` adds a monitor
#   named Meta-0 and on, each with that one mode at 60 Hz, vendor
#   `MetaVendor`, product `MetaVirtualMonitor` and serial 0x00 and on
#   (`src/core/meta-context-main.c`, `src/backends/native/
#   meta-output-virtual.c`). `--no-x11` keeps Xwayland from starting: a
#   Mutter that is not a systemd user unit otherwise starts it at once.
# - A D-Bus session bus of its own (`dbus-run-session`), which Mutter owns
#   `org.gnome.Mutter.DisplayConfig` on and the tests talk to, so the
#   session's bus is never reached. Mutter still finds the system bus, as
#   any program does; on a headless backend it asks it nothing that changes
#   the machine (no session is taken). GSettings come from memory
#   (`GSETTINGS_BACKEND=memory`), so dconf is not started.
# - Mutter and the bus run in a private XDG_RUNTIME_DIR with `env -i`, so no
#   socket of the session is reachable, and dsplr's detection cannot find
#   the live desktop. Mutter writes `monitors.xml` in $XDG_CONFIG_HOME on a
#   `persistent` apply: a private one, shared with the tests, which read it
#   there. Mutter and the tests get a private HOME too. cargo and rustup
#   keep their real homes through CARGO_HOME and RUSTUP_HOME, pinned before
#   HOME changes.
# - Mutter alone, without GNOME Shell, keeps a `persistent` apply at once:
#   its default plugin has no `confirm_display_change`, and
#   `meta_plugin_manager_confirm_display_change` then confirms it
#   (`src/compositor/meta-plugin-manager.c`). Under GNOME Shell its dialog
#   asks first.
set -uo pipefail

# The first pass makes the private directories and starts the bus, running
# this script again inside it.
if [ -z "${_DSPLR_MUTTER_BUS_STARTED:-}" ]; then
  runtime=$(mktemp -d)
  chmod 700 "$runtime"
  mkdir "$runtime/home" "$runtime/config" "$runtime/data" "$runtime/cache"
  # Nothing of the caller's environment but what the command needs: no
  # session bus, Wayland or X11 display, or compositor socket of theirs.
  kept=()
  for name in DSPLR_BIN RUST_BACKTRACE RUSTUP_TOOLCHAIN CARGO_TARGET_DIR; do
    [ -n "${!name:-}" ] && kept+=("$name=${!name}")
  done
  exec env -i "${kept[@]}" \
    _DSPLR_MUTTER_BUS_STARTED=1 DSPLR_E2E_MUTTER="$runtime" \
    PATH="$PATH" HOME="$runtime/home" TERM="${TERM:-dumb}" \
    CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}" RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" \
    XDG_RUNTIME_DIR="$runtime" XDG_CONFIG_HOME="$runtime/config" \
    XDG_DATA_HOME="$runtime/data" XDG_CACHE_HOME="$runtime/cache" \
    XDG_STATE_HOME="$runtime/state" \
    dbus-run-session -- "$0" "$@"
fi
unset _DSPLR_MUTTER_BUS_STARTED
runtime="$DSPLR_E2E_MUTTER"
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  echo "dbus-run-session gave no bus" >&2
  exit 1
fi

# The facts the tests rest on were read in Mutter 51.1's source; say which
# Mutter this is, so a failure after an update shows it.
version=$(timeout 10 mutter --version 2>/dev/null | head -n 1)
echo "== ${version:-mutter --version said nothing}" >&2

cleanup() {
  # The command's whole process group: it, and anything it started that
  # outlived it.
  [ -n "${command_pid:-}" ] && kill -- "-$command_pid" 2>/dev/null
  kill "${mutter_pid:-}" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# The monitors the tests expect, in order; they check them. Two of one size,
# so one can mirror the other, and one of another.
monitors=(1920x1080 1920x1080 1280x720)
args=()
for monitor in "${monitors[@]}"; do
  args+=(--virtual-monitor "$monitor")
done
env -i HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="$runtime" \
  XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_DATA_HOME="$XDG_DATA_HOME" \
  XDG_CACHE_HOME="$XDG_CACHE_HOME" GSETTINGS_BACKEND=memory \
  DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  mutter --headless --wayland --no-x11 --wayland-display wayland-0 "${args[@]}" \
  > "$runtime/mutter.log" 2>&1 &
mutter_pid=$!
for _ in $(seq 100); do
  [ -S "$runtime/wayland-0" ] && break
  kill -0 "$mutter_pid" 2>/dev/null || break
  sleep 0.1
done
if ! [ -S "$runtime/wayland-0" ]; then
  echo "Mutter did not start" >&2
  tail -n 30 "$runtime/mutter.log" >&2
  exit 1
fi

# Detection tries KWin's protocols on the Wayland socket first, as on a real
# GNOME session, then finds Mutter on the bus.
export WAYLAND_DISPLAY=wayland-0
export DSPLR_E2E_MUTTER_OUTPUTS="${#monitors[@]}"
# The bus the tests check they talk to: the harness's, never the session's.
export DSPLR_E2E_MUTTER_BUS="$DBUS_SESSION_BUS_ADDRESS"
# In a session and process group of its own, so cleanup can end all of it.
# A background job of a script is no group leader, so setsid runs it in
# place and its pid names the group.
setsid "$@" &
command_pid=$!
wait "$command_pid"
status=$?
if [ "$status" -ne 0 ]; then
  echo "== Mutter's log" >&2
  tail -n 40 "$runtime/mutter.log" >&2
  if [ -f "$XDG_CONFIG_HOME/monitors.xml" ]; then
    echo "== monitors.xml" >&2
    cat "$XDG_CONFIG_HOME/monitors.xml" >&2
  fi
fi
exit "$status"
