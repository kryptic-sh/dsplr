# 08 — Platforms

What each backend is planned to support. "?" is to verify in that backend's
milestone; the table is corrected then and the "?" removed.

| Feature           | Hyprland                 | wlroots (`Wlr`)                                                           | KDE Plasma 6                  | GNOME           | X11 (`Randr`)           |
| ----------------- | ------------------------ | ------------------------------------------------------------------------- | ----------------------------- | --------------- | ----------------------- |
| Read state        | IPC                      | protocol                                                                  | protocol                      | D-Bus           | RandR                   |
| Live changes      | push                     | push                                                                      | push                          | signal          | events                  |
| Mode / refresh    | yes                      | yes                                                                       | yes                           | yes             | yes                     |
| Custom mode       | yes                      | yes                                                                       | ?                             | no              | yes (new RandR mode)    |
| Position          | yes                      | yes                                                                       | yes                           | yes             | yes                     |
| Scale             | yes                      | yes                                                                       | yes                           | per-mode list   | no (read-only `1.0`)    |
| Transform         | yes                      | yes                                                                       | yes                           | yes             | yes                     |
| Adaptive sync     | yes                      | v4 only                                                                   | yes (policy)                  | ?               | read-only               |
| Mirroring         | yes (IPC)                | no                                                                        | ?                             | yes             | yes                     |
| Test before apply | yes (protocol)           | yes                                                                       | ?                             | yes (`verify`)  | no                      |
| Persistence       | `monitors.lua`/`.conf`   | per compositor ([05](05-persistence-and-profiles.md#wlroots-compositors)) | by KWin                       | `monitors.xml`  | dsplr store + autostart |
| Extra fields      | colour preset, bit depth | —                                                                         | overscan, RGB range, priority | primary monitor | primary output          |

Not supported, by decision: Windows, macOS, X11 without RandR 1.5, Wayland
compositors that offer none of the protocols above (Weston, Mir — dsplr says
which interfaces it looked for).
