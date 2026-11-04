# dsplr plan — index

dsplr is a Rust display settings editor for Linux: an nwg-displays replacement
that arranges, sizes and configures monitors on Hyprland, the wlroots
compositors (sway, niri, river, labwc, Wayfire), KDE Plasma, GNOME and X11
window managers. The GUI is drawn with crcbl, as pikr and lyr are; a CLI does
everything the GUI does.

Section numbers are document identity. Build order is
[11-milestones.md](11-milestones.md).

| #   | Section                                                          | Covers                                                                                       |
| --- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 00  | [00-index.md](00-index.md)                                       | This list                                                                                    |
| 01  | [01-overview.md](01-overview.md)                                 | Goals, baseline, MVP scope, non-goals, decisions, naming                                     |
| 02  | [02-architecture.md](02-architecture.md)                         | Process, workspace crates, the apply/confirm/revert flow, prior art, dependencies            |
| 03  | [03-display-model.md](03-display-model.md)                       | Outputs, modes, the layout, transforms and scale, validation, snapping                       |
| 04  | [04-backends.md](04-backends.md)                                 | Detection, the backend seam, Hyprland, wlr-output-management, KDE, GNOME, XRandR             |
| 05  | [05-persistence-and-profiles.md](05-persistence-and-profiles.md) | Saving per desktop, managed files, dsplr's own profile store, `dsplr restore`, hotplug       |
| 06  | [06-gui.md](06-gui.md)                                           | The window, layout canvas, property panel, confirm dialog, identify overlay, **crcbl needs** |
| 07  | [07-cli.md](07-cli.md)                                           | Commands, output formats, exit codes                                                         |
| 08  | [08-platforms.md](08-platforms.md)                               | Per-desktop capability table                                                                 |
| 09  | [09-testing.md](09-testing.md)                                   | Unit, recorded replies, nested and headless compositors, manual checklists                   |
| 10  | [10-ci-and-release.md](10-ci-and-release.md)                     | CI checks now, release pipeline later                                                        |
| 11  | [11-milestones.md](11-milestones.md)                             | Build order and exit checks                                                                  |
| 12  | [12-open-questions.md](12-open-questions.md)                     | Provisional decisions awaiting the owner's call                                              |
