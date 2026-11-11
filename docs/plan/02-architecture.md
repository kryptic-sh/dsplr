# 02 — Architecture

## Process

One process. The GUI and the CLI share everything below the front end.

```
            main thread                                  backend thread
 ┌──────────────────────────────────┐            ┌──────────────────────────────────────┐
 │ crcbl-shell loop (Wayland / X11) │  wake +    │ the detected backend                 │
 │                                  │◄───────────┤  Wayland event queue · zbus signals  │
 │ editor state (dsplr-core)        │  Snapshot  │  · Hyprland socket2 · RandR events   │
 │   ──► DrawList ──► crcbl-ui      │            │                                      │
 │ pointer / keys ──► hit test      │  Request   │  read state · test · apply · revert  │
 │   ──► edit / Apply / Save ───────├───────────►│  · persist                           │
 └──────────────────────────────────┘            └──────────────────────────────────────┘
```

- The **main thread** owns the window, the editor state and the renderer. It
  never blocks on the display server.
- The **backend thread** owns the connection to the desktop. It sends a
  `Snapshot` (the full current state, [03](03-display-model.md)) on start and
  whenever the desktop reports a change, with a crcbl wakeup. It receives
  `Request`s (`Test(Layout)`, `Apply(Layout)`, `Revert`, `Persist(Layout)`) and
  answers each with a result.
- The backend thread runs a **current-thread tokio runtime** (the owner's
  decision, 2026-11-04). It drives the Wayland fd through `AsyncFd`, the
  Hyprland sockets and zbus, so there is one event loop shape for every backend.
- The **editor state is plain data**. `dsplr-core` turns
  `(snapshot, pending edits, theme)` into a `DrawList` and hit regions with no
  platform types, so the canvas logic, snapping and validation are tested
  without a display.
- The **CLI** uses the same backend and `Request`s synchronously and has no
  window.

As built (M3, 2026-11-10): the requests are `dsplr_core::link`'s (refresh,
apply, save, save as a profile, restore a profile, revert), and the backend
thread (`apps/dsplr/src/gui.rs`) reads the displays again after each one and
after each change the desktop reports (`Backend::watch`: a
`wlr-output-management` connection of its own on wlroots, the event socket on
Hyprland), waiting on both with `tokio::select!`. A watch that ends is opened
again with `dsplr watch`'s backoff (`watch::Backoff`: a wait from
`BACKOFF_FIRST`, doubling up to `BACKOFF_MAX`, given up after `RECONNECT_TRIES`,
started afresh by a watch that stayed up or reported a change), each step said
in the bar; meanwhile Refresh and every request still read the displays, and a
watch opened again reads them at once. The watch ends only when the compositor's
socket closes, which on Hyprland and the wlroots compositors usually means the
compositor went away, or on a protocol error, which a new connection may meet
again; the backoff bounds both. The CLI does not send `Request`s: it and the
backend thread drive the one apply flow (`dsplr_core::flow`, steps 1 to 6 below
as a state machine with no I/O) through `apps/dsplr/src/apply.rs`'s `drive`, and
differ only in who confirms (the terminal, the dialogs) and where the messages
go.

## Apply, confirm, revert

The safety net, shared by every backend and both front ends:

1. **Validate** the pending layout locally
   ([03](03-display-model.md#validation)). Refused here: no enabled output,
   overlapping outputs where the backend forbids it, a mode the output does not
   list (unless it is a custom mode on a backend that accepts them).
2. **Test** it where the backend really can: `wlr-output-management`'s `test` on
   wlroots compositors, GNOME's `verify` method. **Hyprland's `test` is a stub
   that always reports success** (it logs "TODO: STUB: applyTestConfiguration
   for test not implemented, returning true." in
   `src/protocols/OutputManagement.cpp`), and **KDE's management protocol has no
   test request** at all, so on both the step is skipped and the CLI's
   `--dry-run` says it validated locally only. A refusal is shown with the
   backend's reason and nothing changes.
3. **Remember** the current state as the revert point, write it to
   `$XDG_STATE_HOME/dsplr/revert.json`, start the **revert watchdog**, then
   **apply**.
4. **Confirm.** If the change can hide or move a screen (any mode, position,
   scale, transform or enable change), open a confirm prompt — in the GUI a
   dialog ([06](06-gui.md#confirm-dialog)), in the CLI a prompt on the terminal
   unless `--yes`. The timeout is a named constant (`CONFIRM_TIMEOUT`) with a
   config override.
5. **Revert** on timeout or on Escape. If dsplr itself dies or hangs, the
   watchdog reverts: a detached child process
   (`dsplr restore --revert --after <timeout>`) started in step 3, which applies
   `revert.json` unless the parent tells it the change was kept. A file alone
   would revert nothing, since a user with a blank screen cannot type a command
   to read it. `dsplr restore --revert` by hand is the last resort, and lands
   with the confirm flow in M2.
6. **Re-read** the state after the apply and compare it with what was asked for.
   KDE and Hyprland's `eval` path have no serial, so a change made by something
   else between the snapshot and the apply is only caught here; a difference is
   shown, never silently overwritten.
7. **Persist** only after confirmation, and only when the user asked to save.
   Apply without save is a valid, deliberate choice everywhere except where the
   desktop saves on every apply (KDE: KWin writes `kwinoutputconfig.json` on
   every successful apply, read in KWin 6.8.2's source; the GUI's bar says so
   while the change waits to be kept, and the watchdog's revert is what undoes
   it).

Revert on X11, wlroots and Hyprland is dsplr re-applying the old state.

**GNOME is different** (read in Mutter's `src/backends/meta-monitor-manager.c`):

- A `temporary` apply never reverts on its own and is not saved.
- A `persistent` apply from any D-Bus client starts Mutter's own confirmation:
  it emits `confirm-display-change`, GNOME Shell shows its own "Keep changes?"
  dialog, and Mutter saves `monitors.xml` only if the user keeps it, restoring
  the previous config on timeout (`DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`).
- So on GNOME dsplr applies `temporary` and runs its own confirm, and **Save**
  is a `persistent` apply whose confirm is GNOME Shell's dialog, not a second
  one of dsplr's.
- A stale serial is refused ("based on stale information"), and the whole D-Bus
  path can be disabled by policy ("Monitor configuration via D-Bus is
  disabled"); both are shown as backend refusals.

## Workspace layout

Follows the org convention of `apps/<bin>` + `crates/<bin>-*`.

| Path                   | Crate           | Owns                                                                                                                                                                                                           |
| ---------------------- | --------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `apps/dsplr`           | `dsplr`         | The binary: CLI parsing, logging, wiring backend + GUI or CLI together                                                                                                                                         |
| `crates/dsplr-model`   | `dsplr-model`   | The single `Layout` struct every other crate converts to and from ([03](03-display-model.md#one-struct-many-formats)): `Output`, `Mode`, `Transform`, scale rules, validation, snapping, the JSON form; no I/O |
| `crates/dsplr-backend` | `dsplr-backend` | Detection, the `Backend` enum and one module per backend (`hyprland/`, `wlr/`, `kde/`, `gnome/`, `randr/`)                                                                                                     |
| `crates/dsplr-persist` | `dsplr-persist` | Pure writers and readers for each native config format (Hyprland Lua, sway, niri KDL) and dsplr's own profile store                                                                                            |
| `crates/dsplr-core`    | `dsplr-core`    | Editor state, pending edits, undo, `DrawList` building, hit testing, the confirm timer as data                                                                                                                 |
| `crates/dsplr-gui`     | `dsplr-gui`     | crcbl-shell windows (main, identify overlays, confirm dialogs), the device and swapchains, translating `DrawList` into crcbl                                                                                   |
| `crates/dsplr-config`  | `dsplr-config`  | dsplr's own TOML settings (theme, keys, timeout, persistence mode), loaded through `hjkl-config` / `hjkl-xdg`                                                                                                  |

`dsplr-model`, `dsplr-persist`, `dsplr-core` and `dsplr-config` have **no
platform code**. That rule keeps the geometry, the config writers and the editor
testable on every CI runner without a display.

`dsplr-backend` is the crate most likely to sprawl. Each backend is a directory
from day one. The backends are an **enum**, not a trait object: the set is
closed and lives in one crate, as lyr's `lyr-wm` does. A backend that cannot do
something returns a typed `Unsupported { what, reason }` error, never a silent
no-op ([08](08-platforms.md)).

## Prior art

Read when choosing the design (2026-11-04). Nothing here is a dependency.

| Tool               | Desktops       | Took from it                                                                                  |
| ------------------ | -------------- | --------------------------------------------------------------------------------------------- |
| nwg-displays       | Hyprland, sway | The editing surface; the generated `monitors.lua` the owner runs today                        |
| wdisplays          | wlroots        | A canvas over `wlr-output-management` alone                                                   |
| kanshi             | wlroots        | Profile matching on the set of connected outputs; the profile file format as an import source |
| wlr-randr          | wlroots        | Cross-check in tests                                                                          |
| kscreen-doctor     | KDE            | Cross-check in tests                                                                          |
| gnome-randr(-rust) | GNOME          | How `ApplyMonitorsConfig` is driven from outside gnome-control-center                         |
| arandr, autorandr  | X11            | Canvas on RandR; autorandr's EDID-based profile matching                                      |

## Dependencies (initial)

Each is added with `cargo add` in the milestone that first uses it, not at
scaffold time. Anything not on this list needs a call before it is added.

| Crate                                                              | Used by            | Notes                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crcbl-app`, `crcbl-shell`, `crcbl-hal`, `crcbl-ui`, `crcbl-frame` | gui                | House UI stack, as pikr. Every crcbl crate names the same version; bumps are their own commit                                                                                                                                                                                                                          |
| `glam`                                                             | gui                | crcbl-ui and crcbl-frame take glam vectors and do not re-export it (pikr precedent)                                                                                                                                                                                                                                    |
| `clap` 4 (derive)                                                  | app                | CLI                                                                                                                                                                                                                                                                                                                    |
| `anyhow`                                                           | app                | Binary-level errors                                                                                                                                                                                                                                                                                                    |
| `thiserror`                                                        | library crates     | Typed errors                                                                                                                                                                                                                                                                                                           |
| `tracing`, `tracing-subscriber`                                    | all                | Logs to `$XDG_STATE_HOME/dsplr/logs/`                                                                                                                                                                                                                                                                                  |
| `serde`, `serde_json`, `toml`                                      | model, config      | The JSON form, Hyprland's `j/` replies, config                                                                                                                                                                                                                                                                         |
| `hjkl-config`, `hjkl-xdg`                                          | config             | TOML loading and paths, as tmxr and lyr do                                                                                                                                                                                                                                                                             |
| `crcbl-wl-scanner` (build), `crcbl-wayland`                        | backend (wlr, kde) | The owner's choice (2026-11-04) over `wayland-client`: bindings generated from vendored protocol XML (`wlr-output-management`, and KDE's since M5), calling crcbl's libwayland-client binding. `crcbl-wayland` was added to crcbl for dsplr; the `wlr` backend uses it since M2                                        |
| `zbus` (no default features; `tokio`, `proxy`; `service` in tests) | backend (gnome)    | `org.gnome.Mutter.DisplayConfig`, added at M6 (6.2.1); house precedent in lyr                                                                                                                                                                                                                                          |
| `crcbl-x11`                                                        | backend (randr)    | The owner's choice (2026-11-11) over `x11rb`: crcbl for X11 as for Wayland. A RandR client crate of its own beside `crcbl-wayland`, not RandR inside crcbl-shell, as the crcbl session suggested; on crates.io since 0.19.1, which dsplr builds on (Linux and FreeBSD; it loads libxcb and libxcb-randr with `dlopen`) |
| `tokio` (current-thread runtime, `net`)                            | backend            | The owner's choice (2026-11-04); `AsyncFd` for the Wayland fd                                                                                                                                                                                                                                                          |
| dev: `tempfile`                                                    | tests              | Temp config dirs                                                                                                                                                                                                                                                                                                       |
