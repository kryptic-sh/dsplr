# 10 — CI and release

## Template

lyr's pipeline (itself from tmxr and hrdr) is the template: checks now, release
jobs at the first release. dsplr is Linux-only, so the matrix is ubuntu only.

## Phase 1 — scaffold (M0)

`.github/workflows/ci.yml` on `pull_request`, `push` to `main`, `v*` tags and
`workflow_dispatch`:

| Job          | What                                                                                   |
| ------------ | -------------------------------------------------------------------------------------- |
| `fmt`        | `cargo fmt --all --check`                                                              |
| `clippy`     | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`        |
| `test`       | `cargo nextest run --workspace --all-features --locked --no-fail-fast` + doctests      |
| `smoke`      | build, `dsplr --version`, `dsplr --help`                                               |
| `leak-guard` | the suite with `$HOME` and XDG dirs at an empty sentinel; fails if a test writes there |
| `machete`    | unused dependencies                                                                    |
| `deny`       | `cargo deny --all-features check`                                                      |
| `audit`      | `cargo audit`                                                                          |

`e2e` is a matrix with one entry per desktop, each running that desktop's
`tests/e2e/headless-<desktop>.sh` harness and `<desktop>_e2e` test inside a KVM
VM (an Arch cloud image with a virtio GPU): a runner has no GPU render node, and
Hyprland cannot allocate buffers without one. Hyprland, sway and niri (nested in
a headless sway) and KWin (its virtual backend) are in it now; GNOME (M6) and
X11 (M7) join with their backends. Test compositors never run on or against a
desktop session: private runtime directory, no shared sockets, no seat.

Conventions as lyr: `RUSTFLAGS=-D warnings`, a concurrency group cancelling
superseded non-tag runs, `fail-fast: false`, `Swatinem/rust-cache`,
`taiki-e/install-action` for nextest, machete, deny and audit, `cron.yml` for a
weekly `cargo deny`, `dependabot.yml`, `CODEOWNERS`, PR and issue templates,
`.config/nextest.toml`.

## Phase 2 — release (M9)

Tag-triggered: Linux x86_64 and aarch64 binaries, an AUR package (`pkg/`, as lyr
and pikr), crates.io publish of the workspace in dependency order, completions
and man page in the archive, a site entry on kryptic.sh. Each publish job gated
on the build jobs; the release is checked by enumerating the run's jobs and
confirming the artifacts landed.
