//! `dsplr set <OUTPUT> …`: one output of a fresh snapshot edited, and the
//! whole layout handed to the confirm flow `dsplr apply` runs
//! ([`apply::run`]), so validation, the test, the revert point and the
//! prompt are the same.

use dsplr_backend::Backend;
use dsplr_model::{
    AdaptiveSync, Capabilities, Layout, Mode, ModeSpec, OutputEntry, OutputId, Transform,
    is_valid_scale,
};

use crate::list::heading;
use crate::{Exit, apply};

/// How many of an output's modes a refused `--mode` lists, nearest first.
const NEAREST_MODES: usize = 5;

/// What `dsplr set` changes; at least one is required.
#[derive(Debug, clap::Args)]
#[group(required = true, multiple = true)]
pub struct Changes {
    /// The mode, as WxH@R with the refresh in hertz (`2560x1440@143.998`).
    /// WxH alone keeps the current refresh when the output lists it at that
    /// size, and otherwise takes the highest it lists there. A mode the
    /// output does not list is refused, unless the desktop takes custom
    /// modes; a custom mode needs its refresh.
    #[arg(long, value_name = "WxH[@R]")]
    pub mode: Option<ModeSpec>,
    /// The position of the output's top-left corner in the layout, in
    /// logical pixels.
    #[arg(long, value_name = "X,Y", allow_hyphen_values = true, value_parser = parse_position)]
    pub pos: Option<(i32, i32)>,
    /// The scale, a positive number (`1.5`).
    #[arg(long, value_name = "S", value_parser = parse_scale)]
    pub scale: Option<f64>,
    /// normal, 90, 180, 270, flipped, flipped-90, flipped-180 or
    /// flipped-270, as `dsplr list` prints them. These are wl_output's names
    /// (its 90 is a quarter turn counter-clockwise); sway names the quarter
    /// turns the other way round, so dsplr's 90 is sway's `transform 270`.
    #[arg(long, value_name = "T")]
    pub transform: Option<Transform>,
    /// Adaptive sync: off, on or automatic, as far as the desktop takes it
    /// (`dsplr backend` lists what it takes).
    #[arg(long, value_name = "off|on|automatic")]
    pub vrr: Option<AdaptiveSync>,
    /// Turn the output on. One without a mode takes its preferred mode, or
    /// else its largest; one the desktop reports at the origin may need
    /// `--pos` to stay clear of the others.
    #[arg(long, conflicts_with = "disable")]
    pub enable: bool,
    /// Turn the output off.
    #[arg(long)]
    pub disable: bool,
    /// Show the same picture as this output, named as the output is.
    #[arg(long, value_name = "OUT")]
    pub mirror: Option<String>,
    /// Stop mirroring another output; nothing changes for one that is not
    /// mirroring. The output keeps the position the desktop reports for it,
    /// and Hyprland reports a mirror at its source's position, so there it
    /// needs `--pos` to stay clear of the source.
    #[arg(long, conflicts_with = "mirror")]
    pub no_mirror: bool,
}

impl Changes {
    /// The flags given that set something on the output itself, rather
    /// than turn it on or off.
    fn settings(&self) -> Vec<&'static str> {
        [
            ("--mode", self.mode.is_some()),
            ("--pos", self.pos.is_some()),
            ("--scale", self.scale.is_some()),
            ("--transform", self.transform.is_some()),
            ("--vrr", self.vrr.is_some()),
            ("--mirror", self.mirror.is_some()),
            ("--no-mirror", self.no_mirror),
        ]
        .into_iter()
        .filter_map(|(flag, given)| given.then_some(flag))
        .collect()
    }
}

/// `X,Y`, each a signed whole number.
fn parse_position(text: &str) -> Result<(i32, i32), String> {
    text.split_once(',')
        .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
        .ok_or_else(|| format!("expected X,Y, such as 2560,0 or -1920,0, not {text:?}"))
}

/// A scale [`is_valid_scale`] takes.
fn parse_scale(text: &str) -> Result<f64, String> {
    text.parse::<f64>()
        .ok()
        .filter(|&scale| is_valid_scale(scale))
        .ok_or_else(|| format!("expected a positive number, such as 1.5, not {text:?}"))
}

/// Why a name found no single output.
#[derive(Debug, PartialEq, Eq)]
enum NotFound {
    Unknown,
    /// The indices of every output it names.
    Ambiguous(Vec<usize>),
}

/// The index of the output `name` names in `layout`: the one on that
/// connector, else the one with exactly that description, else the one
/// whose description starts with its words, as [`OutputId::matches`] reads
/// a description.
fn find(layout: &Layout, name: &str) -> Result<usize, NotFound> {
    let named = |by: &dyn Fn(&OutputEntry) -> bool| -> Vec<usize> {
        (0..layout.entries.len())
            .filter(|&i| by(&layout.entries[i]))
            .collect()
    };
    let connector = OutputId::Connector(name.to_owned());
    let description = OutputId::Description(name.to_owned());
    for found in [
        named(&|entry| connector.matches(entry)),
        named(&|entry| entry.description().as_deref() == Some(name.trim())),
        named(&|entry| description.matches(entry)),
    ] {
        match found[..] {
            [] => {}
            [index] => return Ok(index),
            _ => return Err(NotFound::Ambiguous(found)),
        }
    }
    Err(NotFound::Unknown)
}

/// [`find`], with a miss reported, listing the outputs to pick from.
fn find_reported(layout: &Layout, name: &str) -> Result<usize, Exit> {
    let (problem, listed): (_, Vec<usize>) = match find(layout, name) {
        Ok(index) => return Ok(index),
        Err(NotFound::Unknown) => ("names no output", (0..layout.entries.len()).collect()),
        Err(NotFound::Ambiguous(found)) => ("names more than one output", found),
    };
    eprintln!("dsplr: {name:?} {problem}; name one by its connector or description:");
    for index in listed {
        eprintln!("  {}", heading(&layout.entries[index]));
    }
    Err(Exit::Usage)
}

/// Why `changes` cannot be made to an output that is on (`enabled`) or off:
/// they set something on an output that is off once they are made, which no
/// desktop applies and validation does not look at.
fn off_refusal(enabled: bool, changes: &Changes) -> Option<String> {
    let settings = changes.settings();
    if settings.is_empty() {
        return None;
    }
    let settings = settings.join(", ");
    if changes.disable {
        Some(format!(
            "{settings} cannot be changed together with --disable"
        ))
    } else if !enabled && !changes.enable {
        Some(format!("it is off; add --enable to change {settings}"))
    } else {
        None
    }
}

/// Edits `output` in a fresh snapshot of `backend` as `changes` say, and
/// applies the result through [`apply::run`].
pub async fn run(
    backend: &Backend,
    output: &str,
    changes: &Changes,
    options: &apply::Options,
) -> Exit {
    let current = match crate::snapshot(backend).await {
        Ok(layout) => layout,
        Err(exit) => return exit,
    };
    let index = match find_reported(&current, output) {
        Ok(index) => index,
        Err(exit) => return exit,
    };
    let named = heading(&current.entries[index]);
    if let Some(refusal) = off_refusal(current.entries[index].state.enabled, changes) {
        eprintln!("dsplr: {named}: {refusal}");
        return Exit::Usage;
    }
    let mirror_of = match &changes.mirror {
        None => None,
        Some(name) => match find_reported(&current, name) {
            Ok(source) if source == index => {
                eprintln!("dsplr: {output} cannot mirror itself");
                return Exit::Usage;
            }
            Ok(source) => Some(current.entries[source].id.clone()),
            Err(exit) => return exit,
        },
    };
    let mut asked = current.clone();
    if let Err(refusal) = edit(
        &mut asked.entries[index],
        changes,
        mirror_of,
        &backend.capabilities(),
    ) {
        eprintln!("dsplr: {named}: {refusal}");
        return Exit::Refused;
    }
    // A description names an output by its first words, so say which one
    // this is.
    eprintln!("dsplr: setting {named}");
    apply::run(backend, current, &asked, options).await
}

/// Makes `changes` to `entry`, with `mirror_of` already resolved. The error
/// says why a setting cannot be had on this output or desktop; whatever
/// [`Layout::validate`] refuses is left to it.
fn edit(
    entry: &mut OutputEntry,
    changes: &Changes,
    mirror_of: Option<OutputId>,
    capabilities: &Capabilities,
) -> Result<(), String> {
    let listed = entry
        .output
        .as_ref()
        .map_or(&[][..], |output| &output.modes[..]);
    if let Some(asked) = changes.mode {
        entry.state.mode = Some(pick_mode(
            asked,
            listed,
            entry.state.mode.as_ref(),
            capabilities,
        )?);
    } else if changes.enable && entry.state.mode.is_none() {
        entry.state.mode = default_mode(listed).cloned();
    }
    if changes.enable {
        entry.state.enabled = true;
    }
    if changes.disable {
        entry.state.enabled = false;
    }
    if let Some((x, y)) = changes.pos {
        (entry.state.x, entry.state.y) = (x, y);
    }
    if let Some(scale) = changes.scale {
        entry.state.scale = scale;
    }
    if let Some(transform) = changes.transform {
        entry.state.transform = transform;
    }
    if let Some(vrr) = changes.vrr {
        if !capabilities.adaptive_sync.contains(&vrr) {
            return Err(format!("this desktop cannot set adaptive sync {vrr}"));
        }
        entry.state.adaptive_sync = Some(vrr);
    }
    if changes.no_mirror {
        entry.state.mirror_of = None;
    } else if mirror_of.is_some() {
        entry.state.mirror_of = mirror_of;
    }
    Ok(())
}

/// The mode `asked` names among `listed`, the output's modes, or a custom
/// mode where the desktop takes one. Without a refresh: the size at the
/// `current` mode's refresh when listed, so a new size keeps the refresh,
/// else the highest refresh listed at that size.
fn pick_mode(
    asked: ModeSpec,
    listed: &[Mode],
    current: Option<&Mode>,
    capabilities: &Capabilities,
) -> Result<Mode, String> {
    let at_size = || {
        listed
            .iter()
            .filter(|mode| (mode.width, mode.height) == (asked.width, asked.height))
    };
    let found = match asked.refresh_mhz {
        Some(refresh_mhz) => {
            let wanted = Mode::new(asked.width, asked.height, refresh_mhz);
            let closest = listed
                .iter()
                .filter(|mode| mode.matches(&wanted))
                .min_by_key(|mode| mode.refresh_mhz.abs_diff(refresh_mhz));
            match closest {
                Some(mode) => Some(mode),
                None if capabilities.custom_modes => return Ok(wanted),
                None => None,
            }
        }
        None => current
            .and_then(|current| {
                let same_refresh = Mode::new(asked.width, asked.height, current.refresh_mhz);
                at_size().find(|mode| mode.matches(&same_refresh))
            })
            .or_else(|| at_size().max_by_key(|mode| mode.refresh_mhz)),
    };
    if let Some(mode) = found {
        return Ok(mode.clone());
    }
    let size = format!("{}x{}", asked.width, asked.height);
    let mut refusal = match asked.refresh_mhz {
        Some(refresh_mhz) => format!(
            "{} is not a mode it lists, and this desktop takes no custom modes",
            Mode::new(asked.width, asked.height, refresh_mhz)
        ),
        None if capabilities.custom_modes => {
            format!("{size} is not a size it lists; give the refresh of a custom mode, as {size}@R")
        }
        None => format!("{size} is not a size it lists"),
    };
    let nearest = nearest(asked, listed);
    if !nearest.is_empty() {
        refusal.push_str("; nearest: ");
        refusal.push_str(&nearest.join(", "));
    }
    Err(refusal)
}

/// Up to [`NEAREST_MODES`] of `listed`, as `--mode` takes them, nearest
/// `asked` in size and then in refresh first, each once.
fn nearest(asked: ModeSpec, listed: &[Mode]) -> Vec<String> {
    let mut sorted: Vec<&Mode> = listed.iter().collect();
    sorted.sort_by_key(|mode| {
        (
            mode.width.abs_diff(asked.width) + mode.height.abs_diff(asked.height),
            asked
                .refresh_mhz
                .map_or(0, |refresh| mode.refresh_mhz.abs_diff(refresh)),
            std::cmp::Reverse(mode.refresh_mhz),
        )
    });
    let mut texts: Vec<String> = sorted.into_iter().map(ToString::to_string).collect();
    // Hyprland lists some modes twice; they read the same.
    texts.dedup();
    texts.truncate(NEAREST_MODES);
    texts
}

/// The mode an output turned on without one takes: the one it prefers, else
/// its largest, at its highest refresh (Hyprland's `highres`).
fn default_mode(listed: &[Mode]) -> Option<&Mode> {
    listed.iter().find(|mode| mode.preferred).or_else(|| {
        listed.iter().max_by_key(|mode| {
            (
                u64::from(mode.width) * u64::from(mode.height),
                mode.refresh_mhz,
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Capabilities, Layout, Mode, Output, OutputEntry, OutputId, OutputState,
        Transform,
    };

    use super::{
        Changes, ModeSpec, NotFound, default_mode, edit, find, off_refusal, parse_position,
        parse_scale, pick_mode,
    };

    fn mode_arg(width: u32, height: u32, refresh_mhz: Option<u32>) -> ModeSpec {
        ModeSpec {
            width,
            height,
            refresh_mhz,
        }
    }

    #[test]
    fn positions_are_two_signed_numbers() {
        assert_eq!(parse_position("2560,0"), Ok((2560, 0)));
        assert_eq!(parse_position("-1920,-1080"), Ok((-1920, -1080)));
        assert_eq!(
            parse_position("2147483647,-2147483648"),
            Ok((i32::MAX, i32::MIN))
        );
        for text in [
            "",
            "0",
            "0,",
            ",0",
            "0,0,0",
            "0 ,0",
            "1.5,0",
            "2147483648,0",
            "0x0",
        ] {
            assert!(parse_position(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn scales_are_positive_finite_numbers() {
        assert_eq!(parse_scale("1.5"), Ok(1.5));
        assert_eq!(parse_scale("2"), Ok(2.0));
        for text in ["", "0", "0.0", "-1", "-0", "inf", "NaN", "1,5", "x"] {
            assert!(parse_scale(text).is_err(), "{text:?}");
        }
    }

    fn output(connector: &str, description: &str, modes: Vec<Mode>) -> OutputEntry {
        OutputEntry {
            id: OutputId::Connector(connector.into()),
            output: Some(Output {
                connector: connector.into(),
                description: description.into(),
                physical_size_mm: None,
                modes,
            }),
            state: OutputState::default(),
        }
    }

    fn outputs() -> Layout {
        Layout {
            entries: vec![
                output("DP-1", "AOC Q27G4 SER1", vec![]),
                output("DP-2", "AOC Q27G4 SER12", vec![]),
                output("eDP-1", "BOE 0x0AFC", vec![]),
            ],
        }
    }

    #[test]
    fn an_output_is_found_by_connector_or_description() {
        let layout = outputs();
        assert_eq!(find(&layout, "DP-2"), Ok(1));
        assert_eq!(find(&layout, "BOE 0x0AFC"), Ok(2));
        assert_eq!(find(&layout, "BOE"), Ok(2), "the start of one");
        assert_eq!(find(&layout, "AOC Q27G4 SER12"), Ok(1));
        assert_eq!(
            find(&layout, "AOC Q27G4 SER1"),
            Ok(0),
            "a whole description beats one it starts"
        );
    }

    #[test]
    fn a_name_for_several_outputs_or_none_is_not_found() {
        let layout = outputs();
        assert_eq!(
            find(&layout, "AOC Q27G4"),
            Err(NotFound::Ambiguous(vec![0, 1]))
        );
        for name in [
            "DP-9",
            "dp-1",
            "Q27G4",
            "AOC Q27",
            "AOC Q27G4 SER",
            "",
            "  ",
        ] {
            assert_eq!(find(&layout, name), Err(NotFound::Unknown), "{name:?}");
        }
        // With SER1 unplugged, its name does not find SER12.
        let mut unplugged = outputs();
        unplugged.entries.remove(0);
        assert_eq!(find(&unplugged, "AOC Q27G4 SER1"), Err(NotFound::Unknown));
        let mut twins = outputs();
        twins.entries[1].output.as_mut().unwrap().description = "AOC Q27G4 SER1".into();
        assert_eq!(
            find(&twins, "AOC Q27G4 SER1"),
            Err(NotFound::Ambiguous(vec![0, 1]))
        );
        assert_eq!(
            find(&twins, "DP-2"),
            Ok(1),
            "the connector still tells them apart"
        );
    }

    fn modes() -> Vec<Mode> {
        vec![
            Mode::new(2560, 1440, 59_951),
            Mode::new(2560, 1440, 143_998),
            Mode::new(2560, 1440, 120_000),
            Mode::new(1920, 1080, 60_000),
            Mode::new(1920, 1080, 120_000),
            Mode::new(1920, 1080, 120_000),
        ]
    }

    #[test]
    fn a_mode_with_a_refresh_is_the_listed_one_it_matches() {
        let caps = Capabilities::default();
        let mut handled = modes();
        handled[1].handle = Some(dsplr_model::ModeHandle(1));
        let picked = pick_mode(mode_arg(2560, 1440, Some(144_000)), &handled, None, &caps);
        assert_eq!(
            picked,
            Ok(handled[1].clone()),
            "within the tolerance, handle kept"
        );
        assert!(picked.unwrap().handle.is_some());
    }

    #[test]
    fn a_refresh_takes_the_closest_listed_mode_not_the_first_within_tolerance() {
        let caps = Capabilities::default();
        let listed = [
            Mode::new(1920, 1080, 60_004),
            Mode::new(1920, 1080, 60_001),
            Mode::new(1920, 1080, 59_997),
        ];
        assert_eq!(
            pick_mode(mode_arg(1920, 1080, Some(60_000)), &listed, None, &caps),
            Ok(listed[1].clone())
        );
    }

    #[test]
    fn a_size_alone_keeps_the_refresh_or_takes_the_highest() {
        let caps = Capabilities::default();
        let at_120 = Mode::new(2560, 1440, 120_000);
        let at_60 = Mode::new(2560, 1440, 59_951);
        let asked = mode_arg(1920, 1080, None);
        assert_eq!(
            pick_mode(asked, &modes(), Some(&at_120), &caps),
            Ok(Mode::new(1920, 1080, 120_000))
        );
        assert_eq!(
            pick_mode(asked, &modes(), Some(&at_60), &caps),
            Ok(Mode::new(1920, 1080, 120_000)),
            "59.951 is not listed at 1920x1080: the highest"
        );
        assert_eq!(
            pick_mode(mode_arg(2560, 1440, None), &modes(), None, &caps),
            Ok(Mode::new(2560, 1440, 143_998))
        );
    }

    #[test]
    fn a_mode_not_listed_is_refused_naming_the_nearest() {
        let caps = Capabilities::default();
        let refusal =
            pick_mode(mode_arg(1920, 1080, Some(75_000)), &modes(), None, &caps).unwrap_err();
        assert!(refusal.contains("1920x1080@75 is not a mode"), "{refusal}");
        assert!(
            refusal.ends_with(
                "nearest: 1920x1080@60, 1920x1080@120, 2560x1440@59.951, 2560x1440@120, 2560x1440@143.998"
            ),
            "{refusal}"
        );
        let refusal = pick_mode(mode_arg(1280, 720, None), &modes(), None, &caps).unwrap_err();
        assert!(
            refusal.starts_with("1280x720 is not a size it lists;"),
            "{refusal}"
        );
        assert!(
            refusal.contains("nearest: 1920x1080@120, 1920x1080@60"),
            "{refusal}"
        );
        let refusal = pick_mode(mode_arg(1280, 720, None), &[], None, &caps).unwrap_err();
        assert_eq!(refusal, "1280x720 is not a size it lists");
    }

    #[test]
    fn a_custom_mode_needs_its_refresh_and_a_desktop_that_takes_it() {
        let caps = Capabilities {
            custom_modes: true,
            ..Capabilities::default()
        };
        assert_eq!(
            pick_mode(mode_arg(1280, 720, Some(50_000)), &modes(), None, &caps),
            Ok(Mode::new(1280, 720, 50_000))
        );
        let refusal = pick_mode(mode_arg(1280, 720, None), &modes(), None, &caps).unwrap_err();
        assert!(refusal.contains("as 1280x720@R"), "{refusal}");
    }

    #[test]
    fn an_output_turned_on_without_a_mode_takes_its_preferred_or_largest() {
        let mut listed = modes();
        assert_eq!(default_mode(&listed), Some(&Mode::new(2560, 1440, 143_998)));
        listed[3].preferred = true;
        assert_eq!(default_mode(&listed).map(|m| m.refresh_mhz), Some(60_000));
        assert_eq!(default_mode(&[]), None);
    }

    fn changes() -> Changes {
        Changes {
            mode: None,
            pos: None,
            scale: None,
            transform: None,
            vrr: None,
            enable: false,
            disable: false,
            mirror: None,
            no_mirror: false,
        }
    }

    #[test]
    fn an_edit_changes_what_was_asked_and_nothing_else() {
        let mut entry = output("DP-1", "AOC", modes());
        entry.state.mode = Some(modes()[0].clone());
        let before = entry.clone();
        let caps = Capabilities {
            adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
            ..Capabilities::default()
        };
        let asked = Changes {
            pos: Some((-1920, 10)),
            scale: Some(1.5),
            transform: Some(Transform::Flipped90),
            vrr: Some(AdaptiveSync::On),
            ..changes()
        };
        let source = OutputId::Connector("DP-2".into());
        edit(&mut entry, &asked, Some(source.clone()), &caps).unwrap();
        let state = &entry.state;
        assert_eq!((state.x, state.y), (-1920, 10));
        assert!((state.scale - 1.5).abs() < f64::EPSILON);
        assert_eq!(state.transform, Transform::Flipped90);
        assert_eq!(state.adaptive_sync, Some(AdaptiveSync::On));
        assert_eq!(state.mirror_of, Some(source));
        assert_eq!(state.mode, before.state.mode);
        assert_eq!(entry.output, before.output);

        let mut entry = before.clone();
        edit(
            &mut entry,
            &Changes {
                scale: Some(2.0),
                ..changes()
            },
            None,
            &caps,
        )
        .unwrap();
        assert_eq!(
            entry,
            OutputEntry {
                state: OutputState {
                    scale: 2.0,
                    ..before.state.clone()
                },
                ..before
            }
        );
    }

    #[test]
    fn settings_for_an_output_left_off_are_refused() {
        let pos = Changes {
            pos: Some((0, 0)),
            ..changes()
        };
        assert_eq!(off_refusal(true, &pos), None);
        assert_eq!(
            off_refusal(false, &pos).as_deref(),
            Some("it is off; add --enable to change --pos")
        );
        let enable = Changes {
            enable: true,
            ..pos
        };
        assert_eq!(off_refusal(false, &enable), None);
        let disable = Changes {
            disable: true,
            scale: Some(2.0),
            mirror: Some("DP-2".into()),
            ..changes()
        };
        for enabled in [true, false] {
            assert_eq!(
                off_refusal(enabled, &disable).as_deref(),
                Some("--scale, --mirror cannot be changed together with --disable")
            );
        }
        // Turning it off or on alone changes no setting.
        for (enabled, alone) in [(true, "disable"), (false, "disable"), (false, "enable")] {
            let alone = Changes {
                enable: alone == "enable",
                disable: alone == "disable",
                ..changes()
            };
            assert_eq!(off_refusal(enabled, &alone), None);
        }
    }

    #[test]
    fn no_mirror_ends_a_mirror_and_changes_nothing_on_an_output_not_mirroring() {
        let caps = Capabilities::default();
        let no_mirror = Changes {
            no_mirror: true,
            ..changes()
        };
        let mut entry = output("DP-1", "AOC", modes());
        entry.state.mirror_of = Some(OutputId::Connector("DP-2".into()));
        let mirroring = entry.clone();
        edit(&mut entry, &no_mirror, None, &caps).unwrap();
        assert_eq!(entry.state.mirror_of, None);
        assert_eq!(
            entry,
            OutputEntry {
                state: OutputState {
                    mirror_of: None,
                    ..mirroring.state.clone()
                },
                ..mirroring
            }
        );

        // Not mirroring, it is left as it is, on a desktop without
        // mirroring too: like any setting asked for at its current value.
        let before = entry.clone();
        edit(&mut entry, &no_mirror, None, &caps).unwrap();
        assert_eq!(entry, before);

        // It is a setting, so an output left off refuses it.
        assert_eq!(
            off_refusal(false, &no_mirror).as_deref(),
            Some("it is off; add --enable to change --no-mirror")
        );
    }

    #[test]
    fn adaptive_sync_the_desktop_does_not_take_is_refused() {
        let mut entry = output("DP-1", "AOC", modes());
        let asked = Changes {
            vrr: Some(AdaptiveSync::Automatic),
            ..changes()
        };
        let caps = Capabilities {
            adaptive_sync: vec![AdaptiveSync::Off, AdaptiveSync::On],
            ..Capabilities::default()
        };
        let refusal = edit(&mut entry, &asked, None, &caps).unwrap_err();
        assert_eq!(refusal, "this desktop cannot set adaptive sync automatic");
        assert_eq!(entry.state.adaptive_sync, None);
    }

    #[test]
    fn enabling_an_output_without_a_mode_gives_it_one_and_disabling_keeps_it() {
        let mut entry = output("DP-1", "AOC", modes());
        entry.state.enabled = false;
        let caps = Capabilities::default();
        let enable = Changes {
            enable: true,
            ..changes()
        };
        edit(&mut entry, &enable, None, &caps).unwrap();
        assert!(entry.state.enabled);
        assert_eq!(entry.state.mode, Some(Mode::new(2560, 1440, 143_998)));

        let disable = Changes {
            disable: true,
            ..changes()
        };
        edit(&mut entry, &disable, None, &caps).unwrap();
        assert!(!entry.state.enabled);
        assert_eq!(entry.state.mode, Some(Mode::new(2560, 1440, 143_998)));

        // --enable with --mode takes the mode asked.
        let mut entry = output("DP-1", "AOC", modes());
        entry.state.enabled = false;
        let both = Changes {
            enable: true,
            mode: Some(mode_arg(1920, 1080, Some(60_000))),
            ..changes()
        };
        edit(&mut entry, &both, None, &caps).unwrap();
        assert_eq!(entry.state.mode, Some(Mode::new(1920, 1080, 60_000)));
    }
}
