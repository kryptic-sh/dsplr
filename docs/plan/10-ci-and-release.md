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

Added with their backends: `sway-e2e` (M4), `kwin-e2e` (M5), `mutter-e2e` (M6),
`x11-e2e` (M7), `hyprland-e2e` (M2, if it runs headless).

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
