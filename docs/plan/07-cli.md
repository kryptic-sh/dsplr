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
| `dsplr watch [--force]`                                                                                                                    | Hotplug profile daemon: restores the picked profile, kept without asking, whenever the connected outputs change ([05](05-persistence-and-profiles.md#hotplug-profiles)); refused on KDE and GNOME without `--force` |
| `dsplr identify`                                                                                                                           | Show the identify overlay (needs a GUI session)                                                                                                                                                                     |

Common flags: `--backend <name>`, `--yes` (skip the confirm prompt; the revert
point is still written), `--dry-run` (validate, and test where the backend has a
real test; on Hyprland, KDE and X11 it says it validated locally only),
`--json`.

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

`dsplr watch` exits only when stopped or when it cannot go on: `0` on SIGINT or
SIGTERM, `3` on KDE or GNOME without `--force`, `6` when no backend is found at
the start, `1` when it cannot watch the desktop at the start or cannot reach it
again after it went away. A restore it runs does not end it: each one's outcome
is said on stderr.
