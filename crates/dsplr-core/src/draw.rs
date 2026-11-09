//! The draw list: what the GUI paints, as plain data with no platform types,
//! and the hit regions a click is tested against. `dsplr-gui` translates it
//! into crcbl's own draw list.
//!
//! Coordinates are logical pixels of the window, y growing downwards.

use std::fmt;

/// A point in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    /// Whether `point` is inside, its top and left edges included.
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// Shrunk by `by` on every side, never below an empty size.
    pub fn inset(self, by: f32) -> Self {
        Self::new(
            self.x + by,
            self.y + by,
            (self.width - 2.0 * by).max(0.0),
            (self.height - 2.0 * by).max(0.0),
        )
    }
}

/// An sRGB colour with straight alpha, as a theme writes it (`#rrggbb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// An opaque colour from `0xrrggbb`.
    pub const fn hex(rgb: u32) -> Self {
        let [_, r, g, b] = rgb.to_be_bytes();
        Self { r, g, b, a: 255 }
    }
}

/// `#rrggbb`, or `#rrggbbaa` when not opaque.
impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 255 {
            write!(f, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

/// One thing to paint, in order: later items cover earlier ones.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A filled rectangle.
    Fill { bounds: Bounds, color: Color },
    /// A rectangle's border, drawn inside `bounds`.
    Outline {
        bounds: Bounds,
        thickness: f32,
        color: Color,
    },
    /// One line of text whose top-left is `at`, `size` pixels per em, cut
    /// off at `clip`.
    Text {
        at: Point,
        text: String,
        size: f32,
        color: Color,
        clip: Bounds,
    },
}

/// A region a click on selects output `output` (its index in the layout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub bounds: Bounds,
    pub output: usize,
}

/// A frame's items and hit regions.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawList {
    pub items: Vec<Item>,
    /// Tested last to first, so a region added later wins where two overlap.
    /// A region is added only for something drawn filled, which covers what
    /// was drawn before it; an outline, which covers only its border, adds
    /// none.
    pub hits: Vec<Hit>,
}

impl DrawList {
    /// The output a click at `point` selects, if any.
    pub fn hit(&self, point: Point) -> Option<usize> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.bounds.contains(point))
            .map(|hit| hit.output)
    }

    pub(crate) fn fill(&mut self, bounds: Bounds, color: Color) {
        self.items.push(Item::Fill { bounds, color });
    }

    pub(crate) fn outline(&mut self, bounds: Bounds, thickness: f32, color: Color) {
        self.items.push(Item::Outline {
            bounds,
            thickness,
            color,
        });
    }

    pub(crate) fn text(
        &mut self,
        at: Point,
        text: impl Into<String>,
        size: f32,
        color: Color,
        clip: Bounds,
    ) {
        self.items.push(Item::Text {
            at,
            text: text.into(),
            size,
            color,
            clip,
        });
    }
}

/// `x,y wxh`, to a tenth of a pixel.
impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:.1},{:.1} {:.1}x{:.1}",
            self.x, self.y, self.width, self.height
        )
    }
}

/// The deterministic text form the snapshot tests compare: one line per
/// item, then one per hit region.
impl fmt::Display for DrawList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for item in &self.items {
            match item {
                Item::Fill { bounds, color } => writeln!(f, "fill {bounds} {color}")?,
                Item::Outline {
                    bounds,
                    thickness,
                    color,
                } => writeln!(f, "outline {bounds} {thickness:.1} {color}")?,
                Item::Text {
                    at,
                    text,
                    size,
                    color,
                    clip,
                } => writeln!(
                    f,
                    "text {:.1},{:.1} {size:.0} {color} clip {clip} {text:?}",
                    at.x, at.y
                )?,
            }
        }
        for hit in &self.hits {
            writeln!(f, "hit {} output {}", hit.bounds, hit.output)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Bounds, Color, DrawList, Hit, Point};

    #[test]
    fn a_colour_reads_as_hex() {
        assert_eq!(Color::hex(0x1a1b26).to_string(), "#1a1b26");
        let translucent = Color {
            a: 0x80,
            ..Color::hex(0x7aa2f7)
        };
        assert_eq!(translucent.to_string(), "#7aa2f780");
    }

    #[test]
    fn bounds_hold_their_top_left_edges_only() {
        let bounds = Bounds::new(10.0, 20.0, 30.0, 40.0);
        assert!(bounds.contains(Point::new(10.0, 20.0)));
        assert!(bounds.contains(Point::new(39.9, 59.9)));
        assert!(!bounds.contains(Point::new(40.0, 30.0)));
        assert!(!bounds.contains(Point::new(20.0, 60.0)));
        assert!(!bounds.contains(Point::new(9.9, 30.0)));
    }

    #[test]
    fn the_later_of_two_overlapping_hit_regions_wins() {
        let list = DrawList {
            items: Vec::new(),
            hits: vec![
                Hit {
                    bounds: Bounds::new(0.0, 0.0, 100.0, 100.0),
                    output: 0,
                },
                Hit {
                    bounds: Bounds::new(50.0, 50.0, 100.0, 100.0),
                    output: 1,
                },
            ],
        };
        assert_eq!(list.hit(Point::new(10.0, 10.0)), Some(0));
        assert_eq!(list.hit(Point::new(60.0, 60.0)), Some(1));
        assert_eq!(list.hit(Point::new(200.0, 10.0)), None);
    }
}
