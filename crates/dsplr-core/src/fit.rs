//! Where the canvas is in the window, and how layout coordinates map onto it.

use dsplr_model::{Layout, Rect};

use crate::draw::{Bounds, Point};
use crate::editor::on_canvas;
use crate::{Editor, Size};

/// The property panel's width.
pub const PANEL_WIDTH: f32 = 320.0;
/// The bottom bar's height.
pub const BAR_HEIGHT: f32 = 32.0;
/// Space kept clear around the outputs, and around the tray.
pub const MARGIN: f32 = 24.0;
/// A tray tile's height.
pub const TILE_HEIGHT: f32 = 52.0;

/// The window's regions.
#[derive(Debug, Clone, Copy)]
pub struct Regions {
    /// Left of the panel and above the bar.
    pub canvas: Bounds,
    /// Inside the canvas: where outputs are fitted, above the tray.
    pub area: Bounds,
    pub panel: Bounds,
    pub bar: Bounds,
}

/// The regions of a window of `viewport` for `editor`: the tray takes a band
/// at the bottom of the canvas only when there is an output to put in it.
pub fn regions(editor: &Editor, viewport: Size) -> Regions {
    let canvas = Bounds::new(
        0.0,
        0.0,
        (viewport.width - PANEL_WIDTH).max(0.0),
        (viewport.height - BAR_HEIGHT).max(0.0),
    );
    let tray = editor
        .live()
        .entries
        .iter()
        .any(|entry| !on_canvas(&entry.state));
    let tray_band = if tray { TILE_HEIGHT + MARGIN } else { 0.0 };
    Regions {
        canvas,
        area: Bounds::new(
            canvas.x + MARGIN,
            canvas.y + MARGIN,
            (canvas.width - 2.0 * MARGIN).max(0.0),
            (canvas.height - 2.0 * MARGIN - tray_band).max(0.0),
        ),
        panel: Bounds::new(
            canvas.right(),
            0.0,
            viewport.width - canvas.width,
            canvas.height,
        ),
        bar: Bounds::new(
            0.0,
            canvas.height,
            viewport.width,
            viewport.height - canvas.height,
        ),
    }
}

/// The index and rectangle of each of `layout`'s outputs on the canvas.
pub fn rects(layout: &Layout) -> impl Iterator<Item = (usize, Rect)> + '_ {
    layout
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| on_canvas(&entry.state))
        .filter_map(|(index, entry)| Some((index, Rect::of(entry)?)))
}

/// The fit of `editor`'s live and pending outputs, together, in `area`.
pub fn fit(editor: &Editor, area: Bounds) -> Fit {
    Fit::new(
        rects(editor.live())
            .chain(rects(editor.pending()))
            .map(|(_, rect)| rect),
        area,
    )
}

/// The scale and offset that fit a set of layout rectangles, as a whole,
/// inside an area of the window, centred in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Window pixels per layout pixel.
    scale: f64,
    /// The layout point drawn at `origin`.
    min: (i64, i64),
    origin: (f64, f64),
}

impl Fit {
    fn new(rects: impl Iterator<Item = Rect>, area: Bounds) -> Self {
        let mut extent: Option<(i64, i64, i64, i64)> = None;
        for rect in rects {
            let (left, top, right, bottom) =
                extent.unwrap_or_else(|| (rect.x, rect.y, rect.right(), rect.bottom()));
            extent = Some((
                left.min(rect.x),
                top.min(rect.y),
                right.max(rect.right()),
                bottom.max(rect.bottom()),
            ));
        }
        let (left, top, right, bottom) = extent.unwrap_or_default();
        // An empty extent (nothing on the canvas, or only zero-sized
        // outputs) is taken as one pixel, so the scale stays finite.
        let width = (right - left).max(1) as f64;
        let height = (bottom - top).max(1) as f64;
        let scale = (f64::from(area.width) / width).min(f64::from(area.height) / height);
        Self {
            scale,
            min: (left, top),
            origin: (
                f64::from(area.x) + (f64::from(area.width) - width * scale) / 2.0,
                f64::from(area.y) + (f64::from(area.height) - height * scale) / 2.0,
            ),
        }
    }

    /// Window pixels per layout pixel.
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// Where `rect` is drawn in the window.
    pub fn place(&self, rect: Rect) -> Bounds {
        let x = self.origin.0 + (rect.x - self.min.0) as f64 * self.scale;
        let y = self.origin.1 + (rect.y - self.min.1) as f64 * self.scale;
        Bounds::new(
            x as f32,
            y as f32,
            (rect.width as f64 * self.scale) as f32,
            (rect.height as f64 * self.scale) as f32,
        )
    }

    /// The layout point drawn at `point`: [`Fit::place`] undone.
    pub fn layout_point(&self, point: Point) -> (f64, f64) {
        (
            (f64::from(point.x) - self.origin.0) / self.scale + self.min.0 as f64,
            (f64::from(point.y) - self.origin.1) / self.scale + self.min.1 as f64,
        )
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::Rect;

    use super::Fit;
    use crate::{Bounds, Point};

    #[test]
    fn a_point_maps_back_to_the_layout_point_drawn_there() {
        let rects = [
            Rect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
            Rect {
                x: 0,
                y: 200,
                width: 2560,
                height: 1440,
            },
        ];
        let fit = Fit::new(rects.into_iter(), Bounds::new(24.0, 24.0, 900.0, 600.0));
        let drawn = fit.place(rects[1]);
        let (x, y) = fit.layout_point(Point::new(drawn.x, drawn.y));
        assert!(
            (x - 0.0).abs() < 0.01 && (y - 200.0).abs() < 0.01,
            "{x},{y}"
        );
        let (x, y) = fit.layout_point(Point::new(drawn.right(), drawn.bottom()));
        assert!(
            (x - 2560.0).abs() < 0.01 && (y - 1640.0).abs() < 0.01,
            "{x},{y}"
        );
    }
}
