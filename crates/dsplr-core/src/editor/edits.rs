//! The edits the keys and the property panel make to one output's pending
//! settings (`docs/plan/06-gui.md`). Each goes through [`Editor::edit`], so
//! each is one undo step, and each returns whether it changed anything.
//!
//! Settings other than whether it is on are not changed on an output that is
//! off, as `dsplr set` refuses them: no desktop applies them and validation
//! does not look at them.

use dsplr_model::{
    AdaptiveSync, Capabilities, Mode, ModeSpec, OutputId, Rect, SCALE_EPSILON, Transform,
    default_mode, is_valid_scale, pick_mode,
};

use super::{Editor, on_canvas};

/// How far `h j k l` move an output, in layout pixels: one snap step.
pub const NUDGE_STEP: i32 = 10;

/// Scales offered on a backend with fractional scaling when the mode lists
/// none of its own: quarter steps.
const FRACTIONAL_SCALES: [f64; 11] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0];
/// Scales offered on a backend with whole-number scales only.
const WHOLE_SCALES: [f64; 3] = [1.0, 2.0, 3.0];

/// A direction on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The scales to offer for `mode` on a backend with `capabilities`: the ones
/// the mode lists (GNOME), else quarter steps where the backend takes
/// fractional scales, else whole numbers.
pub fn scale_choices(mode: Option<&Mode>, capabilities: &Capabilities) -> Vec<f64> {
    match mode {
        Some(mode) if !mode.scales.is_empty() => mode.scales.clone(),
        _ if capabilities.fractional_scale => FRACTIONAL_SCALES.to_vec(),
        _ => WHOLE_SCALES.to_vec(),
    }
}

impl Editor {
    /// The modes output `index` lists; none for an output without hardware
    /// facts.
    pub fn listed_modes(&self, index: usize) -> &[Mode] {
        self.pending
            .entries
            .get(index)
            .and_then(|entry| entry.output.as_ref())
            .map_or(&[], |output| &output.modes)
    }

    /// Whether output `index` is on in the pending layout.
    fn is_on(&self, index: usize) -> bool {
        self.pending
            .entries
            .get(index)
            .is_some_and(|entry| entry.state.enabled)
    }

    /// [`Editor::edit`] for a setting of an output that is on.
    fn edit_on(
        &mut self,
        index: usize,
        change: impl FnOnce(&mut dsplr_model::OutputState),
    ) -> bool {
        self.is_on(index) && self.edit(index, change)
    }

    /// Moves output `index` one [`NUDGE_STEP`] towards `direction`.
    pub fn nudge(&mut self, index: usize, direction: Direction) -> bool {
        let (dx, dy) = match direction {
            Direction::Left => (-NUDGE_STEP, 0),
            Direction::Right => (NUDGE_STEP, 0),
            Direction::Up => (0, -NUDGE_STEP),
            Direction::Down => (0, NUDGE_STEP),
        };
        self.edit_on(index, |state| {
            state.x = state.x.saturating_add(dx);
            state.y = state.y.saturating_add(dy);
        })
    }

    /// Moves output `index` towards `direction` until it touches the nearest
    /// output that way, keeping its other coordinate: the nearest of the
    /// outputs placed on the canvas (on, sized, not mirrors) that lie wholly
    /// that way and share some of its span across. Nothing moves when there
    /// is none, and nothing when that nearest one already touches it: the
    /// key stops at a neighbour, it does not jump past it to the next.
    pub fn move_to_neighbour(&mut self, index: usize, direction: Direction) -> bool {
        let layout_mode = self.pending.extent_mode();
        let Some(me) = self
            .pending
            .entries
            .get(index)
            .and_then(|entry| Rect::of(entry, layout_mode))
        else {
            return false;
        };
        let others: Vec<Rect> = self
            .pending
            .entries
            .iter()
            .enumerate()
            .filter(|&(other, entry)| {
                other != index && on_canvas(&entry.state) && entry.state.mirror_of.is_none()
            })
            .filter_map(|(_, entry)| Rect::of(entry, layout_mode))
            .collect();
        let across = |other: &Rect| match direction {
            Direction::Left | Direction::Right => other.y < me.bottom() && me.y < other.bottom(),
            Direction::Up | Direction::Down => other.x < me.right() && me.x < other.right(),
        };
        let candidates = others.iter().filter(|other| across(other));
        let target = match direction {
            Direction::Left => candidates
                .filter(|o| o.right() <= me.x)
                .map(|o| (o.right(), me.y))
                .max_by_key(|&(x, _)| x),
            Direction::Right => candidates
                .filter(|o| o.x >= me.right())
                .map(|o| (o.x - me.width, me.y))
                .min_by_key(|&(x, _)| x),
            Direction::Up => candidates
                .filter(|o| o.bottom() <= me.y)
                .map(|o| (me.x, o.bottom()))
                .max_by_key(|&(_, y)| y),
            Direction::Down => candidates
                .filter(|o| o.y >= me.bottom())
                .map(|o| (me.x, o.y - me.height))
                .min_by_key(|&(_, y)| y),
        };
        let Some((x, y)) =
            target.and_then(|(x, y)| Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?)))
        else {
            return false;
        };
        self.edit_on(index, |state| (state.x, state.y) = (x, y))
    }

    /// Turns output `index` a quarter turn, clockwise or not.
    pub fn rotate(&mut self, index: usize, clockwise: bool) -> bool {
        self.edit_on(index, |state| {
            state.transform = state.transform.rotated(clockwise)
        })
    }

    /// Sets output `index`'s scale to the next of [`scale_choices`] above its
    /// own (`up`) or below it.
    pub fn step_scale(&mut self, index: usize, up: bool) -> bool {
        let Some(entry) = self.pending.entries.get(index) else {
            return false;
        };
        let current = entry.state.scale;
        let choices = scale_choices(entry.state.mode.as_ref(), &self.snapshot.capabilities);
        let next = if up {
            choices
                .into_iter()
                .filter(|&scale| scale > current + SCALE_EPSILON)
                .reduce(f64::min)
        } else {
            choices
                .into_iter()
                .filter(|&scale| scale < current - SCALE_EPSILON)
                .reduce(f64::max)
        };
        next.is_some_and(|scale| self.set_scale(index, scale))
    }

    /// Turns output `index` on or off. Turned on without a mode, it takes
    /// [`default_mode`]; turned on where it would overlap an output on the
    /// canvas, or touch none of them (the layout moved on while it was off,
    /// which moving to the origin does), it moves to the right of them all,
    /// level with the topmost. Where it still touches one and overlaps none,
    /// it keeps its place.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) -> bool {
        if !enabled {
            return self.edit(index, |state| state.enabled = false);
        }
        let default = default_mode(self.listed_modes(index)).cloned();
        let layout_mode = self.pending.extent_mode();
        let others: Vec<Rect> = self
            .pending
            .entries
            .iter()
            .enumerate()
            .filter(|&(other, entry)| other != index && on_canvas(&entry.state))
            .filter_map(|(_, entry)| Rect::of(entry, layout_mode))
            .collect();
        self.edit(index, |state| {
            state.enabled = true;
            if state.mode.is_none() {
                state.mode = default;
            }
            let Some(entry_rect) = Rect::of_state(state, layout_mode) else {
                return;
            };
            let overlaps = others.iter().any(|other| other.overlaps(entry_rect));
            let detached =
                !others.is_empty() && !others.iter().any(|other| other.touches(entry_rect));
            if overlaps || detached {
                let right = others.iter().map(|other| other.right()).max();
                let top = others.iter().map(|other| other.y).min();
                if let (Some(Ok(x)), Some(Ok(y))) =
                    (right.map(i32::try_from), top.map(i32::try_from))
                {
                    (state.x, state.y) = (x, y);
                }
            }
        })
    }

    /// Turns output `index` off if on, on if off; see [`Editor::set_enabled`].
    pub fn toggle_enabled(&mut self, index: usize) -> bool {
        let on = self.is_on(index);
        self.set_enabled(index, !on)
    }

    /// Sets output `index` to the size `width` x `height`, at the refresh
    /// [`pick_mode`] keeps or chooses among its listed modes.
    pub fn set_size(&mut self, index: usize, width: u32, height: u32) -> bool {
        let Some(entry) = self.pending.entries.get(index) else {
            return false;
        };
        let asked = ModeSpec {
            width,
            height,
            refresh_mhz: None,
        };
        let picked = pick_mode(
            asked,
            self.listed_modes(index),
            entry.state.mode.as_ref(),
            false,
        );
        picked.is_some_and(|mode| self.set_mode(index, mode))
    }

    /// Sets output `index` to `mode`.
    pub fn set_mode(&mut self, index: usize, mode: Mode) -> bool {
        self.edit_on(index, |state| state.mode = Some(mode))
    }

    /// Sets output `index` to the mode `asked` names, a custom mode where the
    /// backend takes them. The error says why it cannot be had.
    pub fn set_custom_mode(&mut self, index: usize, asked: ModeSpec) -> Result<bool, String> {
        let custom_modes = self.snapshot.capabilities.custom_modes;
        let current = self
            .pending
            .entries
            .get(index)
            .and_then(|entry| entry.state.mode.as_ref());
        match pick_mode(asked, self.listed_modes(index), current, custom_modes) {
            Some(mode) => Ok(self.set_mode(index, mode)),
            None if !custom_modes => {
                Err(format!("{} takes no custom modes", self.snapshot.backend))
            }
            None => {
                let size = format!("{}x{}", asked.width, asked.height);
                Err(format!(
                    "{size} is not listed: a custom mode needs its refresh, as {size}@60"
                ))
            }
        }
    }

    /// Sets output `index`'s scale; a scale that is not a positive number
    /// changes nothing.
    pub fn set_scale(&mut self, index: usize, scale: f64) -> bool {
        is_valid_scale(scale) && self.edit_on(index, |state| state.scale = scale)
    }

    /// Sets output `index`'s transform.
    pub fn set_transform(&mut self, index: usize, transform: Transform) -> bool {
        self.edit_on(index, |state| state.transform = transform)
    }

    /// Sets output `index`'s adaptive sync, when the backend takes `sync`.
    pub fn set_adaptive_sync(&mut self, index: usize, sync: AdaptiveSync) -> bool {
        self.snapshot.capabilities.adaptive_sync.contains(&sync)
            && self.edit_on(index, |state| state.adaptive_sync = Some(sync))
    }

    /// Makes output `index` mirror `source`, another output in the layout, or
    /// stop mirroring; only where the backend mirrors.
    pub fn set_mirror(&mut self, index: usize, source: Option<OutputId>) -> bool {
        let valid = match &source {
            None => true,
            Some(source) => self
                .pending
                .entries
                .iter()
                .enumerate()
                .any(|(other, entry)| other != index && entry.id == *source),
        };
        valid
            && self.snapshot.capabilities.mirroring
            && self.edit_on(index, |state| state.mirror_of = source)
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{
        AdaptiveSync, Capabilities, Mode, ModeSpec, OutputId, OutputState, Rect, Transform,
    };

    use super::{Direction, NUDGE_STEP, scale_choices};
    use crate::Editor;
    use crate::fixtures::{output, owners_machine, three_monitors};

    /// DP-1 (0,360, 1920x1080), DP-2 (1920,0, turned and at 1.25: 1152x2048)
    /// and HDMI-A-1 (off), each listing 1920x1080 at 60 and 144 Hz and
    /// 1280x720 at 60 Hz.
    fn editor() -> Editor {
        let mut snapshot = three_monitors();
        for entry in &mut snapshot.layout.entries {
            let connector = entry.id.to_string();
            let mut modes = vec![
                Mode::new(1920, 1080, 60_000),
                Mode::new(1920, 1080, 144_000),
                Mode::new(1280, 720, 60_000),
            ];
            modes[0].preferred = connector == "HDMI-A-1";
            entry.output = Some(output(&connector, "", modes));
        }
        Editor::new(snapshot)
    }

    fn state(editor: &Editor, index: usize) -> &OutputState {
        &editor.pending().entries[index].state
    }

    #[test]
    fn edits_keep_the_layout_at_the_origin_where_the_backend_needs_it() {
        let mut snapshot = three_monitors();
        snapshot.capabilities.origin_required = true;
        let mut editor = Editor::new(snapshot);
        // DP-1 (0,360) and DP-2 (1920,0): turning DP-1 off moves DP-2 to
        // the origin, in the same undo step.
        assert!(editor.set_enabled(0, false));
        assert_eq!((state(&editor, 1).x, state(&editor, 1).y), (0, 0));
        assert!(editor.undo());
        assert_eq!(state(&editor, 1).x, 1920);
        // Nudging the output at x 0 further left keeps it there and moves
        // the others right instead.
        assert!(editor.nudge(0, Direction::Left));
        assert_eq!(state(&editor, 0).x, 0);
        assert_eq!(state(&editor, 1).x, 1920 + NUDGE_STEP);
    }

    #[test]
    fn moving_to_the_origin_leaves_the_output_that_is_off_where_it_was() {
        let mut snapshot = three_monitors();
        snapshot.capabilities.origin_required = true;
        let mut editor = Editor::new(snapshot);
        let off = (state(&editor, 2).x, state(&editor, 2).y);
        // DP-1 left from x 0 moves DP-2 right instead; DP-2 left again puts
        // everything on screen back as it was.
        assert!(editor.nudge(0, Direction::Left));
        assert!(editor.nudge(1, Direction::Left));
        assert_eq!((state(&editor, 2).x, state(&editor, 2).y), off);
        assert!(!editor.has_pending(), "no change that shows nowhere");
    }

    #[test]
    fn a_nudge_moves_one_step_and_is_one_undo_step() {
        let mut editor = editor();
        assert!(editor.nudge(0, Direction::Right));
        assert!(editor.nudge(0, Direction::Down));
        assert_eq!(
            (state(&editor, 0).x, state(&editor, 0).y),
            (NUDGE_STEP, 360 + NUDGE_STEP)
        );
        assert!(editor.nudge(0, Direction::Left));
        assert!(editor.nudge(0, Direction::Up));
        assert!(!editor.has_pending(), "back where it was");
        for _ in 0..4 {
            assert!(editor.undo());
        }
        assert!(!editor.undo());
    }

    #[test]
    fn an_output_that_is_off_keeps_its_settings() {
        let mut editor = editor();
        assert!(!editor.nudge(2, Direction::Left));
        assert!(!editor.rotate(2, true));
        assert!(!editor.step_scale(2, true));
        assert!(!editor.set_transform(2, Transform::Rotate180));
        assert!(!editor.has_pending());
    }

    #[test]
    fn moving_to_a_neighbour_stops_touching_it() {
        let mut editor = editor();
        // DP-1 leaves DP-2 by 500 pixels, then comes back to touch it.
        editor.edit(0, |state| state.x = -500);
        assert!(editor.move_to_neighbour(0, Direction::Right));
        assert_eq!(state(&editor, 0).x, 0, "1920 wide, touching DP-2 at 1920");
        assert!(
            !editor.move_to_neighbour(0, Direction::Right),
            "already touching"
        );
        // DP-2 goes left to touch DP-1, which shares its span.
        editor.edit(1, |state| state.x = 2500);
        assert!(editor.move_to_neighbour(1, Direction::Left));
        assert_eq!(state(&editor, 1).x, 1920);
        // Nothing above DP-1, which DP-2's span shares.
        assert!(!editor.move_to_neighbour(0, Direction::Up));
        // Side by side, neither is below the other.
        assert!(!editor.move_to_neighbour(1, Direction::Down));
        editor.edit(0, |state| (state.x, state.y) = (1920, 3000));
        assert!(editor.move_to_neighbour(0, Direction::Up));
        assert_eq!(state(&editor, 0).y, 2048, "under DP-2");
        editor.edit(0, |state| state.y = -2000);
        assert!(editor.move_to_neighbour(0, Direction::Down));
        assert_eq!(state(&editor, 0).y, -1080, "above DP-2");
    }

    #[test]
    fn moving_to_a_neighbour_stops_at_one_it_touches_and_never_jumps_past() {
        let mut editor = editor();
        // HDMI-A-1 on, far right of DP-2, level with DP-1.
        editor.edit(2, |state| {
            (state.enabled, state.x, state.y) = (true, 6000, 360)
        });
        assert!(
            !editor.move_to_neighbour(0, Direction::Right),
            "DP-2 is touching"
        );
        assert_eq!(state(&editor, 0).x, 0);
        assert!(
            editor.move_to_neighbour(1, Direction::Right),
            "DP-2 to HDMI-A-1"
        );
        assert_eq!(state(&editor, 1).x, 6000 - 1152);
    }

    #[test]
    fn rotating_turns_a_quarter_each_way() {
        let mut editor = editor();
        assert!(editor.rotate(0, true));
        assert_eq!(state(&editor, 0).transform, Transform::Rotate270);
        assert!(editor.rotate(0, false));
        assert!(editor.rotate(0, false));
        assert_eq!(state(&editor, 0).transform, Transform::Rotate90);
    }

    #[test]
    fn scale_steps_through_the_offered_scales() {
        let mut editor = editor();
        assert!(editor.step_scale(1, true));
        assert_eq!(state(&editor, 1).scale, 1.5);
        assert!(editor.step_scale(1, false));
        assert!(editor.step_scale(1, false));
        assert_eq!(state(&editor, 1).scale, 1.0);
        editor.edit(1, |state| state.scale = 3.0);
        assert!(!editor.step_scale(1, true), "the largest offered");
        // A scale between steps goes to the neighbouring step.
        editor.edit(1, |state| state.scale = 1.3);
        assert!(editor.step_scale(1, true));
        assert_eq!(state(&editor, 1).scale, 1.5);
    }

    #[test]
    fn whole_scales_only_without_fractional_scaling() {
        let whole = Capabilities::default();
        assert_eq!(scale_choices(None, &whole), [1.0, 2.0, 3.0]);
        let mut mode = Mode::new(1920, 1080, 60_000);
        mode.scales = vec![1.0, 1.5];
        assert_eq!(
            scale_choices(Some(&mode), &whole),
            [1.0, 1.5],
            "the mode's own"
        );
    }

    #[test]
    fn turning_on_gives_a_mode_and_a_place_clear_of_the_others() {
        let mut editor = editor();
        editor.edit(2, |state| state.mode = None);
        assert!(editor.toggle_enabled(2));
        let on = state(&editor, 2);
        assert!(on.enabled);
        let mode = on.mode.as_ref().map(|m| (m.width, m.height, m.refresh_mhz));
        assert_eq!(mode, Some((1920, 1080, 60_000)), "the preferred mode");
        assert_eq!((on.x, on.y), (3072, 0), "right of DP-2, level with its top");
        assert!(editor.toggle_enabled(2));
        assert!(!state(&editor, 2).enabled);
        // Beside DP-1 and clear of the others: it stays where it was.
        editor.edit(2, |state| state.x = -1920);
        assert!(editor.set_enabled(2, true));
        assert_eq!(state(&editor, 2).x, -1920);
    }

    #[test]
    fn turned_on_away_from_the_others_it_joins_them() {
        let mut editor = editor();
        // Beside DP-1 when it went off; then the outputs on screen moved on
        // without it, as moving a layout to the origin does now.
        editor.edit(2, |state| (state.x, state.y) = (-1920, 360));
        for index in [0, 1] {
            editor.edit(index, |state| state.x += 500);
        }
        assert!(editor.set_enabled(2, true));
        let layout_mode = editor.pending().extent_mode();
        let rect = |index| Rect::of(&editor.pending().entries[index], layout_mode).unwrap();
        assert!(
            [0, 1].iter().any(|&other| rect(2).touches(rect(other))),
            "it touches the layout: {:?}",
            (state(&editor, 2).x, state(&editor, 2).y)
        );
        assert!(!editor.pending().entries.iter().any(|e| e.state.x < 0));
    }

    #[test]
    fn a_new_size_keeps_the_refresh_where_it_can() {
        let mut editor = editor();
        editor.edit(0, |state| state.mode = Some(Mode::new(1920, 1080, 144_000)));
        assert!(editor.set_size(0, 1280, 720));
        assert_eq!(
            state(&editor, 0).mode,
            Some(Mode::new(1280, 720, 60_000)),
            "only 60 there"
        );
        assert!(editor.set_size(0, 1920, 1080));
        assert_eq!(
            state(&editor, 0).mode,
            Some(Mode::new(1920, 1080, 60_000)),
            "keeps 60"
        );
        assert!(!editor.set_size(0, 800, 600), "not listed");
    }

    #[test]
    fn a_custom_mode_needs_a_refresh_and_a_backend_that_takes_it() {
        let mut editor = editor();
        let spec = |refresh_mhz| ModeSpec {
            width: 1000,
            height: 700,
            refresh_mhz,
        };
        assert_eq!(editor.set_custom_mode(0, spec(Some(50_000))), Ok(true));
        assert_eq!(state(&editor, 0).mode, Some(Mode::new(1000, 700, 50_000)));
        let refusal = editor.set_custom_mode(0, spec(None)).unwrap_err();
        assert!(refusal.contains("needs its refresh"), "{refusal}");

        let mut hyprland = Editor::new(owners_machine());
        let refusal = hyprland.set_custom_mode(1, spec(Some(50_000))).unwrap_err();
        assert_eq!(refusal, "hyprland takes no custom modes");
        // A listed mode is no custom mode, and is taken anywhere.
        let listed = ModeSpec {
            width: 2560,
            height: 1440,
            refresh_mhz: Some(119_999),
        };
        assert_eq!(
            hyprland.set_custom_mode(1, listed),
            Ok(false),
            "it is set already"
        );
    }

    #[test]
    fn settings_the_backend_lacks_are_refused() {
        let mut editor = editor();
        assert!(!editor.set_scale(0, 0.0));
        assert!(!editor.set_scale(0, f64::NAN));
        assert!(editor.set_adaptive_sync(0, AdaptiveSync::On));
        assert!(
            !editor.set_adaptive_sync(0, AdaptiveSync::Automatic),
            "wlr: off or on"
        );
        assert!(
            !editor.set_mirror(0, Some(OutputId::Connector("DP-2".into()))),
            "wlr cannot mirror"
        );

        let mut hyprland = Editor::new(owners_machine());
        let laptop = hyprland.pending().entries[0].id.clone();
        let monitor = hyprland.pending().entries[1].id.clone();
        assert!(!hyprland.set_mirror(1, Some(monitor)), "not itself");
        assert!(!hyprland.set_mirror(1, Some(OutputId::Connector("DP-9".into()))));
        assert!(hyprland.set_mirror(1, Some(laptop)));
        assert!(hyprland.set_mirror(1, None));
    }
}
