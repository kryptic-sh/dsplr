//! sway's `outputs`: one `output <name> { … }` block per output, written as
//! sway-output(5) (sway 1.12) describes the commands and read back in the
//! same form. The main sway config loads it with `include outputs`.

mod read;
mod save;
mod write;

use dsplr_model::{Layout, OutputEntry, OutputId, Transform};

use crate::identity::identifier;

pub use read::{OutputsBlock, ReadError, read_outputs};
pub use save::{INCLUDE_LINE, SaveError, save};
pub use write::{WriteError, output_block, output_blocks};

/// sway's name for each transform. sway turns the quarter turns clockwise
/// and `wl_output.transform` counter-clockwise, so sway's `90` is
/// [`Transform::Rotate270`], and its `flipped-90` is
/// [`Transform::Flipped270`]: each `transform` in a config, applied by a
/// headless sway 1.12 on reload, read back through wlr-output-management as
/// the transform paired with it here.
const TRANSFORMS: [(Transform, &str); 8] = [
    (Transform::Normal, "normal"),
    (Transform::Rotate90, "270"),
    (Transform::Rotate180, "180"),
    (Transform::Rotate270, "90"),
    (Transform::Flipped, "flipped"),
    (Transform::Flipped90, "flipped-270"),
    (Transform::Flipped180, "flipped-180"),
    (Transform::Flipped270, "flipped-90"),
];

/// sway's word for `transform`.
fn transform_name(transform: Transform) -> &'static str {
    TRANSFORMS
        .iter()
        .find(|(each, _)| *each == transform)
        .map(|(_, name)| *name)
        .expect("every transform has a sway name")
}

/// The words an `output` command sets that dsplr writes, in every spelling
/// sway takes. A setting with another word is not dsplr's to replace.
const SETTINGS: [&str; 10] = [
    "enable",
    "disable",
    "mode",
    "resolution",
    "res",
    "position",
    "pos",
    "scale",
    "transform",
    "adaptive_sync",
];

/// The name `entry`'s block is written under: sway's identifier when the
/// monitor has a serial, no other entry of `layout` has that identifier, and
/// it can be written into a config ([`quoted`]), so the settings follow the
/// monitor to another port; else its connector, so two monitors reporting
/// the same identifier each keep their own. A name read from a file (a
/// [`OutputId::Description`]) is written as it was read. `None` when there
/// is no usable name.
fn output_name(layout: &Layout, entry: &OutputEntry) -> Option<String> {
    if let OutputId::Description(name) = &entry.id {
        return Some(name.trim().to_owned()).filter(|name| !name.is_empty());
    }
    let own = identifier(entry);
    let usable = own.as_ref().filter(|own| {
        matches!(entry.id, OutputId::Edid { .. })
            && quoted(own).is_some()
            && layout
                .entries
                .iter()
                .filter(|other| identifier(other).as_ref() == Some(own))
                .count()
                == 1
    });
    usable
        .cloned()
        .or_else(|| entry.connector().map(str::to_owned))
        .or(own)
}

/// Whether sway applies a config for `name` to `live`: its connector or its
/// identifier, compared whole, as sway compares them (on headless sway 1.12,
/// `"Unknown Unknown"` and `HEADLESS` named no output).
fn names(name: &str, live: &OutputEntry) -> bool {
    live.connector() == Some(name) || identifier(live).as_deref() == Some(name)
}

/// `name` as a word of a sway command: as it is when it is plain, else in
/// double quotes. `None` for a name sway would not read back as it is: one
/// with a quote, a backslash, a `$` (sway's variables) or a control
/// character in it.
fn quoted(name: &str) -> Option<String> {
    if name.is_empty()
        || name
            .chars()
            .any(|c| matches!(c, '"' | '\'' | '\\' | '$') || c.is_control())
    {
        return None;
    }
    let plain = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
    Some(if plain {
        name.to_owned()
    } else {
        format!("\"{name}\"")
    })
}

/// `line` split into words as sway splits a command: at whitespace, with a
/// `"…"` or `'…'` span kept as one word without its quotes. `None` for a
/// line with a quote left open.
fn words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    for c in line.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => word.get_or_insert_default().push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            None if c.is_whitespace() => words.extend(word.take()),
            None => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    words.extend(word);
    Some(words)
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Layout, Output, OutputEntry, OutputId, OutputState, Transform};

    use super::{TRANSFORMS, identifier, names, output_name, transform_name, words};

    pub(super) fn entry(id: OutputId, connector: &str) -> OutputEntry {
        OutputEntry {
            id,
            output: Some(Output {
                connector: connector.into(),
                description: String::new(),
                physical_size_mm: None,
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                modes: vec![],
            }),
            state: OutputState::default(),
        }
    }

    #[test]
    fn a_monitor_with_a_serial_is_named_by_sway_s_identifier() {
        let aoc = entry(
            OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
            "HDMI-A-1",
        );
        let panel = entry(OutputId::Connector("eDP-1".into()), "eDP-1");
        let layout = Layout {
            entries: vec![aoc.clone(), panel.clone()],
        };
        assert_eq!(
            output_name(&layout, &aoc).as_deref(),
            Some("AOC Q27G4 SERIAL0000001")
        );
        assert_eq!(output_name(&layout, &panel).as_deref(), Some("eDP-1"));
    }

    #[test]
    fn a_missing_make_or_model_is_unknown_as_sway_writes_it() {
        let headless = entry(OutputId::new("", " ", "X1", "HEADLESS-1"), "HEADLESS-1");
        let layout = Layout {
            entries: vec![headless.clone()],
        };
        assert_eq!(
            output_name(&layout, &headless).as_deref(),
            Some("Unknown Unknown X1")
        );
    }

    /// Two monitors with one identifier would both take whichever block
    /// sway reads last.
    #[test]
    fn monitors_sharing_an_identifier_are_named_by_connector() {
        let twin =
            |connector: &str| entry(OutputId::new("AOC", "Q27G4", "0", connector), connector);
        let layout = Layout {
            entries: vec![twin("DP-1"), twin("DP-2")],
        };
        assert_eq!(
            output_name(&layout, &layout.entries[0]).as_deref(),
            Some("DP-1")
        );
        assert_eq!(
            output_name(&layout, &layout.entries[1]).as_deref(),
            Some("DP-2")
        );
    }

    /// sway reads `'`, `"`, `\\` and `$` its own way, so a monitor whose
    /// identifier holds one is named by its connector.
    #[test]
    fn an_identifier_sway_could_not_read_back_falls_back_to_the_connector() {
        for model in ["27\" panel", "it's", "a\\b", "$mon"] {
            let monitor = entry(OutputId::new("Acme", model, "1", "DP-1"), "DP-1");
            let layout = Layout {
                entries: vec![monitor.clone()],
            };
            assert_eq!(
                output_name(&layout, &monitor).as_deref(),
                Some("DP-1"),
                "{model}"
            );
        }
    }

    /// A monitor without a serial is still `Make Model Unknown` to sway,
    /// though it is named by its connector.
    #[test]
    fn a_monitor_without_a_serial_has_an_identifier_but_is_named_by_connector() {
        let mut panel = entry(OutputId::Connector("eDP-1".into()), "eDP-1");
        let output = panel.output.as_mut().unwrap();
        (output.make, output.model) = ("BOE".into(), "0x0AFC".into());
        let layout = Layout {
            entries: vec![panel.clone()],
        };
        assert_eq!(identifier(&panel).as_deref(), Some("BOE 0x0AFC Unknown"));
        assert_eq!(output_name(&layout, &panel).as_deref(), Some("eDP-1"));
        assert!(names("BOE 0x0AFC Unknown", &panel));
        assert!(names("eDP-1", &panel));
        assert!(!names("BOE 0x0AFC", &panel));
    }

    #[test]
    fn a_name_read_from_a_file_is_kept_and_a_blank_one_is_none() {
        let read = OutputEntry {
            id: OutputId::Description("Dell Inc. U2720Q ABC".into()),
            output: None,
            state: OutputState::default(),
        };
        let layout = Layout::default();
        assert_eq!(
            output_name(&layout, &read).as_deref(),
            Some("Dell Inc. U2720Q ABC")
        );
        let blank = OutputEntry {
            id: OutputId::Description(" ".into()),
            ..read
        };
        assert_eq!(output_name(&layout, &blank), None);
    }

    #[test]
    fn sway_names_the_quarter_turns_the_other_way_round() {
        assert_eq!(transform_name(Transform::Rotate90), "270");
        assert_eq!(transform_name(Transform::Rotate270), "90");
        assert_eq!(transform_name(Transform::Flipped90), "flipped-270");
        assert_eq!(transform_name(Transform::Flipped270), "flipped-90");
        assert_eq!(transform_name(Transform::Rotate180), "180");
        // Each name once, so the table reads back.
        for (_, name) in TRANSFORMS {
            assert_eq!(TRANSFORMS.iter().filter(|(_, n)| *n == name).count(), 1);
        }
    }

    #[test]
    fn words_split_at_whitespace_and_keep_quoted_spans_whole() {
        let split = |line| words(line).unwrap();
        assert_eq!(split("  output  eDP-1 {"), ["output", "eDP-1", "{"]);
        assert_eq!(
            split(r#"output "AOC Q27G4 SER" pos 0 0"#),
            ["output", "AOC Q27G4 SER", "pos", "0", "0"]
        );
        assert_eq!(split("include 'a b'/c"), ["include", "a b/c"]);
        assert_eq!(split(r#"x "" y"#), ["x", "", "y"]);
        assert_eq!(split(""), Vec::<String>::new());
        assert_eq!(words(r#"output "open"#), None);
    }
}
