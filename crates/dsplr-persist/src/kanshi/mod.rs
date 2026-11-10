//! kanshi's profiles imported into dsplr's profile store
//! (`docs/plan/05-persistence-and-profiles.md`, "Importing kanshi profiles").
//!
//! [`config::read`] reads kanshi's config as kanshi does; [`import`] turns
//! each profile into a dsplr profile's layout. A kanshi output leaves
//! unset whatever it does not name, and kanshi then keeps the output's own
//! value; a dsplr profile holds every setting. So what kanshi leaves unset,
//! or leaves to the output (a mode without a refresh, `mode preferred`), is
//! taken from the output as it is now, when it is connected, and else from
//! dsplr's defaults; each is said in the profile's notes. A profile dsplr
//! cannot hold (a wildcard output, a description pattern other than a
//! trailing `*`, an enabled output whose mode cannot be had) is skipped,
//! with why, rather than imported to match other outputs than kanshi's.
//! `exec` is not run, and said.

pub mod config;
mod scfg;

use std::str::FromStr;

use dsplr_model::{
    AdaptiveSync, Layout, Mode, OutputEntry, OutputId, OutputState, Transform, default_mode,
    hz_to_mhz, is_valid_scale,
};

pub use config::{ConfigError, KanshiConfig, KanshiMode, KanshiOutput, KanshiProfile};
pub use scfg::ScfgError;

use crate::identity::identifier;
use crate::profile::ProfileName;

/// A kanshi profile as a dsplr profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    pub name: ProfileName,
    /// The kanshi profile it came from: its name, or kanshi's for an
    /// anonymous one, and where it is.
    pub from: String,
    pub layout: Layout,
    /// What was not taken from kanshi as written, one note each.
    pub notes: Vec<String>,
}

/// A kanshi profile not imported, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub from: String,
    pub reason: String,
}

/// Each profile of `config` as a dsplr profile, in kanshi's order, or why
/// it was skipped. `live` is the desktop's outputs now, when it could be
/// read: what kanshi leaves to an output is taken from it.
pub fn import(config: &KanshiConfig, live: Option<&Layout>) -> (Vec<Imported>, Vec<Skipped>) {
    let mut imported: Vec<Imported> = Vec::new();
    let mut skipped = Vec::new();
    for profile in &config.profiles {
        let from = format!("{} ({})", config::label(profile), profile.at);
        let mut notes = Vec::new();
        let name = match profile_name(profile, &mut notes) {
            Ok(name) => name,
            Err(reason) => {
                skipped.push(Skipped { from, reason });
                continue;
            }
        };
        if imported.iter().any(|other| other.name == name) {
            skipped.push(Skipped {
                from,
                reason: format!("an earlier kanshi profile is imported as \"{name}\" too"),
            });
            continue;
        }
        match layout(profile, live, &mut notes) {
            Ok(layout) => {
                if profile.execs > 0 {
                    notes.push(format!(
                        "its {} `exec` command{} not run by dsplr",
                        profile.execs,
                        if profile.execs == 1 { " is" } else { "s are" }
                    ));
                }
                imported.push(Imported {
                    name,
                    from,
                    layout,
                    notes,
                });
            }
            Err(reason) => skipped.push(Skipped { from, reason }),
        }
    }
    (imported, skipped)
}

/// The dsplr name for `profile`: its own, when it is a profile name; else
/// one made from it, its unusable characters each `_`; an anonymous one is
/// `kanshi-N`, kanshi's own number for it.
fn profile_name(profile: &KanshiProfile, notes: &mut Vec<String>) -> Result<ProfileName, String> {
    let Some(name) = &profile.name else {
        let number = profile.anonymous.unwrap_or_default();
        notes.push(format!("anonymous in kanshi; named kanshi-{number} here"));
        return ProfileName::from_str(&format!("kanshi-{number}")).map_err(|e| e.to_string());
    };
    if let Ok(name) = ProfileName::from_str(name) {
        return Ok(name);
    }
    let made: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let made = made.replace("..", "_.");
    let made = made.trim_end_matches([' ', '.']);
    let made = if made.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        made.to_owned()
    } else {
        format!("_{made}")
    };
    let made = ProfileName::from_str(&made)
        .map_err(|error| format!("its name \"{name}\" cannot be a dsplr profile's: {error}"))?;
    notes.push(format!(
        "named \"{made}\" here: \"{name}\" is not a dsplr profile name"
    ));
    Ok(made)
}

/// `profile`'s outputs as a layout.
fn layout(
    profile: &KanshiProfile,
    live: Option<&Layout>,
    notes: &mut Vec<String>,
) -> Result<Layout, String> {
    let mut entries = Vec::new();
    for output in &profile.outputs {
        let id = identity(output, live, notes)?;
        let now = live.and_then(|live| live.entries.iter().find(|entry| id.matches(entry)));
        entries.push(OutputEntry {
            state: state(output, &id, now, notes)?,
            id,
            output: None,
        });
    }
    if entries.is_empty() {
        return Err("it names no output".into());
    }
    Ok(Layout {
        entries,
        ..Layout::default()
    })
}

/// The identity a kanshi criteria names: a connector; or, for `make model
/// serial`, the EDID identity of the connected output whose own `make model
/// serial` it is (kanshi builds that string with `Unknown` for a part the
/// output does not report, as [`identifier`] does), when that output has a
/// serial; else a description, which dsplr matches by its whole leading
/// words, so a trailing `*` (kanshi's glob for any serial) is dropped, and
/// so is a trailing `Unknown`, with a note: such an identity names the same
/// monitor with any serial. A wildcard, an output any number of times,
/// another glob, or one that is all `Unknown` are refused.
fn identity(
    output: &KanshiOutput,
    live: Option<&Layout>,
    notes: &mut Vec<String>,
) -> Result<OutputId, String> {
    let criteria = output.criteria.as_str();
    if output.any_number {
        return Err(format!(
            "`...output {criteria}` matches any number of outputs, and a dsplr profile names each"
        ));
    }
    if criteria == "*" {
        return Err("`output *` matches any output, and a dsplr profile names each".into());
    }
    if !criteria.contains(char::is_whitespace) {
        if criteria.contains(['*', '?', '[']) {
            return Err(format!("the pattern \"{criteria}\" names no one output"));
        }
        return Ok(OutputId::Connector(criteria.to_owned()));
    }
    let monitor = live
        .into_iter()
        .flat_map(|live| &live.entries)
        .find(|entry| identifier(entry).as_deref() == Some(criteria))
        .and_then(|entry| entry.output.as_ref())
        .filter(|output| !output.serial.trim().is_empty());
    if let Some(monitor) = monitor {
        return Ok(OutputId::new(
            &monitor.make,
            &monitor.model,
            &monitor.serial,
            &monitor.connector,
        ));
    }
    let mut words: Vec<&str> = criteria.split_whitespace().collect();
    let mut dropped = false;
    while let Some(&last) = words.last()
        && (last == "*" || last == "Unknown")
    {
        dropped = true;
        words.pop();
    }
    if words.iter().all(|&word| word == "Unknown") {
        return Err(format!(
            "\"{criteria}\" names an output that reports no make, model or serial, which dsplr names only by its connector"
        ));
    }
    let description = words.join(" ");
    if description.contains(['*', '?', '[']) {
        return Err(format!(
            "the pattern \"{criteria}\" is not one dsplr can match (only a trailing `*` is)"
        ));
    }
    if dropped {
        notes.push(format!(
            "\"{criteria}\" is matched as any monitor whose description starts \"{description}\""
        ));
    }
    Ok(OutputId::Description(description))
}

/// What `output` sets, and what it leaves as the output has it taken from
/// `now` (the output as it is, when connected) or dsplr's defaults.
fn state(
    output: &KanshiOutput,
    id: &OutputId,
    now: Option<&OutputEntry>,
    notes: &mut Vec<String>,
) -> Result<OutputState, String> {
    let defaults = OutputState::default();
    let live = now.map(|entry| &entry.state);
    let modes: &[Mode] = now
        .and_then(|entry| entry.output.as_ref())
        .map_or(&[], |output| &output.modes);
    let mut unset: Vec<&str> = Vec::new();
    let mut taken = |what: &'static str| unset.push(what);

    let enabled = output.enabled.unwrap_or_else(|| {
        taken("on or off");
        live.map_or(defaults.enabled, |live| live.enabled)
    });
    if !enabled {
        if !unset.is_empty() {
            note_unset(id, now.is_some(), &unset, notes);
        }
        return Ok(OutputState {
            enabled: false,
            ..defaults
        });
    }
    let not_connected = || {
        format!(
            "{id} is not connected now, so the mode kanshi leaves to it cannot be had; import again with it connected"
        )
    };
    let mode = match output.mode {
        Some(KanshiMode::Size {
            width,
            height,
            refresh_hz: Some(hz),
            ..
        }) => Mode::new(
            width,
            height,
            hz_to_mhz(hz).ok_or_else(|| format!("{id}: refresh {hz} Hz is out of range"))?,
        ),
        Some(KanshiMode::Size {
            width,
            height,
            refresh_hz: None,
            ..
        }) => {
            if now.is_none() {
                return Err(not_connected());
            }
            // kanshi's `match_mode`: the highest refresh at that size.
            let mode = modes
                .iter()
                .filter(|mode| (mode.width, mode.height) == (width, height))
                .max_by_key(|mode| mode.refresh_mhz)
                .ok_or_else(|| format!("{id} lists no {width}x{height} mode"))?;
            notes.push(format!(
                "{id}: {width}x{height} names no refresh; {} here, the highest it lists, as kanshi picks",
                mode
            ));
            Mode {
                handle: None,
                ..mode.clone()
            }
        }
        Some(KanshiMode::Preferred) => {
            if now.is_none() {
                return Err(not_connected());
            }
            let mode = modes
                .iter()
                .find(|mode| mode.preferred)
                .ok_or_else(|| format!("{id} has no preferred mode"))?;
            notes.push(format!("{id}: its preferred mode is {mode} now"));
            Mode {
                handle: None,
                ..mode.clone()
            }
        }
        None => {
            taken("mode");
            let mode = live
                .and_then(|live| live.mode.as_ref().filter(|_| live.enabled))
                .or_else(|| default_mode(modes))
                .ok_or_else(not_connected)?;
            Mode {
                handle: None,
                ..mode.clone()
            }
        }
    };
    let (x, y) = output.position.unwrap_or_else(|| {
        taken("position");
        live.map_or((defaults.x, defaults.y), |live| (live.x, live.y))
    });
    let scale = output.scale.unwrap_or_else(|| {
        taken("scale");
        live.map_or(defaults.scale, |live| live.scale)
    });
    if !is_valid_scale(scale) {
        return Err(format!("{id}: scale {scale} is not a scale"));
    }
    let transform = output.transform.unwrap_or_else(|| {
        taken("transform");
        live.map_or(Transform::Normal, |live| live.transform)
    });
    if !unset.is_empty() {
        note_unset(id, now.is_some(), &unset, notes);
    }
    Ok(OutputState {
        enabled: true,
        mode: Some(mode),
        x,
        y,
        scale,
        transform,
        adaptive_sync: output.adaptive_sync.map(|on| {
            if on {
                AdaptiveSync::On
            } else {
                AdaptiveSync::Off
            }
        }),
        ..defaults
    })
}

fn note_unset(id: &OutputId, connected: bool, unset: &[&str], notes: &mut Vec<String>) {
    let what = unset.join(", ");
    notes.push(if connected {
        format!("{id}: kanshi leaves its {what} as it is; taken from it as it is now")
    } else {
        format!(
            "{id}: kanshi leaves its {what} as it is; it is not connected now, so dsplr's defaults"
        )
    });
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use dsplr_model::{
        AdaptiveSync, Layout, Mode, Output, OutputEntry, OutputId, OutputState, Transform,
    };

    use super::config::{KanshiConfig, KanshiMode, KanshiOutput, KanshiProfile};
    use super::{Skipped, import};
    use crate::profile::ProfileName;

    fn profile(name: Option<&str>, outputs: Vec<KanshiOutput>) -> KanshiProfile {
        KanshiProfile {
            name: name.map(str::to_owned),
            anonymous: name.is_none().then_some(1),
            outputs,
            execs: 0,
            at: "config:1".into(),
        }
    }

    fn output(criteria: &str) -> KanshiOutput {
        KanshiOutput {
            criteria: criteria.into(),
            ..KanshiOutput::default()
        }
    }

    fn size(width: u32, height: u32, refresh_hz: Option<f64>) -> Option<KanshiMode> {
        Some(KanshiMode::Size {
            width,
            height,
            refresh_hz,
            custom: false,
        })
    }

    /// A laptop panel on at 1920x1200@60 at the origin, scale 1.25, and an
    /// AOC listing 2560x1440 at 60 and 120 Hz (preferred 120), off.
    fn live() -> Layout {
        let mut preferred = Mode::new(2560, 1440, 120_000);
        preferred.preferred = true;
        Layout {
            entries: vec![
                OutputEntry {
                    id: OutputId::Connector("eDP-1".into()),
                    output: Some(Output {
                        connector: "eDP-1".into(),
                        description: "BOE 0x0AFC (eDP-1)".into(),
                        make: "BOE".into(),
                        model: "0x0AFC".into(),
                        serial: String::new(),
                        physical_size_mm: None,
                        modes: vec![Mode::new(1920, 1200, 60_000)],
                    }),
                    state: OutputState {
                        mode: Some(Mode::new(1920, 1200, 60_000)),
                        scale: 1.25,
                        ..OutputState::default()
                    },
                },
                OutputEntry {
                    id: OutputId::new("AOC", "Q27G4", "SER1", "DP-1"),
                    output: Some(Output {
                        connector: "DP-1".into(),
                        description: "AOC Q27G4 SER1 (DP-1)".into(),
                        make: "AOC".into(),
                        model: "Q27G4".into(),
                        serial: "SER1".into(),
                        physical_size_mm: None,
                        modes: vec![Mode::new(2560, 1440, 60_000), preferred],
                    }),
                    state: OutputState {
                        enabled: false,
                        ..OutputState::default()
                    },
                },
            ],
            ..Layout::default()
        }
    }

    fn config(profiles: Vec<KanshiProfile>) -> KanshiConfig {
        KanshiConfig {
            profiles,
            notes: Vec::new(),
        }
    }

    #[test]
    fn what_kanshi_sets_is_taken_as_written() {
        let desk = KanshiOutput {
            enabled: Some(true),
            mode: size(2560, 1440, Some(59.951)),
            position: Some((1920, -100)),
            scale: Some(1.5),
            transform: Some(Transform::Rotate270),
            adaptive_sync: Some(true),
            ..output("AOC Q27G4 SER1")
        };
        let laptop = KanshiOutput {
            enabled: Some(false),
            ..output("eDP-1")
        };
        let (imported, skipped) = import(
            &config(vec![profile(Some("desk"), vec![laptop, desk])]),
            None,
        );
        assert_eq!(skipped, []);
        let desk = &imported[0];
        assert_eq!(desk.name, ProfileName::from_str("desk").unwrap());
        assert_eq!(desk.notes, Vec::<String>::new(), "nothing guessed");
        let entries = &desk.layout.entries;
        assert_eq!(entries[0].id, OutputId::Connector("eDP-1".into()));
        assert!(!entries[0].state.enabled);
        assert_eq!(
            entries[1].id,
            OutputId::Description("AOC Q27G4 SER1".into())
        );
        assert_eq!(
            entries[1].state,
            OutputState {
                mode: Some(Mode::new(2560, 1440, 59_951)),
                x: 1920,
                y: -100,
                scale: 1.5,
                transform: Transform::Rotate270,
                adaptive_sync: Some(AdaptiveSync::On),
                ..OutputState::default()
            }
        );
        // The description names the live monitor.
        assert!(entries[1].id.matches(&live().entries[1]));

        // With the monitor connected, `make model serial` is its EDID
        // identity, which follows it to any port.
        let live = live();
        let (imported, _) = import(
            &config(vec![profile(
                Some("desk"),
                vec![KanshiOutput {
                    enabled: Some(false),
                    ..output("AOC Q27G4 SER1")
                }],
            )]),
            Some(&live),
        );
        assert_eq!(
            imported[0].layout.entries[0].id,
            OutputId::new("AOC", "Q27G4", "SER1", "DP-1")
        );
    }

    #[test]
    fn what_kanshi_leaves_to_the_output_comes_from_it_as_it_is_now() {
        let laptop = output("eDP-1");
        let desk = KanshiOutput {
            mode: size(2560, 1440, None),
            enabled: Some(true),
            ..output("DP-1")
        };
        let preferred = KanshiOutput {
            mode: Some(KanshiMode::Preferred),
            enabled: Some(true),
            ..output("AOC Q27G4 *")
        };
        let live = live();
        let (imported, skipped) = import(
            &config(vec![
                profile(Some("a"), vec![laptop, desk]),
                profile(Some("b"), vec![preferred]),
            ]),
            Some(&live),
        );
        assert_eq!(skipped, []);
        let a = &imported[0].layout.entries;
        assert_eq!(a[0].state.mode, Some(Mode::new(1920, 1200, 60_000)));
        assert!(
            (a[0].state.scale - 1.25).abs() < f64::EPSILON,
            "as it is now"
        );
        let top = a[1].state.mode.as_ref().unwrap();
        assert_eq!(
            (top.width, top.height, top.refresh_mhz),
            (2560, 1440, 120_000)
        );
        assert!(
            imported[0]
                .notes
                .iter()
                .any(|n| n.contains("as kanshi picks"))
        );
        assert!(
            imported[0].notes.iter().any(|n| n.starts_with(
                "eDP-1: kanshi leaves its on or off, mode, position, scale, transform"
            )),
            "{:?}",
            imported[0].notes
        );
        let b = &imported[1];
        assert_eq!(
            b.layout.entries[0].id,
            OutputId::Description("AOC Q27G4".into())
        );
        assert_eq!(
            b.layout.entries[0].state.mode.as_ref().unwrap().refresh_mhz,
            120_000
        );
        assert!(b.notes.iter().any(|n| n.contains("starts \"AOC Q27G4\"")));
    }

    #[test]
    fn a_profile_dsplr_cannot_hold_is_skipped_with_why() {
        let wildcard = profile(Some("w"), vec![output("eDP-1"), output("*")]);
        let any = profile(
            Some("n"),
            vec![KanshiOutput {
                any_number: true,
                ..output("*")
            }],
        );
        let pattern = profile(Some("p"), vec![output("AOC Q2? SER1")]);
        let unknown = profile(Some("k"), vec![output("Unknown Unknown Unknown")]);
        let unplugged = profile(
            Some("u"),
            vec![KanshiOutput {
                enabled: Some(true),
                ..output("HDMI-A-9")
            }],
        );
        let empty = profile(Some("e"), vec![]);
        let live = live();
        let (imported, skipped) = import(
            &config(vec![wildcard, any, pattern, unplugged, empty, unknown]),
            Some(&live),
        );
        assert_eq!(imported, []);
        let reasons: Vec<&str> = skipped
            .iter()
            .map(|s: &Skipped| s.reason.as_str())
            .collect();
        assert!(reasons[0].contains("`output *`"), "{reasons:?}");
        assert!(reasons[1].contains("any number"), "{reasons:?}");
        assert!(reasons[2].contains("pattern"), "{reasons:?}");
        assert!(reasons[3].contains("not connected now"), "{reasons:?}");
        assert!(reasons[4].contains("names no output"), "{reasons:?}");
        assert!(
            reasons[5].contains("no make, model or serial"),
            "{reasons:?}"
        );
        assert!(skipped[0].from.starts_with("\"w\" (config:1)"));
    }

    #[test]
    fn names_are_kept_made_or_generated_and_never_taken_twice() {
        let off = || KanshiOutput {
            enabled: Some(false),
            ..output("eDP-1")
        };
        let (imported, skipped) = import(
            &config(vec![
                profile(None, vec![off()]),
                profile(Some("home/desk"), vec![off()]),
                profile(Some("home_desk"), vec![off()]),
            ]),
            None,
        );
        let names: Vec<&str> = imported.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["kanshi-1", "home_desk"]);
        assert!(imported[0].notes[0].contains("anonymous"));
        assert!(imported[1].notes[0].contains("\"home/desk\" is not a dsplr profile name"));
        assert_eq!(skipped.len(), 1, "the second home_desk");
        assert!(skipped[0].reason.contains("too"));
    }

    #[test]
    fn exec_is_said_not_run() {
        let mut with_exec = profile(
            Some("x"),
            vec![KanshiOutput {
                enabled: Some(false),
                ..output("eDP-1")
            }],
        );
        with_exec.execs = 2;
        let (imported, _) = import(&config(vec![with_exec]), None);
        assert!(
            imported[0]
                .notes
                .iter()
                .any(|n| n.contains("2 `exec` commands are not run"))
        );
    }
}
