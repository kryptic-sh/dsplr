//! autorandr's profiles imported into dsplr's profile store
//! (`docs/plan/05-persistence-and-profiles.md`, "Importing autorandr
//! profiles").
//!
//! [`profiles::read`] reads autorandr's profile directories as autorandr
//! does; [`import`] turns each profile into a dsplr profile's layout. Each
//! output is named by the EDID its `setup` line holds
//! ([`dsplr_model::edid`]), or by its connector when that gives no serial
//! or there is no EDID to read. What `config` sets that a layout holds
//! (mode, rate, position, rotation and reflection, primary, off, `same-as`)
//! is taken; what a layout has no place for (gamma, CRTC, filter, panning,
//! `x-prop-*` properties, any other option) is left out, said per output;
//! and what
//! would put the outputs elsewhere than autorandr puts them (a scale or
//! transform, a position relative to another output, a mode left to
//! xrandr) skips the profile, with why. The hook scripts are not run, and
//! said.

pub mod profiles;

use dsplr_model::edid::Edid;
use dsplr_model::{Layout, Mode, OutputEntry, OutputId, OutputState, Transform, hz_text_to_mhz};

pub use crate::import::{Imported, Skipped};
pub use profiles::{
    AutorandrOutput, AutorandrProfile, Read, ReadError, default_dirs, is_profile, read, read_one,
};

use crate::import::name_from;

/// What autorandr writes in `setup` for a connected output it read no EDID
/// for, followed by the output's name (`EDID_UNAVAILABLE`).
const EDID_UNAVAILABLE: &str = "--CONNECTED-BUT-EDID-UNAVAILABLE-";

/// The length of an older autorandr's MD5 of an EDID, in hex.
const MD5_HEX: usize = 32;

/// Options that would put the output elsewhere than the layout can say,
/// which skip the profile.
const REFUSED: [&str; 7] = [
    "left-of",
    "right-of",
    "above",
    "below",
    "auto",
    "preferred",
    "scale-from",
];

/// Values of options a layout has no place for that are xrandr's defaults,
/// which autorandr leaves out (`XRANDR_DEFAULTS`): no setting, so nothing
/// to say.
const DEFAULTS: [(&str, &str); 2] = [("panning", "0x0"), ("gamma", "1.0:1.0:1.0")];

/// Each profile of `read` as a dsplr profile, in the order read, or why it
/// was skipped. `live` is the desktop's outputs now, when they could be
/// read: a mode autorandr gives no rate for takes the highest the output
/// lists at that size.
pub fn import(read: &Read, live: Option<&Layout>) -> (Vec<Imported>, Vec<Skipped>) {
    let mut imported: Vec<Imported> = Vec::new();
    let mut skipped = read.unreadable.clone();
    for profile in &read.profiles {
        let from = format!("\"{}\" ({})", profile.name, profile.dir.display());
        let mut notes = Vec::new();
        let name = match name_from(&profile.name, &mut notes) {
            Ok(name) => name,
            Err(reason) => {
                skipped.push(Skipped { from, reason });
                continue;
            }
        };
        if imported.iter().any(|other| other.name == name) {
            skipped.push(Skipped {
                from,
                reason: format!("an earlier autorandr profile is imported as \"{name}\" too"),
            });
            continue;
        }
        match layout(profile, live, &mut notes) {
            Ok(layout) => {
                for output in &profile.dropped {
                    notes.push(format!(
                        "{output}: off and with no EDID in setup, which autorandr leaves out too"
                    ));
                }
                if !profile.scripts.is_empty() {
                    notes.push(format!(
                        "its {} not run by dsplr",
                        profiles::said_scripts(&profile.scripts)
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

/// `profile`'s outputs as a layout.
fn layout(
    profile: &AutorandrProfile,
    live: Option<&Layout>,
    notes: &mut Vec<String>,
) -> Result<Layout, String> {
    let mut entries: Vec<OutputEntry> = Vec::new();
    let mut same_as: Vec<usize> = Vec::new();
    for output in &profile.outputs {
        let id = identity(output, notes);
        let now = live.and_then(|live| live.entries.iter().find(|entry| id.matches(entry)));
        if output.option("same-as").is_some() {
            same_as.push(entries.len());
        }
        entries.push(OutputEntry {
            state: state(output, &id, now, notes)?,
            id,
            output: None,
        });
    }
    if entries.is_empty() {
        return Err("it names no output".into());
    }

    // `same-as`: a mirror where its source is, a chain followed to its
    // end, as xrandr's `set_positions` places each after the one it names.
    for index in same_as {
        let (source, x, y) = same_as_source(profile, &entries, index)?;
        let source = source.map(|source| entries[source].id.clone());
        let state = &mut entries[index].state;
        (state.x, state.y) = (x, y);
        match source {
            Some(source) => state.mirror_of = Some(source),
            None => notes.push(format!(
                "{}: `same-as` an output that is off; at 0,0 here, as xrandr puts it, and no mirror",
                profile.outputs[index].name
            )),
        }
    }
    // Outputs at one place: xrandr shows the same picture on both, so the
    // later one is a mirror of the first there.
    for index in 0..entries.len() {
        let state = &entries[index].state;
        if !state.enabled || state.mirror_of.is_some() {
            continue;
        }
        let at = (state.x, state.y);
        let first = entries[..index].iter().position(|other| {
            other.state.enabled
                && other.state.mirror_of.is_none()
                && (other.state.x, other.state.y) == at
        });
        if let Some(first) = first {
            notes.push(format!(
                "{}: at {},{} as {} is; a mirror of it here",
                profile.outputs[index].name, at.0, at.1, profile.outputs[first].name
            ));
            entries[index].state.mirror_of = Some(entries[first].id.clone());
        }
    }
    Ok(Layout {
        entries,
        ..Layout::default()
    })
}

/// Where the output at `index` of `profile`, which is `same-as` another,
/// goes, as xrandr's `set_positions` places it: on the output it names,
/// and a mirror of it; or, when that one is `same-as` a third, wherever
/// that one goes, a mirror of the same source, since a mirror of a mirror
/// is none in a layout; or at 0,0 and no mirror when the one it names is
/// off. A name of no output in the profile, or a loop, skips the profile.
fn same_as_source(
    profile: &AutorandrProfile,
    entries: &[OutputEntry],
    index: usize,
) -> Result<(Option<usize>, i32, i32), String> {
    let mut at = index;
    // Each step goes to another output, so more steps than outputs is a
    // loop.
    for _ in 0..profile.outputs.len() {
        let name = &profile.outputs[at].name;
        let Some(Some(source)) = profile.outputs[at].option("same-as") else {
            return Err(format!("{name}: `same-as` names no output"));
        };
        let next = profile
            .outputs
            .iter()
            .position(|output| output.name == source)
            .filter(|&next| next != at)
            .ok_or_else(|| {
                format!("{name}: `same-as {source}` names no other output in the profile")
            })?;
        let state = &entries[next].state;
        if !state.enabled {
            // xrandr's `set_positions`: placed at 0,0 when the one it
            // names has no mode. A chain through it starts there.
            return Ok(if at == index {
                (None, 0, 0)
            } else {
                (Some(at), 0, 0)
            });
        }
        if profile.outputs[next].option("same-as").is_none() {
            return Ok((Some(next), state.x, state.y));
        }
        at = next;
    }
    Err(format!(
        "{}: its `same-as` outputs name each other in a loop, which xrandr refuses",
        profile.outputs[index].name
    ))
}

/// The identity `output`'s `setup` fingerprint gives: the monitor's own,
/// from its EDID, when that has a serial; else its connector, which is
/// said, as when there is no EDID to read (none, an older autorandr's MD5
/// of it, or a pattern).
fn identity(output: &AutorandrOutput, notes: &mut Vec<String>) -> OutputId {
    let name = &output.name;
    let edid = output.edid.as_str();
    let connector = OutputId::Connector(name.clone());
    let why = if edid.starts_with(EDID_UNAVAILABLE) {
        "autorandr read no EDID for it".to_owned()
    } else if edid.contains('*') {
        "setup matches it by an EDID pattern, so any monitor on that connector matches".to_owned()
    } else if edid.len() == MD5_HEX {
        "setup holds an older autorandr's hash of its EDID, not the EDID".to_owned()
    } else {
        match hex(edid).map(|bytes| Edid::parse(&bytes).map_err(|error| error.to_string())) {
            Ok(Ok(parsed)) => {
                let id = parsed.output_id(name);
                if id != connector {
                    return id;
                }
                "its EDID gives no serial".to_owned()
            }
            Ok(Err(error)) | Err(error) => format!("its EDID cannot be read ({error})"),
        }
    };
    notes.push(format!("{name}: {why}; named by its connector here"));
    connector
}

/// `text` as bytes, two hex digits each.
fn hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("an odd number of hex digits".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|at| {
            text.get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| "not hex".to_owned())
        })
        .collect()
}

/// What `output`'s options set, with `now` (the output as it is, when
/// connected) for a mode given without a rate.
fn state(
    output: &AutorandrOutput,
    id: &OutputId,
    now: Option<&OutputEntry>,
    notes: &mut Vec<String>,
) -> Result<OutputState, String> {
    let name = &output.name;
    let given = |key: &str| output.option(key).map(|value| value.unwrap_or_default());
    if output.option("off").is_some() {
        return Ok(OutputState {
            enabled: false,
            ..OutputState::default()
        });
    }
    if let Some((key, _)) = output
        .options
        .iter()
        .find(|(key, _)| REFUSED.contains(&key.as_str()))
    {
        return Err(format!(
            "{name}: `{key}` is not something a dsplr layout can say"
        ));
    }
    if let Some(scale) = given("scale").filter(|scale| !is_one_scale(scale)) {
        return Err(format!(
            "{name}: `scale {scale}` is a RandR transform dsplr does not set"
        ));
    }
    if let Some(transform) = given("transform").filter(|matrix| !is_identity(matrix)) {
        return Err(format!(
            "{name}: `transform {transform}` is a RandR transform dsplr does not set"
        ));
    }

    let mode_name = given("mode").ok_or_else(|| format!("{name}: on, but names no mode"))?;
    let (width, height) =
        size(mode_name).ok_or_else(|| format!("{name}: mode `{mode_name}` names no size"))?;
    if mode_name != format!("{width}x{height}") {
        notes.push(format!(
            "{name}: mode `{mode_name}` taken as {width}x{height}"
        ));
    }
    let mode = match given("rate") {
        Some(rate) => Mode::new(
            width,
            height,
            hz_text_to_mhz(rate).ok_or_else(|| format!("{name}: rate `{rate}` is not a rate"))?,
        ),
        None => {
            let listed = now
                .and_then(|entry| entry.output.as_ref())
                .ok_or_else(|| {
                    let missing = match id {
                        OutputId::Connector(_) => format!(
                            "no output named {name} is connected now (X drivers and the Wayland desktops name connectors differently)"
                        ),
                        _ => format!("no connected monitor is {id} now"),
                    };
                    format!(
                        "{name}: {missing}, so the rate autorandr leaves to it cannot be had; import again where it is connected under that name"
                    )
                })?;
            let mode = listed
                .modes
                .iter()
                .filter(|mode| (mode.width, mode.height) == (width, height))
                .max_by_key(|mode| mode.refresh_mhz)
                .ok_or_else(|| format!("{name} lists no {width}x{height} mode"))?;
            notes.push(format!(
                "{name}: {width}x{height} names no rate; {mode} here, the highest it lists"
            ));
            Mode {
                handle: None,
                ..mode.clone()
            }
        }
    };
    let (x, y) = match given("pos") {
        Some(pos) => {
            position(pos).ok_or_else(|| format!("{name}: pos `{pos}` is not a position"))?
        }
        // A `same-as` output is placed on its source instead.
        None if output.option("same-as").is_some() => (0, 0),
        None => {
            notes.push(format!("{name}: names no position; 0,0 here"));
            (0, 0)
        }
    };
    let transform =
        transform(given("rotate"), given("reflect")).map_err(|what| format!("{name}: {what}"))?;

    let left_out: Vec<String> = output
        .options
        .iter()
        .filter(|(key, value)| {
            !KNOWN.contains(&key.as_str())
                && !DEFAULTS.contains(&(key.as_str(), value.as_deref().unwrap_or_default()))
        })
        .map(|(key, value)| match value {
            Some(value) => format!("`{key} {value}`"),
            None => format!("`{key}`"),
        })
        .collect();
    if !left_out.is_empty() {
        notes.push(format!(
            "{name}: {} not imported: a dsplr layout has no such setting",
            left_out.join(", ")
        ));
    }
    Ok(OutputState {
        enabled: true,
        mode: Some(mode),
        x,
        y,
        transform,
        primary: output.option("primary").is_some(),
        ..OutputState::default()
    })
}

/// The options [`state`] takes, or refuses, or checks for a default.
const KNOWN: [&str; 10] = [
    "mode",
    "rate",
    "pos",
    "rotate",
    "reflect",
    "primary",
    "off",
    "same-as",
    "scale",
    "transform",
];

/// The size a mode name starts with: `1920x1080`, also of `1920x1080i` or
/// `1920x1080_60.00`.
fn size(mode: &str) -> Option<(u32, u32)> {
    let (width, rest) = leading_number(mode)?;
    let (height, _) = leading_number(rest.strip_prefix('x')?)?;
    Some((width, height))
}

/// The number `text` starts with, and the rest of it.
fn leading_number(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// `pos`'s `XxY`, either of which may be negative.
fn position(pos: &str) -> Option<(i32, i32)> {
    let (x, y) = pos.split_once('x')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}

/// Whether `scale` is xrandr's `--scale` of 1: `1`, or `1x1`, in any
/// spelling of 1.
fn is_one_scale(scale: &str) -> bool {
    scale
        .split('x')
        .all(|part| part.parse::<f64>().is_ok_and(|value| value == 1.0))
}

/// Whether `matrix`, nine numbers apart by commas, is the identity.
fn is_identity(matrix: &str) -> bool {
    let values: Vec<Option<f64>> = matrix
        .split(',')
        .map(|value| value.trim().parse().ok())
        .collect();
    values.len() == 9
        && values
            .iter()
            .enumerate()
            .all(|(at, value)| *value == Some(if at % 4 == 0 { 1.0 } else { 0.0 }))
}

/// The transform xrandr's `--rotate` and `--reflect` make.
///
/// `--rotate left` is `RR_Rotate_90`, `inverted` `RR_Rotate_180`, `right`
/// `RR_Rotate_270` (xrandr's `rotation_name` table); `--reflect x`, `y` and
/// `xy` are `RR_Reflect_X`, `RR_Reflect_Y` and both. dsplr's RandR planner
/// sets a flipped transform as `RR_Reflect_X` with the same rotation
/// (`dsplr_backend::randr::plan::rotation`, checked against wlroots). The
/// X server reflects in the turned frame, either axis alike
/// (`RRTransformCompute`), so `RR_Reflect_Y` is `RR_Reflect_X` turned half
/// a turn further, and both together are half a turn and no reflection.
fn transform(rotate: Option<&str>, reflect: Option<&str>) -> Result<Transform, String> {
    let turns: u32 = match rotate.unwrap_or("normal") {
        "normal" => 0,
        "left" => 1,
        "inverted" => 2,
        "right" => 3,
        other => return Err(format!("`rotate {other}` is not a rotation")),
    };
    let (x, y) = match reflect.unwrap_or("normal") {
        "normal" => (false, false),
        "x" => (true, false),
        "y" => (false, true),
        "xy" => (true, true),
        other => return Err(format!("`reflect {other}` is not a reflection")),
    };
    let turns = if y { (turns + 2) % 4 } else { turns };
    let flipped = if x != y { 4 } else { 0 };
    Ok(Transform::from_wayland(flipped + turns).expect("0 to 7"))
}

#[cfg(test)]
mod tests;
