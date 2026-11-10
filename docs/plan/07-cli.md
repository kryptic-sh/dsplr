# 07 — CLI

Everything the GUI does, for scripts and keybinds. `clap` derive, with
completions and a man page generated at release.

| Command                                                                                                                                    | Does                                                                                                                                                                                                                |
| ------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `dsplr`                                                                                                                                    | Open the GUI                                                                                                                                                                                                        |
| `dsplr list [--json]`                                                                                                                      | Outputs, modes and current state                                                                                                                                                                                    |
| `dsplr backend`                                                                                                                            | Which backend was detected, what was checked, its capabilities                                                                                                                                                      |
| `dsplr set <output> [--mode WxH@R] [--pos X,Y] [--scale S] [--transform T] [--vrr on/off] [--enable/--disable] [--mirror OUT/--no-mirror]` | Edit one output and apply, through the confirm flow                                                                                                                                                                 |
| `dsplr apply <FILE>`                                                                                                                       | Apply a whole layout from a JSON or TOML file                                                                                                                                                                       |
| `dsplr save [--force] [--yes] [--profile NAME]`                                                                                            | Persist the current state natively, or as a named profile                                                                                                                                                           |
| `dsplr restore [NAME] [--revert]`                                                                                                          | Apply the matching or named profile; `--revert` applies the last revert point                                                                                                                                       |
| `dsplr profile list [--json]`, `show NAME [--json]`, `rm NAME`                                                                             | Manage the profile store                                                                                                                                                                                            |
| `dsplr profile import kanshi [PATH] [--force]`                                                                                             | Import kanshi's profiles into the store ([05](05-persistence-and-profiles.md#importing-kanshi-profiles)); exits `1` when one is skipped or not written, saying why                                                  |
| `dsplr watch [--force]`                                                                                                                    | Hotplug profile daemon: restores the picked profile, kept without asking, whenever the connected outputs change ([05](05-persistence-and-profiles.md#hotplug-profiles)); refused on KDE and GNOME without `--force` |
| `dsplr identify`                                                                                                                           | Show the identify overlay without the main window, for its time or until a panel is clicked (needs a GUI session with panels: `3` where the window system has none, `1` where no window opens)                      |

Common flags: `--backend <name>`, `--yes` (skip the confirm prompt; the revert
point is still written), `--dry-run` (validate, and test where the backend has a
real test; on Hyprland, KDE and X11 it says it validated locally only),
`--json`.

`--backend <name>` is global: before or after the command, and on `dsplr` alone
for the GUI. It takes the names `dsplr backend` prints (`hyprland`, `kde`,
`gnome`, `wlr`, and `randr` or `x11`); another is a usage error (`2`) listing
them. Only the backend named is checked, in place of detection's order and its
stop at an unreachable Hyprland ([04](04-backends.md#detection)), and it is
never replaced by another: when it is not reachable here, dsplr says which
backend and what its check found, and exits `6`. `randr` says it is not written
yet (M7). Every command that finds a backend uses it, including `dsplr watch`
each time it reconnects, the GUI's backend thread, and the revert watchdog a
change starts, which is given the flag.

The confirm prompt on a terminal reads one line within the timeout; no TTY and
no `--yes` is an error, never a silent keep. `dsplr restore` with a profile runs
the same flow, so from an autostart it is `dsplr restore --yes`
([05](05-persistence-and-profiles.md#dsplrs-profile-store)).

Exit codes: `0` success, `1` other failure, `2` usage (clap's own code for a
usage error, `USAGE_CODE` in `clap_builder`), `3` validation refused (including
a layout the backend refuses before sending it, because the desktop's interface
cannot express it, and a save the backend cannot do there), `4` the desktop
refused, `5` reverted (timeout or declined), `6` no backend, `7` no profile for
the connected outputs (`dsplr restore` without a name; nothing changed). Named
in one enum in `apps/dsplr`.

`dsplr identify` reads the layout from the backend like the other commands (`6`
without one), then shows the GUI's identify panels and exits `0` once they
close. A window system without panels (GNOME, any without layer-shell) is `3`,
as a save the backend cannot do there is; no GUI session to open (no display, or
a window or GPU that cannot be opened, or crcbl's headless shell chosen with
`CRCBL_SHELL`, which shows nothing) is `1`, saying it needs one. With no display
at all, detection finds no backend first: `6`. No new exit code was added for
it. It takes no `--time`: the overlay's time is the GUI's (`IDENTIFY_TIME`), and
nothing asked for it to differ.

`dsplr watch` exits only when stopped or when it cannot go on: `0` on SIGINT or
SIGTERM, `3` on KDE or GNOME without `--force`, `6` when no backend is found at
the start, `1` when it cannot watch the desktop at the start or cannot reach it
again after it went away. A restore it runs does not end it: each one's outcome
is said on stderr.
