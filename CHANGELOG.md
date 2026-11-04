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
  mirroring, fractional and per-output scale, and gaps between outputs. No
  command applies one yet.
