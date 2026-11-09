# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  writes it), matching its outputs to the connected ones by identity. A change
  that can hide or move a screen asks to be kept and reverts after
  `confirm.timeout_secs` without a yes; a detached watchdog reverts it even if
  dsplr dies first, waiting long enough for dsplr's own apply and revert to run
  to the backend's request timeouts first. `--yes` keeps it without asking, and
  `--dry-run` checks without applying. Whatever the desktop did not take as
  asked is listed after the apply.
- `dsplr restore --revert` goes back to the layout from before the last change.
- Exit codes `3` (validation refused), `4` (the desktop refused) and `5`
  (reverted).
