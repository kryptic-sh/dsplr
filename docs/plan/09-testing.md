# 09 — Testing

## 1. Unit tests (most of the coverage)

- `dsplr-model`: logical-size maths for every transform and scale, validation
  problems one by one, snapping, normalisation, JSON round-trip.
- `dsplr-persist`: each writer against golden files; each reader round-trip; the
  hand-edit guard (a changed hash refuses to overwrite).
- `dsplr-core`: editor actions, undo, the confirm timer as a state machine with
  an injected clock, `DrawList` snapshots for a fixed layout. The snapshots are
  the draw list's text form (one line per fill, outline, text and hit region)
  compared with `crates/dsplr-core/src/snapshots/<name>.txt`; a change to the
  canvas that is meant is written back with
  `DSPLR_BLESS=1 cargo test -p dsplr-core`, which still fails each snapshot test
  it rewrote, so a stray `DSPLR_BLESS` can never pass a run; run again without
  it, and read the diff before the files are committed.
- `dsplr-backend`: each backend's translation layer against **recorded replies**
  — `hyprctl monitors all -j` output, `GetCurrentState` replies, RandR screen
  resources — kept as fixtures, including the owner's machine (eDP-1 disabled,
  HDMI-A-1 at 2560x1440@120). The xrandr CRTC algorithm is tested against
  recorded resources with known xrandr results.

## 2. Integration tests against real servers

Each runs in CI on ubuntu, in a private `XDG_RUNTIME_DIR`, as crcbl's and lyr's
Wayland suites do:

| Backend       | Server                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Wlr`         | `tests/e2e/headless-sway.sh`: headless sway (`WLR_BACKENDS=headless`, pixman renderer), private runtime, `HOME` and `XDG_CONFIG_HOME` (whose `sway/config` includes `outputs`), more outputs with `swaymsg create_output`; in CI in the same VM as Hyprland                                                                                                                                          |
| `Wlr` on niri | `tests/e2e/headless-niri.sh`: niri has no headless mode, so it runs nested (winit, Mesa's software EGL) in a headless sway whose windows float; private runtime, `HOME` and `XDG_CONFIG_HOME` (whose `niri/config.kdl` includes `outputs.kdl`, optionally); one output, `winit`, whose mode, transform and adaptive sync niri ignores; a second niri started on the saved file; in CI in the same VM |
| `Randr`       | Xorg with the `dummy` video driver, if its RandR support gives several outputs (checked at M7). Xvfb and Xephyr are not expected to exercise multi-output CRTC mode setting, and `xrandr --setmonitor` makes RandR monitors, not outputs                                                                                                                                                             |
| `Gnome`       | `mutter --headless --virtual-monitor` under a private D-Bus session                                                                                                                                                                                                                                                                                                                                  |
| `Kde`         | `tests/e2e/headless-kwin.sh`: `kwin_wayland --virtual --output-count 3`, three 1920x1080 outputs with one mode each and no EDID; private runtime, `HOME` and `XDG_CONFIG_HOME` (where KWin writes `kwinoutputconfig.json`, which the tests read); in CI in the same VM. Not run locally: the owner's machine has no `kwin_wayland`                                                                   |
| Hyprland      | `tests/e2e/headless-hyprland.sh`: nested in `kwin_wayland --virtual`, private runtime, `LIBSEAT_BACKEND=seatd`; in CI inside a KVM VM with a virtio GPU                                                                                                                                                                                                                                              |

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
- The crcbl rendering of the GUI beyond `DrawList` snapshots (the canvas, and
  the confirm dialog's, `dsplr_core::dialog`). `apps/dsplr/tests/gui_e2e.rs`
  drives the window in `tests/e2e/headless-sway.sh` (Vulkan through Mesa's
  lavapipe, keys through `wtype` with a held keyboard): an apply kept with
  Enter, one left to time out, one whose dsplr is killed mid-confirm, which its
  watchdog reverts, and an output plugged in (`swaymsg create_output`) while the
  window is open, which Tab then reaches, edited and applied with an edit made
  before it came, each read back with `dsplr list --json`. It is ignored and not
  in CI's matrix: the VM has neither lavapipe nor `wtype` yet. No click is sent
  to the window (no pointer injector), so dragging and the panel's widgets are
  covered by `dsplr-core`'s editor and hit-region tests, a build of the panel's
  tree, and the window's key and request tests against crcbl's headless shell,
  not by a pointer in a window. The apply flow every front end drives is
  unit-tested with a fake backend (`dsplr_core::flow`).
