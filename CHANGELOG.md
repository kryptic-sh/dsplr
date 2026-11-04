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
- Hyprland is the only backend so far. With no backend found, both commands exit
  with code 6; on targets other than Linux, detection says dsplr supports Linux
  only.
- The Hyprland backend can apply a layout, as runtime monitor rules through
  Hyprland's `eval` (dropped on a config reload), so `dsplr backend` now lists
  mirroring, fractional and per-output scale, and gaps between outputs.
- `dsplr apply <FILE>` applies a layout from a JSON file (as `dsplr list --json`
  writes it), matching its outputs to the connected ones by identity. A change
  that can hide or move a screen asks to be kept and reverts after
  `confirm.timeout_secs` without a yes; a detached watchdog reverts it even if
  dsplr dies first. `--yes` keeps it without asking, and `--dry-run` only
  validates. Whatever the desktop did not take as asked is listed after the
  apply.
- `dsplr restore --revert` goes back to the layout from before the last change.
- Exit codes `3` (validation refused), `4` (the desktop refused) and `5`
  (reverted).
