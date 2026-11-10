//! CDATA sections turned into the character data they stand for, so
//! `crcbl-wl-scanner`'s reader, which refuses CDATA, can read KDE's protocol
//! files. Included by `build.rs`, and by the library's tests, which is where
//! it is tested.

use core::fmt;

/// What [`without_cdata`] found left open. Each names the byte
/// offset where the construct starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unclosed {
    /// A `<![CDATA[` with no `]]>` after it.
    Cdata(usize),
    /// A `<!--` with no `-->` after it.
    Comment(usize),
    /// A `<?` with no `?>` after it.
    Instruction(usize),
    /// A tag with no `>` after it, outside its quoted attribute values.
    Tag(usize),
}

impl fmt::Display for Unclosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (what, at) = match *self {
            Self::Cdata(at) => ("CDATA section", at),
            Self::Comment(at) => ("comment", at),
            Self::Instruction(at) => ("processing instruction", at),
            Self::Tag(at) => ("tag", at),
        };
        write!(f, "the {what} at byte {at} is not closed")
    }
}

/// `xml` with each CDATA section replaced by the same characters escaped as
/// ordinary character data, which is what a CDATA section means (XML 1.0,
/// section 2.7: its text runs from `<![CDATA[` to the first `]]>` and is not
/// markup). A `<![CDATA[` inside a comment, a processing instruction or a
/// tag's quoted attribute value is not markup there, and is kept as it is:
/// those are copied whole. Anything left open is an error rather than a
/// guess.
pub fn without_cdata(xml: &str) -> Result<String, Unclosed> {
    const CDATA: &str = "<![CDATA[";
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    while let Some(next) = xml[at..].find('<') {
        let start = at + next;
        out.push_str(&xml[at..start]);
        let rest = &xml[start..];
        at = if let Some(inside) = rest.strip_prefix(CDATA) {
            let end = inside.find("]]>").ok_or(Unclosed::Cdata(start))?;
            for c in inside[..end].chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    c => out.push(c),
                }
            }
            start + CDATA.len() + end + "]]>".len()
        } else {
            let end = if rest.starts_with("<!--") {
                rest.find("-->")
                    .map(|end| end + "-->".len())
                    .ok_or(Unclosed::Comment(start))?
            } else if rest.starts_with("<?") {
                rest.find("?>")
                    .map(|end| end + "?>".len())
                    .ok_or(Unclosed::Instruction(start))?
            } else {
                tag_end(rest).ok_or(Unclosed::Tag(start))?
            };
            out.push_str(&rest[..end]);
            start + end
        };
    }
    out.push_str(&xml[at..]);
    Ok(out)
}

/// The length of the tag `tag` starts with, through its `>`: the first `>`
/// outside a quoted attribute value. `None` when there is none.
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in tag.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, '>') => return Some(index + 1),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{Unclosed, without_cdata};

    #[test]
    fn a_cdata_section_becomes_its_escaped_text() {
        assert_eq!(
            without_cdata("<copyright><![CDATA[A <a@b.org> & co]]></copyright>"),
            Ok("<copyright>A &lt;a@b.org&gt; &amp; co</copyright>".to_owned())
        );
        // Its text ends at the first `]]>`, and a document without one is
        // left as it is.
        assert_eq!(
            without_cdata("<a><![CDATA[x]]>]]></a>"),
            Ok("<a>x]]></a>".to_owned())
        );
        let plain = "<?xml version=\"1.0\"?>\n<p a=\"1 &gt; 0\">t</p>\n";
        assert_eq!(without_cdata(plain).as_deref(), Ok(plain));
    }

    #[test]
    fn a_marker_in_a_comment_instruction_or_attribute_is_kept() {
        for kept in [
            // Each holds a `>` before the marker, which a tag would end at.
            "<!-- a > <![CDATA[ not markup --><a/>",
            "<?pi a > <![CDATA[ ?><a/>",
            "<a s=\"<![CDATA[\" t='>'/>",
        ] {
            assert_eq!(without_cdata(kept).as_deref(), Ok(kept), "{kept}");
        }
    }

    #[test]
    fn what_is_left_open_is_an_error_naming_where_it_starts() {
        assert_eq!(without_cdata("<a><![CDATA[x</a>"), Err(Unclosed::Cdata(3)));
        assert_eq!(without_cdata("<a/><!-- x"), Err(Unclosed::Comment(4)));
        assert_eq!(without_cdata("<?x"), Err(Unclosed::Instruction(0)));
        assert_eq!(without_cdata("<a s=\">\""), Err(Unclosed::Tag(0)));
        assert_eq!(
            Unclosed::Cdata(3).to_string(),
            "the CDATA section at byte 3 is not closed"
        );
    }

    #[test]
    fn the_vendored_kde_files_lose_every_cdata_section_and_nothing_else() {
        for file in [
            "kde-output-device-v2.xml",
            "kde-output-management-v2.xml",
            "kde-output-order-v1.xml",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("protocols")
                .join(file);
            let xml = std::fs::read_to_string(&path).expect("vendored");
            assert!(xml.contains("<![CDATA["), "{file} has a CDATA section");
            let out = without_cdata(&xml).expect("reads");
            assert!(!out.contains("<![CDATA["), "{file}");
            assert!(out.contains("SPDX-License-Identifier: MIT-CMU"), "{file}");
            // Only the copyright block changes.
            let after = |text: &str| text[text.find("</copyright>").expect("one")..].to_owned();
            assert_eq!(after(&out), after(&xml), "{file}");
        }
    }
}
