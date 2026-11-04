# 07 — CLI

Everything the GUI does, for scripts and keybinds. `clap` derive, with
completions and a man page generated at release.

| Command                                                                                                                        | Does                                                                          |
| ------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------- |
| `dsplr`                                                                                                                        | Open the GUI                                                                  |
| `dsplr list [--json]`                                                                                                          | Outputs, modes and current state                                              |
| `dsplr backend`                                                                                                                | Which backend was detected, what was checked, its capabilities                |
| `dsplr set <output> [--mode WxH@R] [--pos X,Y] [--scale S] [--transform T] [--vrr on/off] [--enable/--disable] [--mirror OUT]` | Edit one output and apply, through the confirm flow                           |
| `dsplr apply <FILE>`                                                                                                           | Apply a whole layout from a JSON or TOML file                                 |
| `dsplr save [--profile NAME]`                                                                                                  | Persist the current state natively, or as a named profile                     |
| `dsplr restore [NAME] [--revert]`                                                                                              | Apply the matching or named profile; `--revert` applies the last revert point |
| `dsplr profile list`, `show NAME`, `rm NAME`                                                                                   | Manage the profile store                                                      |
| `dsplr watch`                                                                                                                  | Hotplug profile daemon (M8)                                                   |
| `dsplr identify`                                                                                                               | Show the identify overlay (needs a GUI session)                               |

Common flags: `--backend <name>`, `--yes` (skip the confirm prompt; the revert
point is still written), `--dry-run` (validate and test only), `--json`.

The confirm prompt on a terminal reads one line within the timeout; no TTY and
no `--yes` is an error, never a silent keep.

Exit codes: `0` success, `1` usage, `2` validation refused, `3` backend refused,
`4` reverted (timeout or declined), `5` no backend. Named in one enum in
`apps/dsplr`.
