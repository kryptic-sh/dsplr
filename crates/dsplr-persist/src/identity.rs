//! How sway and niri name a monitor besides its connector: its make, model
//! and serial in one string, `Unknown` standing for a part it does not
//! report. Both compositors name outputs this way, so their writers share
//! it.

use dsplr_model::{OutputEntry, OutputId};

/// The identifier `make model serial`, with `Unknown` for a part not
/// reported (`swaymsg -t get_outputs` on headless sway 1.12 shows all three
/// as `Unknown`, and a config naming `"Unknown Unknown Unknown"` applies to
/// those outputs; niri 26.04 names its winit output `Smithay Winit Unknown`
/// in `niri msg outputs`, and a config naming it so applies to it). Taken
/// from an EDID identity, else from the make, model and serial its output
/// reported, so a monitor without a serial has one too (`Make Model
/// Unknown`). `None` for an entry with neither: a connector or a name read
/// from a file.
pub fn identifier(entry: &OutputEntry) -> Option<String> {
    let (make, model, serial) = match (&entry.id, &entry.output) {
        (
            OutputId::Edid {
                make,
                model,
                serial,
            },
            _,
        ) => (make, model, serial),
        (_, Some(output)) => (&output.make, &output.model, &output.serial),
        _ => return None,
    };
    let part = |text: &str| {
        let text = text.trim();
        if text.is_empty() { "Unknown" } else { text }.to_owned()
    };
    Some(format!("{} {} {}", part(make), part(model), part(serial)))
}

/// The outputs that settings kept from an old file are for, each once, from
/// the names those settings were written under: a name with whitespace in
/// it is an identifier ([`identifier`]), anything else a connector.
pub fn kept_ids<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<OutputId> {
    let mut ids = Vec::new();
    for name in names {
        let id = if name.contains(char::is_whitespace) {
            OutputId::Description(name.to_owned())
        } else {
            OutputId::Connector(name.to_owned())
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use dsplr_model::OutputId;

    use super::kept_ids;

    #[test]
    fn kept_names_are_identifiers_when_they_have_spaces_each_once() {
        assert_eq!(
            kept_ids(["DP-9", "Dell Inc. U2720Q X1", "DP-9"]),
            [
                OutputId::Connector("DP-9".into()),
                OutputId::Description("Dell Inc. U2720Q X1".into()),
            ]
        );
    }
}
