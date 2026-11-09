# 09 — Testing

## 1. Unit tests (most of the coverage)

- `dsplr-model`: logical-size maths for every transform and scale, validation
  problems one by one, snapping, normalisation, JSON round-trip.
- `dsplr-persist`: each writer against golden files; each reader round-trip; the
  hand-edit guard (a changed hash refuses to overwrite).
- `dsplr-core`: editor actions, undo, the confirm timer as a state machine with
  an injected clock, `DrawList` snapshots for a fixed layout.
- `dsplr-backend`: each backend's translation layer against **recorded replies**
  — `hyprctl monitors all -j` output, `GetCurrentState` replies, RandR screen
  resources — kept as fixtures, including the owner's machine (eDP-1 disabled,
  HDMI-A-1 at 2560x1440@120). The xrandr CRTC algorithm is tested against
  recorded resources with known xrandr results.

## 2. Integration tests against real servers

Each runs in CI on ubuntu, in a private `XDG_RUNTIME_DIR`, as crcbl's and lyr's
Wayland suites do:

| Backend  | Server                                                                                                                                                                                                                                                      |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Wlr`    | `tests/e2e/headless-sway.sh`: headless sway (`WLR_BACKENDS=headless`, pixman renderer), private runtime, `HOME` and `XDG_CONFIG_HOME` (whose `sway/config` includes `outputs`), more outputs with `swaymsg create_output`; in CI in the same VM as Hyprland |
| `Randr`  | Xorg with the `dummy` video driver, if its RandR support gives several outputs (checked at M7). Xvfb and Xephyr are not expected to exercise multi-output CRTC mode setting, and `xrandr --setmonitor` makes RandR monitors, not outputs                    |
| `Gnome`  | `mutter --headless --virtual-monitor` under a private D-Bus session                                                                                                                                                                                         |
| `Kde`    | `kwin_wayland --virtual` (lyr's notes record running Hyprland nested in it)                                                                                                                                                                                 |
| Hyprland | `tests/e2e/headless-hyprland.sh`: nested in `kwin_wayland --virtual`, private runtime, `LIBSEAT_BACKEND=seatd`; in CI inside a KVM VM with a virtio GPU                                                                                                     |

Each test reads, applies a changed layout, reads back, reverts and reads back.
`wlr-randr`, `kscreen-doctor`, `gnome-randr` and `xrandr` are used as
independent readers to cross-check that dsplr's view matches the server's.

Every integration test is shown to fail with the apply step stubbed out before
it is trusted.

## 3. Manual checklists

Per backend, on real hardware: hotplug a monitor while the GUI is open, apply a
mode the monitor rejects, let a confirm time out, kill dsplr mid-confirm and run
`dsplr restore --revert`, save and reboot.

## Not covered by automated tests (stated as a gap)

- Real monitors' behaviour on a mode they advertise but cannot show.
- The crcbl rendering of the GUI beyond `DrawList` snapshots.
