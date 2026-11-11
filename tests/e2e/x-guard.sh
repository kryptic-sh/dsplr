# shellcheck shell=bash
# Sourced by every e2e harness: a check that the X displays below
# `x_guard_first` (:90) are left as the harness found them.
#
# Those are the machine's own: a desktop's Xwayland on :0, other sessions'
# test servers. An X server or a compositor's Xwayland takes the first
# display whose lock it cannot see a live process for, and unlinks that
# display's socket path before it binds its own (wlroots 0.20's
# `open_display_sockets`, Xtrans under Xvfb), then removes it on exit. A
# harness run in another PID namespace sees no process behind a live
# desktop's lock, so a headless server it starts can take :0 and delete the
# desktop's socket, as one did on 2026-11-12. So the harnesses start no
# Xwayland, and every harness records the sockets and locks below :90 when
# it starts and compares them once its servers are up and again at exit,
# failing loudly with `!!` when anything changed (another session's server
# there starting or stopping counts too).
#
#   source "$(dirname "${BASH_SOURCE[0]}")/x-guard.sh"   # records them
#   x_guard_check || exit 1                              # compares

x_guard_first=90

# Every socket in /tmp/.X11-unix and lock in /tmp for a display below
# `x_guard_first`, with its inode and change time.
x_guard_entries() {
  local path name number
  for path in /tmp/.X11-unix/X* /tmp/.X*-lock; do
    [ -e "$path" ] || [ -L "$path" ] || continue
    name=${path##*/}
    number=${name#.X}
    number=${number#X}
    number=${number%%[!0-9]*}
    [ -n "$number" ] && [ "$number" -lt "$x_guard_first" ] || continue
    stat -c '%n %i %Z' "$path"
  done | sort
}

# Fails loudly when a guarded entry changed since the harness sourced this.
x_guard_check() {
  local now
  now=$(x_guard_entries)
  if [ "$now" != "$x_guard_at_start" ]; then
    echo "!! the X sockets or locks of displays below :$x_guard_first changed while the harness ran:" >&2
    diff <(echo "$x_guard_at_start") <(echo "$now") >&2
    return 1
  fi
}

x_guard_at_start=$(x_guard_entries)
