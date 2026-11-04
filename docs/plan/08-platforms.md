# 08 — Platforms

What each backend is planned to support. "?" is to verify in that backend's
milestone; the table is corrected then and the "?" removed.

| Feature             | Hyprland                   | wlroots (`Wlr`)                                                           | KDE Plasma 6                      | GNOME                                       | X11 (`Randr`)           |
| ------------------- | -------------------------- | ------------------------------------------------------------------------- | --------------------------------- | ------------------------------------------- | ----------------------- |
| Read state          | IPC                        | protocol                                                                  | protocol                          | D-Bus                                       | RandR                   |
| Live changes        | push                       | push                                                                      | push                              | signal                                      | events                  |
| Mode / refresh      | yes                        | yes                                                                       | yes                               | yes                                         | yes                     |
| Custom mode         | yes                        | yes                                                                       | in KWin 6.8, needs newer bindings | no                                          | yes (new RandR mode)    |
| Position            | yes                        | yes                                                                       | yes                               | yes                                         | yes                     |
| Gaps allowed        | yes                        | yes                                                                       | no                                | ?                                           | yes                     |
| Scale               | yes                        | yes                                                                       | yes                               | per-mode list                               | no (read-only `1.0`)    |
| Transform           | yes                        | yes                                                                       | yes                               | yes                                         | yes                     |
| Adaptive sync       | yes                        | v4 only                                                                   | yes (policy)                      | ?                                           | read-only               |
| Mirroring           | via `eval` (M2 exit check) | no                                                                        | in KWin 6.8, needs newer bindings | yes                                         | yes                     |
| Test before apply   | no (stub always succeeds)  | yes                                                                       | no (no test request)              | yes (`verify`)                              | no                      |
| Persistence         | `monitors.lua`/`.conf`     | per compositor ([05](05-persistence-and-profiles.md#wlroots-compositors)) | by KWin, likely on every apply    | `monitors.xml`, after GNOME Shell's confirm | dsplr store + autostart |
| Extra fields        | colour preset, bit depth   | —                                                                         | overscan, RGB range, priority     | primary monitor                             | primary output          |
| Per-monitor dialogs | panels                     | panels                                                                    | panels                            | no (toplevel; Shell confirm on save)        | panels                  |

Not supported, by decision: Windows, macOS, X11 without RandR 1.5, Wayland
compositors that offer none of the protocols above (Weston, Mir — dsplr says
which interfaces it looked for).
