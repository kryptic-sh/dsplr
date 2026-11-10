//! Pairing the entries of a saved layout (a layout file, a profile) with the
//! connected outputs they name, each with its own.

use crate::{OutputEntry, OutputId};

/// How closely `wanted`, a saved entry, names `live`, a connected output:
/// lower is closer, `None` when it does not name it ([`OutputId::matches`]).
/// The same EDID identity on the same connector is closest, then the same
/// EDID identity or connector, then a description that is the whole of
/// `live`'s, then one that is only its first words.
fn closeness(wanted: &OutputEntry, live: &OutputEntry) -> Option<u8> {
    if !wanted.id.matches(live) {
        return None;
    }
    Some(match &wanted.id {
        OutputId::Edid { .. }
            if wanted.connector().is_some() && wanted.connector() == live.connector() =>
        {
            0
        }
        OutputId::Edid { .. } | OutputId::Connector(_) => 1,
        OutputId::Description(description) => {
            if live.description().as_deref() == Some(description.trim()) {
                2
            } else {
                3
            }
        }
    })
}

/// Gives each entry of `wanted` its own entry of `live` that it names: the
/// index into `live` for each, in `wanted`'s order. An identity that names
/// several outputs (a description shared by two monitors, twins with one
/// EDID identity) takes whichever is left, so a pairing is found whenever
/// one exists. It is a maximum bipartite matching by augmenting paths
/// (Kuhn's algorithm). Where several pairings exist, the one chosen is
/// found by placing the entries with the closest match first, each trying
/// its closest free output first and moving an entry already placed only
/// when there is no free one, ties in the order listed: so twins keep their
/// connectors when those are connected, and otherwise pair in the order
/// listed. This is greedy by closeness, not the pairing of least total
/// distance: an entry that must displace another tries its closest outputs
/// first, even when displacing a different one would cost nothing.
///
/// # Errors
///
/// The indices into `wanted`, in order, of the entries left without an
/// output: those naming none, and those whose outputs all went to others
/// in the largest pairing.
pub fn assign(wanted: &[OutputEntry], live: &[OutputEntry]) -> Result<Vec<usize>, Vec<usize>> {
    let assigned = pair(wanted, live);
    let unassigned: Vec<usize> = (0..wanted.len())
        .filter(|&index| assigned[index].is_none())
        .collect();
    if unassigned.is_empty() {
        Ok(assigned.into_iter().flatten().collect())
    } else {
        Err(unassigned)
    }
}

/// [`assign`]'s pairing as far as it goes: for each entry of `wanted`, in
/// order, the index into `live` it was given, `None` for one left without.
/// For pairing two layouts where outputs may have come and gone, such as
/// two reads of the desktop.
pub fn pair(wanted: &[OutputEntry], live: &[OutputEntry]) -> Vec<Option<usize>> {
    let candidates: Vec<Vec<usize>> = wanted
        .iter()
        .map(|entry| {
            let mut ranked: Vec<(u8, usize)> = live
                .iter()
                .enumerate()
                .filter_map(|(index, live)| closeness(entry, live).map(|rank| (rank, index)))
                .collect();
            ranked.sort_unstable();
            ranked.into_iter().map(|(_, index)| index).collect()
        })
        .collect();
    let mut order: Vec<usize> = (0..wanted.len()).collect();
    order.sort_by_key(|&index| {
        let closest = candidates[index]
            .first()
            .and_then(|&live_index| closeness(&wanted[index], &live[live_index]));
        (closest.unwrap_or(u8::MAX), index)
    });
    let mut owner: Vec<Option<usize>> = vec![None; live.len()];
    for index in order {
        let mut seen = vec![false; live.len()];
        augment(index, &candidates, &mut owner, &mut seen);
    }
    let mut assigned = vec![None; wanted.len()];
    for (live_index, owner) in owner.into_iter().enumerate() {
        if let Some(index) = owner {
            assigned[index] = Some(live_index);
        }
    }
    assigned
}

/// Finds `index` an output along an augmenting path: its closest free
/// candidate, else one whose owner can move to another output, so an entry
/// is moved only when it has to be. `seen` marks the outputs this search
/// has tried.
fn augment(
    index: usize,
    candidates: &[Vec<usize>],
    owner: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    if let Some(&free) = candidates[index]
        .iter()
        .find(|&&live_index| owner[live_index].is_none())
    {
        owner[free] = Some(index);
        return true;
    }
    for &live_index in &candidates[index] {
        if seen[live_index] {
            continue;
        }
        seen[live_index] = true;
        let free = match owner[live_index] {
            None => true,
            Some(other) => augment(other, candidates, owner, seen),
        };
        if free {
            owner[live_index] = Some(index);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{assign, pair};
    use crate::{Output, OutputEntry, OutputId, OutputState};

    fn live(connector: &str, description: &str, serial: &str) -> OutputEntry {
        OutputEntry {
            id: OutputId::new("AOC", "Q27G4", serial, connector),
            output: Some(Output {
                connector: connector.into(),
                description: description.into(),
                physical_size_mm: None,
                make: String::new(),
                model: String::new(),
                serial: serial.into(),
                modes: vec![],
            }),
            state: OutputState::default(),
        }
    }

    fn wanted(id: OutputId) -> OutputEntry {
        OutputEntry {
            id,
            output: None,
            state: OutputState::default(),
        }
    }

    fn desc(text: &str) -> OutputEntry {
        wanted(OutputId::Description(text.into()))
    }

    fn connector(text: &str) -> OutputEntry {
        wanted(OutputId::Connector(text.into()))
    }

    #[test]
    fn a_description_naming_two_outputs_takes_the_one_left() {
        // Listed first, HEADLESS-2 is the description's first match too.
        let now = [
            live("HEADLESS-2", "Headless output 2", ""),
            live("HEADLESS-1", "Headless output 1", ""),
        ];
        assert_eq!(
            assign(&[desc("Headless output"), connector("HEADLESS-2")], &now),
            Ok(vec![1, 0])
        );
        assert_eq!(
            assign(&[connector("HEADLESS-2"), desc("Headless output")], &now),
            Ok(vec![0, 1])
        );
        // The whole description goes first, whatever the order listed.
        assert_eq!(
            assign(&[desc("Headless output"), desc("Headless output 2")], &now),
            Ok(vec![1, 0])
        );
    }

    #[test]
    fn twins_each_get_one_keeping_their_connectors() {
        let now = [
            live("DP-1", "AOC Q27G4 SAME", "SAME"),
            live("DP-2", "AOC Q27G4 SAME", "SAME"),
        ];
        let twin = |connector: Option<&str>| OutputEntry {
            output: connector.map(|connector| Output {
                connector: connector.into(),
                ..now[0].output.clone().unwrap()
            }),
            ..wanted(now[0].id.clone())
        };
        assert_eq!(
            assign(&[twin(Some("DP-2")), twin(Some("DP-1"))], &now),
            Ok(vec![1, 0])
        );
        assert_eq!(assign(&[twin(None), twin(None)], &now), Ok(vec![0, 1]));
        // Saved on other ports: paired in the order listed.
        assert_eq!(
            assign(&[twin(Some("HDMI-A-1")), twin(Some("HDMI-A-2"))], &now),
            Ok(vec![0, 1])
        );
        // An augmenting path: the twin placed first gives DP-1 up to the
        // entry that can have nothing else.
        assert_eq!(
            assign(
                &[twin(None), wanted(OutputId::Connector("DP-1".into()))],
                &now
            ),
            Ok(vec![1, 0])
        );
        // Three twins for two outputs: the last is left over.
        assert_eq!(
            assign(&[twin(None), twin(None), twin(None)], &now),
            Err(vec![2])
        );
    }

    #[test]
    fn a_whole_description_beats_its_first_words() {
        let now = [
            live("DP-1", "AOC Q27G4 SER1", "SER1"),
            live("DP-2", "AOC Q27G4", ""),
        ];
        assert_eq!(assign(&[desc("AOC Q27G4")], &now), Ok(vec![1]));
        // Placed first, the closer match keeps its output, though the entry
        // listed before it, naming both alike, would take that one first.
        let now = [now[1].clone(), now[0].clone()];
        assert_eq!(
            assign(&[desc("AOC"), desc("AOC Q27G4")], &now),
            Ok(vec![1, 0])
        );
    }

    #[test]
    fn what_names_no_output_or_only_taken_ones_is_left_over() {
        let now = [live("DP-1", "AOC Q27G4 SER1", "SER1")];
        assert_eq!(
            assign(&[connector("DP-9"), connector("DP-1"), desc("AOC")], &now),
            Err(vec![0, 2])
        );
        assert_eq!(assign(&[], &now), Ok(vec![]));
        assert_eq!(
            pair(&[connector("DP-9"), connector("DP-1"), desc("AOC")], &now),
            [None, Some(0), None],
            "the pairing as far as it goes"
        );
        assert_eq!(assign(&[connector("DP-1")], &[]), Err(vec![0]));
    }
}
