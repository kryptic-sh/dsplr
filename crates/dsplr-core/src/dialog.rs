//! The confirm dialog shown on every output after an apply that can hide or
//! move a screen (`docs/plan/06-gui.md`, "Confirm dialog"): its draw list
//! and buttons, and which output each of the window system's monitors is.

use std::time::Duration;

use dsplr_model::Layout;

use crate::draw::{Bounds, DrawList, Point};
use crate::editor::on_canvas;
use crate::panel::name;
use crate::{Size, Theme};

/// The dialog's size in logical pixels.
pub const DIALOG_SIZE: Size = Size {
    width: 440.0,
    height: 168.0,
};
/// What the dialog asks.
pub const QUESTION: &str = "Keep these display settings?";
const PAD: f32 = 20.0;
const TITLE_SIZE: f32 = 17.0;
const TEXT_SIZE: f32 = 13.0;
const LINE: f32 = 22.0;
const BUTTON_WIDTH: f32 = 120.0;
const BUTTON_HEIGHT: f32 = 32.0;
const BUTTON_GAP: f32 = 12.0;
const BORDER: f32 = 2.0;

/// The dialog's answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Keep,
    Revert,
}

/// A dialog's frame: its draw list, and where each button is.
#[derive(Debug, Clone, PartialEq)]
pub struct DialogFrame {
    pub list: DrawList,
    pub buttons: Vec<(Bounds, Choice)>,
}

impl DialogFrame {
    /// The button at `point`.
    pub fn hit(&self, point: Point) -> Option<Choice> {
        self.buttons
            .iter()
            .find(|(bounds, _)| bounds.contains(point))
            .map(|&(_, choice)| choice)
    }
}

/// The dialog in a window of `size`, on the output named `output` (`None`
/// when the monitor matched none), reverting in `left`: the question, the
/// output, the countdown in whole seconds rounded up, and Keep (Enter) and
/// Revert (Esc).
pub fn draw_dialog(output: Option<&str>, left: Duration, size: Size, theme: &Theme) -> DialogFrame {
    let mut list = DrawList::default();
    let window = Bounds::new(0.0, 0.0, size.width, size.height);
    list.fill(window, theme.surface);
    list.outline(window, BORDER, theme.accent);
    let clip = window.inset(BORDER);
    let mut y = PAD;
    list.text(Point::new(PAD, y), QUESTION, TITLE_SIZE, theme.text, clip);
    y += LINE + 4.0;
    if let Some(output) = output {
        list.text(Point::new(PAD, y), output, TEXT_SIZE, theme.text_dim, clip);
        y += LINE;
    }
    let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    list.text(
        Point::new(PAD, y),
        format!("Reverting in {seconds} s"),
        TEXT_SIZE,
        theme.pending,
        clip,
    );

    let top = size.height - PAD - BUTTON_HEIGHT;
    let revert_x = size.width - PAD - BUTTON_WIDTH;
    let keep_x = revert_x - BUTTON_GAP - BUTTON_WIDTH;
    let mut buttons = Vec::new();
    for (choice, x, label, fill, text) in [
        (
            Choice::Keep,
            keep_x,
            "Keep (Enter)",
            theme.accent,
            theme.surface,
        ),
        (
            Choice::Revert,
            revert_x,
            "Revert (Esc)",
            theme.output,
            theme.text,
        ),
    ] {
        let bounds = Bounds::new(x, top, BUTTON_WIDTH, BUTTON_HEIGHT);
        list.fill(bounds, fill);
        list.outline(bounds, 1.0, theme.border);
        list.text(
            Point::new(bounds.x + 14.0, bounds.y + (BUTTON_HEIGHT - 16.0) / 2.0),
            label,
            TEXT_SIZE,
            text,
            bounds,
        );
        buttons.push((bounds, choice));
    }
    DialogFrame { list, buttons }
}

/// One of the window system's monitors, as dsplr matches it to an output.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// The connector the window system names it by, when it says (crcbl's
    /// `MonitorInfo::connector`, not released yet: always `None` today).
    pub connector: Option<String>,
    /// Where its top-left corner may be in the desktop's logical
    /// coordinates, most likely first: the window system may report it in
    /// more than one unit, and which is not said (`confirm::screen` in
    /// dsplr-gui).
    pub positions: Vec<Point>,
}

/// The name of the output of `layout` each of `screens` shows, `None` where
/// none matches: by connector where the screen has one, else an enabled
/// output whose logical position is one of the screen's to within a pixel,
/// its positions tried in order. A size is not compared: the window system
/// may report a monitor's untransformed mode, which a turned output's
/// logical size is not. Two outputs at one place (a mirror) name the first.
pub fn match_screens(screens: &[Screen], layout: &Layout) -> Vec<Option<String>> {
    screens
        .iter()
        .map(|screen| {
            let by_connector = screen.connector.as_deref().and_then(|connector| {
                layout
                    .entries
                    .iter()
                    .find(|entry| entry.connector() == Some(connector))
            });
            let by_place = || {
                screen.positions.iter().find_map(|&at| {
                    layout.entries.iter().find(|entry| {
                        on_canvas(&entry.state)
                            && near(entry.state.x, at.x)
                            && near(entry.state.y, at.y)
                    })
                })
            };
            by_connector.or_else(by_place).map(name)
        })
        .collect()
}

fn near(layout: i32, screen: f32) -> bool {
    (layout as f32 - screen).abs() <= 1.0
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Choice, DIALOG_SIZE, Screen, draw_dialog, match_screens};
    use crate::fixtures::{assert_snapshot, three_monitors};
    use crate::{Point, Theme};

    #[test]
    fn the_dialog_on_a_matched_output() {
        let frame = draw_dialog(
            Some("DP-1"),
            Duration::from_millis(14_200),
            DIALOG_SIZE,
            &Theme::tokyo_night(),
        );
        let buttons: String = frame
            .buttons
            .iter()
            .map(|(bounds, choice)| format!("button {bounds} {choice:?}\n"))
            .collect();
        let text = frame.list.to_string() + &buttons;
        assert!(
            text.contains("\"Reverting in 15 s\""),
            "rounded up:\n{text}"
        );
        assert_snapshot("dialog", &text);
    }

    #[test]
    fn an_unmatched_monitor_still_gets_the_dialog_and_its_buttons() {
        let frame = draw_dialog(None, Duration::ZERO, DIALOG_SIZE, &Theme::tokyo_night());
        let text = frame.list.to_string();
        assert!(text.contains("Keep these display settings?"));
        assert!(text.contains("\"Reverting in 0 s\""));
        let centre = |choice| {
            let (b, _) = frame.buttons.iter().find(|(_, c)| *c == choice).unwrap();
            Point::new(b.x + b.width / 2.0, b.y + b.height / 2.0)
        };
        assert_eq!(frame.hit(centre(Choice::Keep)), Some(Choice::Keep));
        assert_eq!(frame.hit(centre(Choice::Revert)), Some(Choice::Revert));
        assert_eq!(frame.hit(Point::new(5.0, 5.0)), None);
    }

    #[test]
    fn monitors_match_outputs_by_place_or_by_connector() {
        let layout = three_monitors().layout;
        // DP-1 at 0,360; DP-2 at 1920,0; HDMI-A-1 off.
        let screen = |positions: &[(f32, f32)]| Screen {
            connector: None,
            positions: positions.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        };
        let screens = [
            screen(&[(1920.0, 0.0)]),
            // Off by the rounding of a fractional scale.
            screen(&[(0.5, 359.6)]),
            screen(&[(5000.0, 0.0)]),
            // The first guess wrong, the second right: a position reported
            // in another unit than the one tried first.
            screen(&[(3840.0, 0.0), (1920.0, 0.0)]),
            Screen {
                connector: Some("HDMI-A-1".into()),
                positions: vec![Point::new(9999.0, 0.0)],
            },
        ];
        assert_eq!(
            match_screens(&screens, &layout),
            [
                Some("DP-2".to_owned()),
                Some("DP-1".to_owned()),
                None,
                Some("DP-2".to_owned()),
                Some("HDMI-A-1".to_owned()),
            ]
        );
    }
}
