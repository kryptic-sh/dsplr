//! dsplr-core's draw list, translated into crcbl's.

use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::font::layout::TextLayout;
use crcbl_ui::style::from_srgb8;
use dsplr_core::{Bounds, Color, Item};
use glam::Vec2;

/// A theme colour as the linear `[r, g, b, a]` crcbl draws with.
pub fn linear(color: Color) -> [f32; 4] {
    from_srgb8(color.r, color.g, color.b, f32::from(color.a) / 255.0)
}

/// Appends `scene`'s items to `out`, text laid out in `font`.
pub fn paint(scene: &dsplr_core::DrawList, out: &mut DrawList, font: &Font) {
    for item in &scene.items {
        match item {
            Item::Fill { bounds, color } => {
                let (min, max) = corners(*bounds);
                out.rect(min, max, linear(*color));
            }
            Item::Outline {
                bounds,
                thickness,
                color,
            } => {
                let (min, max) = corners(*bounds);
                out.rect_outline(min, max, *thickness, linear(*color));
            }
            Item::Text {
                at,
                text,
                size,
                color,
                clip,
            } => {
                let line_height = font.metrics().normal_line_height(*size);
                let layout = TextLayout::nowrap(font, text, *size, line_height);
                let (min, max) = corners(*clip);
                out.push_clip(min, max);
                out.text_glyphs(
                    Vec2::new(at.x, at.y),
                    font,
                    *size,
                    linear(*color),
                    layout.glyphs(),
                    text,
                );
                out.pop_clip().expect("the clip pushed just above");
            }
        }
    }
}

fn corners(bounds: Bounds) -> (Vec2, Vec2) {
    (
        Vec2::new(bounds.x, bounds.y),
        Vec2::new(bounds.right(), bounds.bottom()),
    )
}

#[cfg(test)]
mod tests {
    use crcbl_ui::draw_list::{DrawCommand, DrawList};
    use crcbl_ui::font::Font;
    use dsplr_core::{Bounds, Color, Item, Point};

    use super::paint;

    #[test]
    fn every_item_becomes_a_crcbl_command_with_text_clipped() {
        let scene = dsplr_core::DrawList {
            items: vec![
                Item::Fill {
                    bounds: Bounds::new(0.0, 0.0, 10.0, 20.0),
                    color: Color::hex(0xffffff),
                },
                Item::Outline {
                    bounds: Bounds::new(1.0, 2.0, 3.0, 4.0),
                    thickness: 2.0,
                    color: Color::hex(0x000000),
                },
                Item::Text {
                    at: Point::new(5.0, 6.0),
                    text: "DP-1".into(),
                    size: 13.0,
                    color: Color::hex(0xc0caf5),
                    clip: Bounds::new(0.0, 0.0, 50.0, 30.0),
                },
            ],
            hits: Vec::new(),
        };
        let mut out = DrawList::new();
        paint(&scene, &mut out, Font::sans());
        let commands = out.commands();
        assert_eq!(commands.len(), 3);
        assert!(matches!(
            commands[0],
            DrawCommand::Rect { min, max, color }
                if min.x == 0.0 && max.y == 20.0 && color == [1.0; 4]
        ));
        assert!(matches!(
            commands[1],
            DrawCommand::RectOutline { thickness, color, .. }
                if thickness == 2.0 && color == [0.0, 0.0, 0.0, 1.0]
        ));
        match &commands[2] {
            DrawCommand::Glyphs {
                origin,
                glyphs,
                text,
                ..
            } => {
                assert_eq!((origin.x, origin.y), (5.0, 6.0));
                assert_eq!(text.as_deref(), Some("DP-1"));
                assert_eq!(glyphs.len(), 4, "one glyph per letter");
            }
            other => panic!("not a run of glyphs: {other:?}"),
        }
        let clip = out.clips()[2];
        assert_eq!((clip.max.x, clip.max.y), (50.0, 30.0));
        assert_eq!(out.clip(), crcbl_ui::draw_list::ClipRect::NONE, "popped");
    }
}
