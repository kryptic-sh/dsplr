# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- On GNOME and X11, where a mirror has no source of its own, the check after an
  apply compares which outputs mirror each other, not which of them is named the
  source, so a mirror the desktop reads back the other way round is no longer
  reported as something other than was asked. Hyprland and KDE, which keep the
  source, still compare it.
- The confirm dialogs and the identify overlay name each monitor's output by the
  connector the window system reports for it (crcbl 0.19.1's
  `MonitorInfo::connector`), rather than by its position, which stays the
  fallback where none is reported or the one reported names no output (Xvfb's
  `screen`, RandR 1.0's `default`). Two outputs at one place no longer both read
  as the first, and a monitor whose output the layout being confirmed turns off
  is named as none.
- The GUI's keys are read without Caps Lock (crcbl 0.19.1's `shortcut_keysym`):
  with Caps Lock on, `h j k l` nudge rather than move to the neighbour, `C-r`
  redoes, and the dialogs' `y` and `n` answer. dsplr builds on crcbl 0.19.1.

- Hyprland modes, from `hyprctl -j monitors` and from `monitors.lua`, are read
  by one stricter parser: the size is digits only, and the refresh is plain
  digits with at most one decimal point and at most one trailing `Hz`, so forms
  such as `1920x1080@1e2`, `@inf`, `@60HzHz` and `+1920x1080` are refused rather
  than read. The owner's recorded `hyprctl -j monitors all` reply still reads in
  full.
- Exit code `4` now means only that the desktop itself refused the layout. A
  layout dsplr refuses before sending, because the desktop's interface cannot
  express it (on wlroots compositors, a primary output, colour settings or
  automatic adaptive sync in a layout file), exits `3` like any validation
  refusal, and is refused at the test step, before a revert point is kept or a
  watchdog started, with `--dry-run` too. An output unplugged between reading
  the displays and applying is reported as the displays having changed (exit
  `4`, showing them as they are now), not as a refusal. `dsplr save` where it
  cannot save at all exits `3`. A change declined at the prompt whose revert
  then fails exits `1`, since the declined layout is still on screen; a
  `dsplr restore --revert` that cannot reach the desktop exits `1` rather than
  `4`.

### Added

- X11 is supported, through RandR 1.3 or later (crcbl-x11 0.19.1): i3, bspwm and
  other X window managers, found from `DISPLAY` when no Wayland compositor is,
  Plasma on X11 included. On GNOME on X11 Mutter is found first;
  `--backend randr` forces RandR there, and under Plasma's KScreen or Mutter
  dsplr's changes can fight the layout the desktop restores itself. dsplr reads
  each output (named by its EDID where it has a serial, else by its RandR name),
  its modes, rotation, reflection, mirrors and the primary output, and applies a
  layout as xrandr would: CRTCs picked by xrandr's own algorithm, the changes
  sent under a server grab, a mode the output does not list added, or made with
  CVT timings as `cvt` makes them, and a panning area the new mode no longer
  fits reset and said. The apply goes through the same test, confirm and revert
  watchdog as on every desktop (X has no test request, so the test step checks
  the plan only). `dsplr watch` and the GUI follow the server's output changes.
  `dsplr save` keeps the layout as a profile in dsplr's store and has it put
  back at login; where the session runs XDG autostart dsplr sets that up, and
  under i3 or bspwm it shows the line to add to their startup; desktops that put
  their own layout back at login (Plasma, Xfce, MATE, Cinnamon and others) are
  left to do so. Through Xwayland, applying and saving are refused: the
  compositor's own backend sets its outputs. A CRTC with a transform
  (`xrandr --scale`) is refused, with how to take it off. Tested on Xvfb and on
  Xorg's dummy driver with several outputs; not yet on real monitors.
- `dsplr watch` says when it reads the displays after a change and finds the
  same outputs connected
  (`dsplr: the displays changed; the same outputs are connected`), on every
  desktop; it restores nothing then, as before.
- On Hyprland, the colour preset and bit depth are read (`dsplr list --json`,
  the GUI) and set: the panel offers Hyprland's `cm` presets (`srgb`, `wide`,
  `edid`, `hdr`, `hdredid`, `auto`, `dcip3`, `dp3`, `adobe`) and 8 or 10 bits,
  an apply states them for a monitor whose change sets them (8 bits included,
  which turns 10 off again) and leaves the rule in force alone otherwise, and
  `dsplr save` writes them to `monitors.lua` only when not Hyprland's default
  (`srgb`, 8), or when `hyprland.lua` has its own rule for that monitor. What is
  read is what Hyprland drives: a preset the monitor cannot do reads back as the
  one it fell back to, which the read-back reports, and a format of neither 8
  nor 10 bits per channel is not read as either.
- A layout or profile from another desktop applies even where it names settings
  the desktop has not, when they are neutral: Hyprland's `srgb` preset and 8-bit
  depth, KDE's zero overscan and automatic RGB range, and adaptive sync off, are
  dropped on a desktop without them. Any other value is still refused (exit
  `3`). Adaptive sync is now set on Hyprland too (`vrr` 0, 1 or 2, from the
  panel, a file or a profile); Hyprland reports only whether it is active, so it
  is not read back.
- `--backend <name>` (`hyprland`, `kde`, `gnome`, `wlr`; `randr`/`x11` says it
  is not written yet) uses that backend instead of detecting one, on every
  command and the GUI. Only that backend is checked: when it is not reachable,
  dsplr says what its check found and exits `6` rather than use another. A
  change's revert watchdog and `dsplr watch`'s reconnects keep it.
- `dsplr profile import kanshi [PATH] [--force]` imports kanshi's profiles into
  dsplr's store, from kanshi's own config (`$XDG_CONFIG_HOME/kanshi/config`,
  else `~/.config/kanshi/config`) unless a path is given: each named profile
  under its name (an anonymous one as `kanshi-N`), output defaults, aliases and
  `include` followed as kanshi follows them. A `make model serial` criteria
  becomes the monitor's EDID identity when it is connected with a serial, else a
  description; what kanshi leaves to an output (a setting not given, a mode
  without a refresh, `mode preferred`) is taken from it as it is now, each
  choice said. A profile dsplr cannot hold (`output *`, `...output`, a
  description glob other than a trailing `*`) is skipped with why, `exec` is not
  run, and a profile already in the store is replaced only with `--force`;
  either makes the exit `1`.
- `dsplr profile import autorandr [PATH] [--force]` imports autorandr's profiles
  into dsplr's store, from every directory autorandr loads them from (each of
  `$XDG_CONFIG_DIRS` with `/autorandr`, then `~/.autorandr`, else
  `$XDG_CONFIG_HOME/autorandr`) unless a directory of profiles, or one
  profile's, is given; a link to a profile kept elsewhere is imported, one that
  autorandr reads as another name for a profile is not. Each output is named by
  the EDID in its `setup` line (its make, model and serial, the make as the
  EDID's three-letter PNP code), or by its connector when the EDID gives no
  serial or there is none to read (autorandr's
  `--CONNECTED-BUT-EDID-UNAVAILABLE--OUTPUT`, a `*` pattern, an older
  autorandr's MD5), which is said. Until dsplr maps PNP codes to the company
  names wlroots and Hyprland report, a profile naming a monitor by its EDID does
  not match it there; it matches where the make is the PNP code (GNOME's Mutter
  reports it so, as will dsplr's coming X11 backend). An output named by its
  connector carries the X driver's name (`HDMI-1`, `HDMI1`, `HDMI-A-0`), which a
  Wayland desktop may name otherwise (`HDMI-A-1`), so such a profile matches
  there only where the names agree. `mode`, `rate`, `pos`, `rotate` and
  `reflect`, `primary`, `off` and `same-as` are taken, and outputs autorandr
  puts at one place become mirrors; `crtc`, `gamma`, `panning`, `filter`,
  properties and any other option are left out, said per output; a `scale` or
  `transform` other than 1, a relative position or `auto` skips the profile with
  why, as does anything that makes autorandr fail to load its profiles (it then
  loads none; dsplr skips just that one). Hook scripts are not run, and a
  profile already in the store is replaced only with `--force`; a skipped or
  unwritten profile makes the exit `1`.
- `dsplr watch`, the hotplug profile daemon: it stays running and, each time the
  set of connected outputs changes (and once when it starts), applies the
  profile `dsplr restore` would pick for them, kept without asking as
  `dsplr restore --yes` keeps one, with the revert point and watchdog every
  change has. Mode, position and other changes, its own applies included,
  restore nothing; bursts of changes (a dock) are read once they are quiet, or
  after a few seconds at most; a profile already on screen is not applied again;
  with no profile for the outputs it says so and changes nothing. While another
  change waits to be kept (a `dsplr apply` at its prompt, the GUI's confirm
  dialog) it does not restore over it, and restores once that change is kept or
  reverted. When the compositor goes away it reconnects with backoff, giving up
  on a watch that cannot stay open, and SIGINT or SIGTERM stops it (exit `0`),
  after a restore under way has finished. On KDE Plasma and GNOME, which restore
  their own layouts, it is refused (exit `3`) unless `--force`. Start it from
  the compositor's autostart, for example `exec dsplr watch` in sway's config;
  its messages go to stderr.
- The revert point (`$XDG_STATE_HOME/dsplr/revert.json`) now also records, while
  its change waits to be kept, when its watchdog reverts it
  (`revert_guarded_until_unix_ms`); the field is removed once the change is
  kept.
- `dsplr` with no command opens the GUI: a window showing every enabled output
  as a rectangle at its logical size, scaled to fit, labelled with its
  connector, description and mode, with disabled outputs in a tray below and the
  selected output's settings in a panel beside it. Changes are pending until
  applied, drawn as outlines over the live layout, and every one can be undone
  (`u`) and redone (`C-r`): drag an output (it snaps to the others' edges, and
  an output left at a negative position moves the layout just clear of it on
  release, and no other output moves otherwise), move it with `h j k l` or up to
  its neighbour with `H J K L`, rotate it with `r` / `R`, step its scale with
  `+` / `-`, turn it on or off with `space` (turned on where it would overlap
  another output or touch none, it goes to the right of them), or set its mode,
  refresh, custom mode (where the desktop takes them), scale, transform,
  adaptive sync and mirror source in the panel, and, where the desktop has them,
  the primary output (KDE, GNOME; making one primary takes it from the other,
  and an output turned off stops being primary), overscan (KDE) and RGB range
  (KDE, GNOME). A setting the desktop cannot take is disabled, with why as its
  tooltip. The pending layout is checked as it changes: an overlap is outlined
  in red, a gap in yellow (on the outputs apart from the largest group, so the
  one dragged away) and a second primary output in red, with the problem in the
  bar. A primary flag on an output that is off is ignored on KDE, as on GNOME,
  rather than refused. `Tab` and `Shift+Tab` step through the outputs, a click
  selects one, and `q` or `Escape` quits, asking first when edits are pending.
  `p` (or `F6`) takes the keyboard into the panel, where `Tab` / `S-Tab` move
  between its widgets, the arrows move and choose, Enter or space works one, and
  `Escape` gives the keys back to the canvas; `m` shows the last few messages
  from the backend over the canvas, which the bar shows one at a time. It reads
  the displays on a backend thread before the window opens, so a desktop with no
  backend exits `6` with no window; no window system or no usable GPU exits `1`
  with the reason. `a` (or Apply) applies the pending edits through the same
  confirm flow as `dsplr apply`: tested, guarded by the revert point and its
  watchdog, read back, and kept only once "Keep these display settings?" is
  answered on one of the dialogs shown on every monitor (Enter keeps; Esc, the
  countdown, or dsplr dying reverts; the keys work in the dialog that has the
  keyboard and in the window, and while the change waits the window's title is
  "dsplr: keep these display settings?"). `s` (or Save) saves the displays where
  the desktop reads them, as `dsplr save` does, asking in the window before
  replacing a file dsplr did not write or one edited by hand. The panel also
  saves the displays as a named profile, restores a profile through the same
  flow, goes back to the revert point, and reads the displays again.
- The GUI follows the desktop: an output plugged in, unplugged, turned on or off
  or changed by another program shows up in the window as it happens (on
  Hyprland, a monitor added or removed and a config reload; a rule changed with
  `hyprctl eval` or `keyword` shows on Refresh). Pending edits survive it
  setting by setting: what you changed keeps your value (the bar says so when
  the desktop changed it too), everything else follows the desktop, and edits to
  an output that went away are dropped, which the bar also says. A drag goes on
  through it. If the window stops being able to follow the desktop, the bar says
  so and it tries again, waiting longer each time (from one second up to thirty)
  until it gives up; Refresh still reads the desktop meanwhile.
- `i` (or Identify) shows each output's connector and description in large text
  on that output for a few seconds. Where the window system has no panels, the
  bar says identify is unsupported there. `dsplr identify` shows the same panels
  without opening the window, until their time is up or one is clicked; it exits
  `3` where the window system has no panels and `1` without a GUI session.
  Neither it nor the GUI runs on crcbl's headless shell
  (`CRCBL_SHELL=headless`), where nothing would be seen: both exit `1` saying
  so.
- The GUI's keys are configurable in `config.toml`'s `[keys]` table, one list of
  keys per action (`nudge_left = ["Left", "h"]`, `redo = ["C-y"]`); an action
  left out keeps its default, except a default key the file binds to another
  action, which goes to that action with a warning (printed by the CLI; in the
  GUI's bar and message log). An unknown action, an unknown key name or one key
  the file binds to two actions is an error naming it. The bar's key hint
  follows the configured keys.
- The GUI writes in the system's sans-serif font, with its fallbacks for
  characters it lacks, and in its own built-in font where the system has none.
- `dsplr list` prints each output's mode, position, scale, transform, adaptive
  sync and available modes (a refresh the desktop did not report reads as
  `refresh unknown`, not `0 Hz`); `dsplr list --json` prints the same layout in
  the versioned JSON form profiles will use (refresh in millihertz).
- `dsplr backend` prints what detection checked, the backend it found and that
  backend's capabilities.
- Four backends so far: Hyprland, KDE Plasma 6 on Wayland (`kde`), GNOME
  (`gnome`), and wlroots compositors (`wlr`). With no backend found, both
  commands exit with code 6; on targets other than Linux, detection says dsplr
  supports Linux only.
- The `gnome` backend reads, tests and applies layouts over Mutter's
  `org.gnome.Mutter.DisplayConfig` on the session bus
  (`DBUS_SESSION_BUS_ADDRESS`, else the `bus` socket in `XDG_RUNTIME_DIR`).
  Detection tries it after KDE and before wlroots, when something owns that name
  on the bus (asked of the bus, so nothing is started) and it is this session's
  Mutter: the process serving `WAYLAND_DISPLAY`'s socket, or, with no Wayland
  socket, a session whose `XDG_CURRENT_DESKTOP` names GNOME. So another
  session's GNOME on the user's shared bus, or GNOME around a nested compositor,
  is not taken for this one. It reads every monitor, its modes and the scales
  each mode takes, and sets mode, position, scale (one of the mode's, each
  output its own unless Mutter requires one for all), transform, enabled, the
  primary output, RGB range and mirroring (monitors sharing a logical monitor).
  `dsplr apply --dry-run` has Mutter test the layout (`verify`), and an apply is
  `temporary`, which Mutter neither saves nor reverts, so dsplr's confirm and
  watchdog guard it as on other desktops. Mutter's own reason for a refusal is
  shown (exit `4`); a change made between reading the displays and testing or
  applying is reported as the displays having changed (exit `4`), nothing sent.
  A mode the monitor does not list, a mirror whose mode differs in size from its
  source's and a second primary output are refused before Mutter is asked (exit
  `3`), and validation refuses gaps, which Mutter does not take. Mutter needs
  the outputs to start at 0,0, so on GNOME the window moves the layout there
  after every edit and drag (outputs that are off stay where they are, so no
  edit makes a change that shows nowhere; mirrors stay on their sources), and
  `dsplr apply`, `set` and `restore` move the layout they apply there first and
  say how far; `dsplr backend` lists it as "starts at the origin". With no
  output that is on named primary (turning the primary one off), Mutter's
  primary stays, else the first output left takes it, and the read-back does not
  count that as a difference. A monitor's underscanning and colour mode are sent
  back as they are, since Mutter resets them otherwise. Mutter's physical layout
  mode, where an output is as big as its mode whatever its scale, is read and
  kept: a layout from GNOME carries it (`layout_mode` in the JSON form, written
  only where the desktop has one), the canvas and validation size outputs by it,
  and `dsplr list` says so. A layout file that names no mode keeps GNOME's; one
  naming another switches it, which the confirm asks about and the read-back
  reports when it did not take.
- `dsplr save` on GNOME sends the layout on screen to Mutter again as
  `persistent`: GNOME Shell asks whether to keep the display settings, and
  Mutter writes `monitors.xml` only once they are kept (Mutter alone keeps them
  at once). dsplr says it handed the layout over, since the answer is the
  desktop's. A layout other than the one on screen is refused, as on niri.
- The `kde` backend reads and applies layouts over KWin's own protocols:
  `kde_output_device_v2` through `kde_output_device_registry_v2` (version 21 or
  later, which KWin 6.8 requires), `kde_output_order_v1` for the primary output,
  and `kde_output_management_v2`. Detection tries it after Hyprland and before
  wlroots, when `WAYLAND_DISPLAY` names a compositor that offers KWin's output
  management. It applies a whole layout in one configuration: mode, position,
  fractional per-output scale, transform, enabled, adaptive sync as KWin's VRR
  policy (off, on or automatic), overscan, RGB range, the primary output (the
  first in KWin's output order) and, from management version 13, mirroring.
  Adaptive sync, overscan and RGB range are read and set only on outputs whose
  capabilities say they have them. KWin's own reason for a refusal is shown
  (`KWin refused the configuration: Position of enabled output … is negative`).
  KWin has no test request, so `dsplr apply --dry-run` validates locally and
  checks the layout can be sent, then says KWin cannot test it; and its protocol
  allows no gaps between outputs, so validation refuses one. A mode the output
  does not list is set as a custom mode where KWin takes them (management
  version 18, outputs that say so): KWin is sent the output's custom mode list,
  the custom modes it already has kept in it, then the layout at the mode KWin
  made, whose refresh may differ a little from the one asked; elsewhere it is
  refused (exit `3`). KWin keeps scales in 120ths, so a scale reads back as that
  rounding carried it, and is not reported as not taken. While a change waits to
  be kept, the window's bar says KWin has already saved it.
- `dsplr save` on KDE writes nothing and says the layout is already saved: KWin
  saves every layout it applies in its `kwinoutputconfig.json`. A layout other
  than the one on screen is refused, as on niri.
- A scale travels as a `wl_fixed` (in 256ths) on `wlr` and `kde`, so one that is
  no 256th (`dsplr set … --scale 1.1`) reads back as the nearest one; the
  read-back after an apply now compares against the asked scale carried the same
  way (`Capabilities::scale_steps`), so it is not reported as not taken as
  asked. A compositor that rounds further (niri: 1.1 reads back as 1.09765625)
  is still reported.
- The `wlr` backend reads, tests and applies layouts over
  `zwlr_output_manager_v1`: tested against sway and niri, and meant for any
  compositor offering it (river, labwc, Wayfire). Detection picks it after
  Hyprland, when `WAYLAND_DISPLAY` names a compositor that offers the protocol.
  It applies a whole layout in one configuration and supports custom modes,
  fractional per-output scale and, from protocol version 4, adaptive sync on or
  off. Mirroring, a primary output and colour settings are not in the protocol,
  and a layout asking for them is refused rather than applied without them. A
  configuration the compositor cancels because the outputs changed meanwhile is
  reported, not retried. Only a configuration's first answer counts: niri 26.04
  follows `succeeded` with `cancelled` when the apply changed the outputs, and
  that change was applied.
- `dsplr apply` has the desktop test a layout before applying it where the
  backend has a real test (`wlr`), and says when it was the test that refused;
  `--dry-run` then reports the desktop's verdict, exiting `4` when it refuses.
  On Hyprland, whose test always succeeds, `--dry-run` still says it validated
  locally only. A test or apply that could not reach the desktop (a timeout, a
  lost connection) exits `1` rather than reporting a refusal.
- When `HYPRLAND_INSTANCE_SIGNATURE` is set but Hyprland's socket does not
  answer, detection stops there with no backend, rather than driving Hyprland as
  a wlroots compositor.
- The Hyprland backend can apply a layout, as runtime monitor rules through
  Hyprland's `eval` (dropped on a config reload), so `dsplr backend` now lists
  mirroring, fractional and per-output scale, and gaps between outputs.
- `dsplr apply <FILE>` applies a layout from a JSON file (as `dsplr list --json`
  writes it), matching its outputs to the connected ones by identity (a
  description by whole words, as `dsplr set` names an output), each to its own:
  a description naming several outputs takes the one the other entries leave,
  and twins with one make, model and serial keep the connectors they were saved
  on. A change that can hide or move a screen asks to be kept and reverts after
  `confirm.timeout_secs` without a yes; a detached watchdog reverts it even if
  dsplr dies first, waiting long enough for dsplr's own apply and revert to run
  to the backend's request timeouts first. `--yes` keeps it without asking, and
  `--dry-run` checks without applying. Whatever the desktop did not take as
  asked is listed after the apply.
- `dsplr restore --revert` goes back to the layout from before the last change.
- Profiles: `dsplr save --profile NAME` saves the current layout as a named
  profile in `$XDG_CONFIG_HOME/dsplr/profiles/NAME.json` on any desktop (the
  layout JSON `dsplr list --json` prints, plus its save time, so `dsplr apply`
  reads it too). `dsplr restore NAME` applies one, and `dsplr restore` alone
  picks the profile saved for exactly the connected outputs, else the most
  recently saved one for some of them, matching a monitor with a serial on any
  port and one without on its connector; both go through the confirm flow, and a
  profile output that is not connected is refused (exit `3`). With no profile to
  pick, nothing changes and it exits `7`. From an autostart, where there is no
  terminal, run `dsplr restore --yes`. Replacing a profile shows what changes
  and takes `--yes`, `--force` or a yes on the terminal. `dsplr profile list`
  (with `--json`), `dsplr profile show NAME` (as `dsplr list` prints a layout,
  or `--json`) and `dsplr profile rm NAME` manage the store. A name is ASCII
  letters and digits, spaces, `-`, `_` and `.`; anything else, `..`, a leading
  `.` or `-` or a trailing `.` or space is a usage error (exit `2`). `--revert`
  now cannot be combined with a profile name or the confirm flow's flags.
- `dsplr set <OUTPUT>` changes one output and applies the result through the
  same confirm flow as `dsplr apply` (validation, the desktop's test, the revert
  point and watchdog, `--yes`, `--dry-run`): `--mode WxH[@R]`, `--pos X,Y`,
  `--scale S`, `--transform T`, `--vrr off|on|automatic`, `--enable` or
  `--disable`, and `--mirror OUT` or `--no-mirror` to end a mirror. The output
  is named by its connector or its description (or its first whole words, when
  they name one output, so `AOC Q27G4 SER1` never names `AOC Q27G4 SER12`).
  `--mode WxH` keeps the current refresh where the output lists it at that size,
  else takes the highest there; a mode the output does not list is refused,
  naming the nearest ones, unless the desktop takes custom modes. Transform
  names are wl_output's, as `dsplr list` prints them, so `90` is sway's `270`;
  `--help` says so. It names the output it is setting before applying. A name
  for no output or several exits `2` listing the outputs; so do an output asked
  to mirror itself and a setting for an output that is off without `--enable`
  (or given with `--disable`), which no desktop would apply. A setting the
  desktop cannot take (a mode it does not list, an adaptive sync value it lacks)
  exits `3`; an adaptive sync value the desktop lacks is refused by validation,
  so a layout file, a profile and the GUI's canvas say so too. An output turned
  on with `--enable`, or out of a mirror with `--no-mirror`, without `--pos` is
  placed as the GUI places one: where it was if it touches the others without
  overlapping, else right of them, level with the topmost, which dsplr says
  (`placed HDMI-A-1 at 3840,0, beside the others; --pos to choose`).
- `dsplr save` makes the current layout last on Hyprland with a Lua config: it
  writes a dsplr-managed `monitors.lua` beside the `hyprland.lua` Hyprland reads
  (found through `HYPRLAND_CONFIG`, `$XDG_CONFIG_HOME/hypr` or `~/.config/hypr`,
  as Hyprland looks), and prints the `require("monitors")` line `hyprland.lua`
  needs, to go after its own `hl.monitor` calls, when it does not seem to have
  it, or warns when `hl.monitor` calls after it override the saved file; dsplr
  never edits `hyprland.lua`. A `monitors.lua` edited by hand since dsplr wrote
  it, or holding settings dsplr cannot keep (including `vrr = 3` and any setting
  beside `disabled = true`), is not overwritten without `--force`; one another
  tool wrote (nwg-displays') is replaced after a yes on the terminal or `--yes`,
  and with no terminal and no `--yes` nothing is saved. Each refusal shows the
  diff of what saving would change. The old file's catch-all `output = ""` calls
  (every one, in order, since Hyprland merges them; dsplr says it kept them and
  never writes one of its own), its rules for monitors not connected now, and
  its adaptive sync, bit depth and colour preset (which Hyprland does not
  report) are carried into the new file; the kept rules go before the saved
  ones, so they never win over them, and within each a rule whose description
  starts another's (`AOC Q27G4 SER1`, `AOC Q27G4 SER12`) goes first, since
  Hyprland uses the last rule matching a monitor and matches a description by
  its start. A `monitors.lua` that is a symbolic link is refused, naming its
  target, rather than replaced with a file.
- Reading a `monitors.lua` refuses `vrr = 3` and any setting beside
  `disabled = true` instead of reading them as `vrr = 2` and dropping the
  setting, and a `mirror = "desc:…"` rule is written back as it was read. Under
  a hyprlang `hyprland.conf` saving is refused, saying dsplr writes only a Lua
  config's `monitors.lua`; on wlroots compositors other than sway and niri it is
  not supported, and the message points at `dsplr save --profile NAME` and
  `dsplr restore --yes` from the compositor's autostart; both exit `3`;
  `dsplr backend` now lists native persistence for Hyprland, sway and niri.
- `dsplr save` makes the current layout last on sway: it writes a dsplr-managed
  `outputs` file beside the config sway reads (`~/.sway/config`,
  `$XDG_CONFIG_HOME/sway/config`, `~/.i3/config` or
  `$XDG_CONFIG_HOME/i3/config`, as sway looks), one `output` block per output
  with `enable` or `disable`, `mode`, `position`, `scale`, `transform` and, when
  known, `adaptive_sync`, and prints the `include outputs` line the config needs
  when it does not seem to have it. Transforms are written in sway's own names,
  which turn the quarter turns, flipped or not, the other way from
  `dsplr list`'s. A monitor with a serial is named by sway's identifier
  (`"Make Model Serial"`), so its settings follow it to another port; one
  without, one sharing its identifier with another connected monitor, or one
  whose identifier sway could not read back from a config (a quote, a backslash
  or a `$` in it), by connector. sway gives an identifier's setting precedence
  over a name's at startup wherever each is, and the last setting precedence on
  a reload, so the saved file is laid out to win both ways: a command for a
  connected output, by its connector or its identifier, is dropped from the old
  file, and dsplr says when the config's own `output` commands would still win,
  with the fix for each: commands after the include win on a reload (move the
  include below them); a command naming an output by identifier while the file
  names it by connector wins at startup wherever it is (remove it, or name the
  output by connector there), which is reported even before the config includes
  the file. As on Hyprland, a file edited by hand needs `--force`, another
  tool's needs a yes or `--yes`, a link is refused, and every refusal shows the
  diff; a file in dsplr's form keeps its commands for outputs not connected now
  and its `output *` commands (said when kept), while one with settings dsplr
  does not write (`bg`, `power`, `scale_filter`) needs `--force`. A layout with
  a mirror, which sway has not, and saving on a wlroots compositor other than
  sway are refused with exit `3` before anything is written; it counts as sway
  only when the socket `SWAYSOCK` names answers sway's IPC and is served by the
  same process as the Wayland socket, so a stale `SWAYSOCK` under another
  compositor, or one a compositor nested in sway inherited, does not. That
  socket is asked only by `dsplr save` and `dsplr backend`, so one that hangs
  delays no other command.
- `dsplr save` makes the current layout last on niri: it writes a dsplr-managed
  `outputs.kdl` beside the `config.kdl` niri reads (`NIRI_CONFIG`, else
  `$XDG_CONFIG_HOME/niri` or `~/.config/niri`, as niri looks), one
  `output "<name>" { … }` node per output with `off`, or `mode` (`custom=true`
  for a mode the output does not list), `scale`, `transform`, `position` and,
  when on, `variable-refresh-rate`, and prints the `include "outputs.kdl"` line
  the config needs when it does not seem to have it (an `optional=true` include
  counts). niri uses the first node naming an output, so that line goes before
  the config's own `output` nodes, and dsplr says when one before it for a
  connected output wins over the saved file. niri reloads the file as soon as it
  is written, so saving also applies it; dsplr saves only the layout on screen
  (`dsplr save` saves what it reads), so nothing moves, and refuses any other
  (exit `3`). The config's own `output` nodes after the include, for a connected
  output, are hidden whole by the saved node once it is written: when they set
  anything dsplr does not write (`backdrop-color`, `focus-at-startup`, on-demand
  adaptive sync), saving lists them and asks first (a yes or `--yes`), even when
  there is no file yet; with no include yet, every such node counts, since the
  include goes before them. An output whose old saved node had
  `variable-refresh-rate on-demand=true` keeps it when adaptive sync reads as
  on, since wlr-output-management cannot say on demand. Transforms keep their
  names, which niri shares with `dsplr list`. A monitor with a serial is named
  by its identifier (`"Make Model Serial"`), others by connector; niri matches
  both ignoring case, and so does dsplr when it drops the old file's nodes for
  connected outputs and keeps the rest. As on sway, a file edited by hand needs
  `--force`, another tool's needs a yes or `--yes`, one with settings dsplr does
  not write (`backdrop-color`, `focus-at-startup`) needs `--force`, a link is
  refused, and a mirror is refused (exit `3`). It counts as niri only when the
  socket `NIRI_SOCKET` names answers niri's IPC and is served by the same
  process as the Wayland socket, so a compositor nested in niri, which inherits
  `NIRI_SOCKET`, does not write niri's config.
- `dsplr list --json` (and the revert point) now carry each output's `make`,
  `model` and `serial` as the desktop reported them, empty when it reported
  none; a layout file without them still reads.
- Exit codes `3` (validation refused), `4` (the desktop refused) and `5`
  (reverted).
