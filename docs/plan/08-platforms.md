# 08 — Platforms

What each backend is planned to support. "?" is to verify in that backend's
milestone; the table is corrected then and the "?" removed.

| Feature             | Hyprland                  | wlroots (`Wlr`)                                                           | KDE Plasma 6                      | GNOME                                        | X11 (`Randr`)           |
| ------------------- | ------------------------- | ------------------------------------------------------------------------- | --------------------------------- | -------------------------------------------- | ----------------------- |
| Read state          | IPC                       | protocol                                                                  | protocol                          | D-Bus                                        | RandR                   |
| Live changes        | push                      | push                                                                      | push                              | signal                                       | events                  |
| Mode / refresh      | yes                       | yes                                                                       | yes                               | yes                                          | yes                     |
| Custom mode         | yes                       | yes                                                                       | not yet (KWin takes a mode list)  | no                                           | yes (new RandR mode)    |
| Position            | yes                       | yes                                                                       | yes                               | yes                                          | yes                     |
| Gaps allowed        | yes                       | yes                                                                       | no                                | no (logical monitors must touch)             | yes                     |
| Scale               | yes                       | yes                                                                       | yes                               | per-mode list                                | no (read-only `1.0`)    |
| Transform           | yes                       | yes                                                                       | yes                               | yes                                          | yes                     |
| Adaptive sync       | yes                       | v4 only                                                                   | yes (VRR policy, where it has it) | no (Mutter lists VRR as modes; not read yet) | read-only               |
| Mirroring           | via `eval`                | no                                                                        | yes (management v13 and later)    | yes                                          | yes                     |
| Test before apply   | no (stub always succeeds) | yes (niri's checks little: [04](04-backends.md#wlroots-compositors-wlr))  | no (no test request)              | yes (`verify`)                               | no                      |
| Persistence         | `monitors.lua`            | per compositor ([05](05-persistence-and-profiles.md#wlroots-compositors)) | by KWin, on every apply           | `monitors.xml`, after GNOME Shell's confirm  | dsplr store + autostart |
| Extra fields        | colour preset, bit depth  | —                                                                         | overscan, RGB range, priority     | primary monitor, RGB range, layout mode      | primary output          |
| Per-monitor dialogs | panels                    | panels                                                                    | panels                            | no (one toplevel; identify unsupported)      | panels                  |

Not supported, by decision: Windows, macOS, X11 without RandR 1.5, Wayland
compositors that offer none of the protocols above (Weston, Mir — dsplr says
which interfaces it looked for).
