# 07 — CLI

Everything the GUI does, for scripts and keybinds. `clap` derive, with
completions and a man page generated at release.

| Command                                                                                                                                    | Does                                                                          |
| ------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `dsplr`                                                                                                                                    | Open the GUI                                                                  |
| `dsplr list [--json]`                                                                                                                      | Outputs, modes and current state                                              |
| `dsplr backend`                                                                                                                            | Which backend was detected, what was checked, its capabilities                |
| `dsplr set <output> [--mode WxH@R] [--pos X,Y] [--scale S] [--transform T] [--vrr on/off] [--enable/--disable] [--mirror OUT/--no-mirror]` | Edit one output and apply, through the confirm flow                           |
| `dsplr apply <FILE>`                                                                                                                       | Apply a whole layout from a JSON or TOML file                                 |
| `dsplr save [--force] [--yes] [--profile NAME]`                                                                                            | Persist the current state natively, or as a named profile                     |
| `dsplr restore [NAME] [--revert]`                                                                                                          | Apply the matching or named profile; `--revert` applies the last revert point |
| `dsplr profile list`, `show NAME`, `rm NAME`                                                                                               | Manage the profile store                                                      |
| `dsplr watch`                                                                                                                              | Hotplug profile daemon (M8)                                                   |
| `dsplr identify`                                                                                                                           | Show the identify overlay (needs a GUI session)                               |

Common flags: `--backend <name>`, `--yes` (skip the confirm prompt; the revert
point is still written), `--dry-run` (validate, and test where the backend has a
real test; on Hyprland, KDE and X11 it says it validated locally only),
`--json`.

The confirm prompt on a terminal reads one line within the timeout; no TTY and
no `--yes` is an error, never a silent keep.

Exit codes: `0` success, `1` other failure, `2` usage (clap's own code for a
usage error, `USAGE_CODE` in `clap_builder`), `3` validation refused (including
a layout the backend refuses before sending it, because the desktop's interface
cannot express it, and a save the backend cannot do there), `4` the desktop
refused, `5` reverted (timeout or declined), `6` no backend. Named in one enum
in `apps/dsplr`.
