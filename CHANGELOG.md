# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

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

- `dsplr list` prints each output's mode, position, scale, transform, adaptive
  sync and available modes; `dsplr list --json` prints the same layout in the
  versioned JSON form profiles will use (refresh in millihertz).
- `dsplr backend` prints what detection checked, the backend it found and that
  backend's capabilities.
- Two backends so far: Hyprland, and wlroots compositors (`wlr`). With no
  backend found, both commands exit with code 6; on targets other than Linux,
  detection says dsplr supports Linux only.
- The `wlr` backend reads, tests and applies layouts over
  `zwlr_output_manager_v1`: tested against sway, and meant for any compositor
  offering it (niri, river, labwc, Wayfire). Detection picks it after Hyprland,
  when `WAYLAND_DISPLAY` names a compositor that offers the protocol. It applies
  a whole layout in one configuration and supports custom modes, fractional
  per-output scale and, from protocol version 4, adaptive sync on or off.
  Mirroring, a primary output and colour settings are not in the protocol, and a
  layout asking for them is refused rather than applied without them. A
  configuration the compositor cancels because the outputs changed meanwhile is
  reported, not retried.
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
  description by whole words, as `dsplr set` names an output). A change that can
  hide or move a screen asks to be kept and reverts after `confirm.timeout_secs`
  without a yes; a detached watchdog reverts it even if dsplr dies first,
  waiting long enough for dsplr's own apply and revert to run to the backend's
  request timeouts first. `--yes` keeps it without asking, and `--dry-run`
  checks without applying. Whatever the desktop did not take as asked is listed
  after the apply.
- `dsplr restore --revert` goes back to the layout from before the last change.
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
  exits `3`.
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
  diff of what saving would change. The old file's catch-all `output = ""` rule,
  its rules for monitors not connected now, and its adaptive sync, bit depth and
  colour preset (which Hyprland does not report) are carried into the new file;
  the kept rules go before the saved ones, so they never win over them, and
  within each a rule whose description starts another's (`AOC Q27G4 SER1`,
  `AOC Q27G4 SER12`) goes first, since Hyprland uses the last rule matching a
  monitor and matches a description by its start. A `monitors.lua` that is a
  symbolic link is refused, naming its target, rather than replaced with a file.
- Reading a `monitors.lua` refuses `vrr = 3` and any setting beside
  `disabled = true` instead of reading them as `vrr = 2` and dropping the
  setting, and a `mirror = "desc:…"` rule is written back as it was read. Under
  a hyprlang `hyprland.conf` saving is refused, saying dsplr writes only a Lua
  config's `monitors.lua`; on wlroots compositors it is not supported yet; both
  exit `3`; `dsplr backend` now lists native persistence for Hyprland.
- Exit codes `3` (validation refused), `4` (the desktop refused) and `5`
  (reverted).
