//! The face the GUI writes in (C7 in `docs/plan/06-gui.md`): the system's
//! sans-serif through fontconfig, carrying the system's fallback faces for
//! what it has no glyph for, as pikr does; crcbl's committed sans-serif
//! (`Font::sans`) where the system has none that reads.

use crcbl_ui::font::Font;
use crcbl_ui::font::system::crcbl_font::{FontQuery, SystemFonts};

/// The family the panel's stylesheet names, under which [`Ui::register_font`]
/// registers the face.
///
/// [`Ui::register_font`]: crcbl_ui::tree::Ui::register_font
pub const FAMILY: &str = "dsplr-sans";

/// The face and its fallbacks, as the GUI uses them.
pub struct Faces {
    /// The face, carrying the fallbacks.
    pub face: Font,
    /// The fallbacks alone, which a tree takes on its own.
    pub fallbacks: Vec<Font>,
    /// Why the committed face is used, when it is.
    pub note: Option<String>,
}

/// The system's sans-serif and its fallbacks, each fallback read only once a
/// glyph needs it; else the committed face, with why.
pub fn system_sans() -> Faces {
    let system = SystemFonts::new();
    let query = FontQuery::new("sans-serif");
    let found = system.find(&query).map(|face| Font::from_system(&face));
    match found {
        Some(Ok(face)) => {
            let fallbacks: Vec<Font> = system
                .fallbacks(&query)
                .into_iter()
                .map(Font::from_system_lazy)
                .collect();
            Faces {
                face: face.with_fallback(&fallbacks),
                fallbacks,
                note: None,
            }
        }
        Some(Err(error)) => committed(format!("the system's sans-serif cannot be read ({error})")),
        None => committed("the system has no sans-serif font".to_owned()),
    }
}

/// The committed face, for `why`.
fn committed(why: String) -> Faces {
    Faces {
        face: Font::sans().clone(),
        fallbacks: Vec::new(),
        note: Some(format!("{why}; writing in dsplr's own")),
    }
}

#[cfg(test)]
mod tests {
    use crcbl_ui::font::Font;
    use crcbl_ui::font::system::crcbl_font::{FontQuery, SystemFonts};

    use super::system_sans;

    /// The system's face where fontconfig finds a sans-serif (this machine,
    /// and CI's runners where one is installed), the committed one, said,
    /// where it finds none.
    #[test]
    fn the_system_face_is_used_where_there_is_one() {
        let faces = system_sans();
        let committed = Font::sans().id();
        if SystemFonts::new()
            .find(&FontQuery::new("sans-serif"))
            .is_some()
        {
            assert_eq!(faces.note, None);
            assert_ne!(faces.face.id(), committed, "the system's, not dsplr's own");
        } else {
            assert!(faces.note.is_some_and(|note| note.contains("dsplr's own")));
            assert_eq!(faces.face.id(), committed);
        }
    }
}
