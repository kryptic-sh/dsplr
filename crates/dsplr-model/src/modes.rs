//! Choosing a mode among the modes an output lists: the rules `dsplr set`
//! and the GUI share.

use crate::{Mode, ModeSpec};

/// The mode an output turned on without one takes: the one it prefers, else
/// its largest, at its highest refresh (Hyprland's `highres`).
pub fn default_mode(listed: &[Mode]) -> Option<&Mode> {
    listed.iter().find(|mode| mode.preferred).or_else(|| {
        listed.iter().max_by_key(|mode| {
            (
                u64::from(mode.width) * u64::from(mode.height),
                mode.refresh_mhz,
            )
        })
    })
}

/// The mode `asked` names, given the output's `listed` modes and the mode it
/// is set to now (`current`):
///
/// - with a refresh, the listed mode at that size closest to it within
///   [`REFRESH_TOLERANCE_MHZ`](crate::REFRESH_TOLERANCE_MHZ), else a custom
///   mode of exactly that when `custom_modes`;
/// - without one, the size at `current`'s refresh when listed, so a new size
///   keeps the refresh, else the highest refresh listed at that size.
///
/// `None` when no such mode can be had; the caller says why.
pub fn pick_mode(
    asked: ModeSpec,
    listed: &[Mode],
    current: Option<&Mode>,
    custom_modes: bool,
) -> Option<Mode> {
    let at_size = || {
        listed
            .iter()
            .filter(|mode| (mode.width, mode.height) == (asked.width, asked.height))
    };
    match asked.refresh_mhz {
        Some(refresh_mhz) => {
            let wanted = Mode::new(asked.width, asked.height, refresh_mhz);
            let closest = listed
                .iter()
                .filter(|mode| mode.matches(&wanted))
                .min_by_key(|mode| mode.refresh_mhz.abs_diff(refresh_mhz));
            match closest {
                Some(mode) => Some(mode.clone()),
                None => custom_modes.then_some(wanted),
            }
        }
        None => current
            .and_then(|current| {
                let same_refresh = Mode::new(asked.width, asked.height, current.refresh_mhz);
                at_size().find(|mode| mode.matches(&same_refresh))
            })
            .or_else(|| at_size().max_by_key(|mode| mode.refresh_mhz))
            .cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{default_mode, pick_mode};
    use crate::{Mode, ModeHandle, ModeSpec};

    fn spec(width: u32, height: u32, refresh_mhz: Option<u32>) -> ModeSpec {
        ModeSpec {
            width,
            height,
            refresh_mhz,
        }
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
        let mut handled = modes();
        handled[1].handle = Some(ModeHandle(1));
        let picked = pick_mode(spec(2560, 1440, Some(144_000)), &handled, None, false);
        assert_eq!(
            picked,
            Some(handled[1].clone()),
            "within the tolerance, handle kept"
        );
        assert!(picked.unwrap().handle.is_some());
    }

    #[test]
    fn a_refresh_takes_the_closest_listed_mode_not_the_first_within_tolerance() {
        let listed = [
            Mode::new(1920, 1080, 60_004),
            Mode::new(1920, 1080, 60_001),
            Mode::new(1920, 1080, 59_997),
        ];
        assert_eq!(
            pick_mode(spec(1920, 1080, Some(60_000)), &listed, None, false),
            Some(listed[1].clone())
        );
    }

    #[test]
    fn a_size_alone_keeps_the_refresh_or_takes_the_highest() {
        let at_120 = Mode::new(2560, 1440, 120_000);
        let at_60 = Mode::new(2560, 1440, 59_951);
        let asked = spec(1920, 1080, None);
        assert_eq!(
            pick_mode(asked, &modes(), Some(&at_120), false),
            Some(Mode::new(1920, 1080, 120_000))
        );
        assert_eq!(
            pick_mode(asked, &modes(), Some(&at_60), false),
            Some(Mode::new(1920, 1080, 120_000)),
            "59.951 is not listed at 1920x1080: the highest"
        );
        assert_eq!(
            pick_mode(spec(2560, 1440, None), &modes(), None, false),
            Some(Mode::new(2560, 1440, 143_998))
        );
    }

    #[test]
    fn a_mode_not_listed_is_a_custom_mode_only_with_a_refresh_and_custom_modes() {
        let custom = spec(1280, 720, Some(50_000));
        assert_eq!(pick_mode(custom, &modes(), None, false), None);
        assert_eq!(
            pick_mode(custom, &modes(), None, true),
            Some(Mode::new(1280, 720, 50_000))
        );
        assert_eq!(pick_mode(spec(1280, 720, None), &modes(), None, true), None);
    }

    #[test]
    fn an_output_turned_on_without_a_mode_takes_its_preferred_or_largest() {
        let mut listed = modes();
        assert_eq!(default_mode(&listed), Some(&Mode::new(2560, 1440, 143_998)));
        listed[3].preferred = true;
        assert_eq!(default_mode(&listed).map(|m| m.refresh_mhz), Some(60_000));
        assert_eq!(default_mode(&[]), None);
    }
}
