//! niri's `outputs.kdl`: one `output "<name>" { … }` node per output, in the
//! KDL niri 26.04 reads (KDL 1: names are quoted strings), written as niri's
//! wiki and `niri validate` take the settings and read back in the same
//! form. The main `config.kdl` loads it with `include "outputs.kdl"`, before
//! its own `output` nodes.
//!
//! What niri 26.04 does with output nodes, nested (winit) in a headless sway
//! 1.12 (2026-11-10):
//! - The **first** node naming an output wins, whole: a second node for it,
//!   in the same file or another, is not merged in. So the include goes
//!   before the main config's own output nodes.
//! - A setting the node leaves out takes niri's default, not the value it
//!   had at runtime: an output whose node is gone goes back to scale 1 at
//!   the origin. So every setting is stated.
//! - niri reloads its config, included files too, as soon as one changes,
//!   so writing the file applies it. A runtime change (one made through
//!   wlr-output-management, as `dsplr apply` makes them) survives a reload
//!   that leaves its output's node as it was, and is replaced by one that
//!   changes it.
//! - A node names an output by its connector or by its identifier
//!   ([`crate::identity::identifier`]), each compared whole and ignoring
//!   ASCII case; a part of the identifier names nothing.

mod read;
mod save;
mod write;

use dsplr_model::{Layout, OutputEntry, OutputId};

use crate::identity;

pub use read::{OutputsNode, ReadError, read_outputs};
pub use save::{INCLUDE_LINE, SaveError, save};
pub use write::{WriteError, output_node, output_nodes};

/// The nodes an `output` node holds that dsplr writes. A node with another
/// one is not dsplr's to replace.
const SETTINGS: [&str; 6] = [
    "off",
    "mode",
    "scale",
    "transform",
    "position",
    "variable-refresh-rate",
];

/// Whether `setting`, the tokens of one node inside an `output` node, is
/// `variable-refresh-rate on-demand=true`: adaptive sync niri turns on only
/// for windows that ask for it.
fn on_demand(setting: &[Token]) -> bool {
    matches!(setting, [Token::Word(name), arguments @ ..]
        if name == "variable-refresh-rate"
            && arguments.contains(&Token::Word("on-demand=true".to_owned())))
}

/// niri's identifier for a monitor: [`identity::identifier`], except that a
/// monitor reporting no make, model or serial has none (niri then matches
/// its connector only).
fn identifier(entry: &OutputEntry) -> Option<String> {
    identity::identifier(entry).filter(|id| id != "Unknown Unknown Unknown")
}

/// The name `entry`'s node is written under: niri's identifier when the
/// monitor has a serial and no other entry of `layout` has that identifier
/// (ignoring case, as niri compares them), so the settings follow the
/// monitor to another port; else its connector, so two monitors reporting
/// the same identifier each keep their own. A name read from a file (a
/// [`OutputId::Description`]) is written as it was read. `None` when there
/// is no usable name.
fn output_name(layout: &Layout, entry: &OutputEntry) -> Option<String> {
    if let OutputId::Description(name) = &entry.id {
        return Some(name.trim().to_owned()).filter(|name| !name.is_empty());
    }
    let own = identifier(entry);
    let usable = own.as_ref().filter(|own| {
        matches!(entry.id, OutputId::Edid { .. })
            && layout
                .entries
                .iter()
                .filter(|other| identifier(other).is_some_and(|id| id.eq_ignore_ascii_case(own)))
                .count()
                == 1
    });
    usable
        .cloned()
        .or_else(|| entry.connector().map(str::to_owned))
        .or(own)
}

/// Whether niri applies a node for `name` to `live`: its connector or its
/// identifier, compared whole and ignoring ASCII case.
fn names(name: &str, live: &OutputEntry) -> bool {
    live.connector()
        .is_some_and(|connector| connector.eq_ignore_ascii_case(name))
        || identifier(live).is_some_and(|id| id.eq_ignore_ascii_case(name))
}

/// `text` as a KDL string: in double quotes, with `"` and `\` escaped.
/// `None` for text with a control character, which dsplr does not write.
fn quoted(text: &str) -> Option<String> {
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        text.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// One token of a line of KDL.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// A bare word: a node name, a number, or a property (`x=0`, its value
    /// unquoted when it was a string).
    Word(String),
    /// A quoted string, its escapes resolved.
    Text(String),
    Open,
    Close,
    Semicolon,
    /// `/-`, which comments out the node after it.
    SlashDash,
}

/// `line` split into KDL tokens, up to a `//` comment. `None` for what
/// dsplr does not read: a `/*` comment, a raw string, a line continuation,
/// an escape other than `\"`, `\\`, `\/`, `\n`, `\t`, `\r`, `\b` and `\f`, or
/// a string left open (KDL lets one span lines; dsplr does not).
fn tokens(line: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '{' | '}' | ';' => {
                chars.next();
                tokens.push(match c {
                    '{' => Token::Open,
                    '}' => Token::Close,
                    _ => Token::Semicolon,
                });
            }
            '"' => {
                chars.next();
                tokens.push(Token::Text(string(&mut chars)?));
            }
            '/' => {
                chars.next();
                match chars.next() {
                    Some('/') => break,
                    Some('-') => tokens.push(Token::SlashDash),
                    _ => return None,
                }
            }
            '\\' => return None,
            _ => {
                let mut word = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || "{};/\\".contains(c) {
                        break;
                    }
                    if c == '"' {
                        // A property's string value; anything else before a
                        // quote is a raw string (`r"…"`, `r#"…"#`).
                        if !word.ends_with('=') {
                            return None;
                        }
                        chars.next();
                        word.push_str(&string(&mut chars)?);
                        break;
                    }
                    word.push(c);
                    chars.next();
                }
                tokens.push(Token::Word(word));
            }
        }
    }
    Some(tokens)
}

/// The rest of a quoted string whose opening quote was read, its escapes
/// resolved. `None` for an escape [`tokens`] does not read, or no closing
/// quote.
fn string(chars: &mut impl Iterator<Item = char>) -> Option<String> {
    let mut text = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(text),
            '\\' => text.push(match chars.next()? {
                '"' => '"',
                '\\' => '\\',
                '/' => '/',
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                'b' => '\u{8}',
                'f' => '\u{c}',
                _ => return None,
            }),
            c => text.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use dsplr_model::{Layout, Output, OutputEntry, OutputId, OutputState};

    use super::{Token, identifier, names, output_name, quoted, tokens};

    pub(super) fn entry(id: OutputId, connector: &str) -> OutputEntry {
        OutputEntry {
            id,
            output: Some(Output {
                connector: connector.into(),
                description: String::new(),
                physical_size_mm: None,
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                modes: vec![],
            }),
            state: OutputState::default(),
        }
    }

    #[test]
    fn a_monitor_with_a_serial_is_named_by_niri_s_identifier() {
        let aoc = entry(
            OutputId::new("AOC", "Q27G4", "SERIAL0000001", "HDMI-A-1"),
            "HDMI-A-1",
        );
        let panel = entry(OutputId::Connector("eDP-1".into()), "eDP-1");
        let layout = Layout {
            entries: vec![aoc.clone(), panel.clone()],
            ..Default::default()
        };
        assert_eq!(
            output_name(&layout, &aoc).as_deref(),
            Some("AOC Q27G4 SERIAL0000001")
        );
        assert_eq!(output_name(&layout, &panel).as_deref(), Some("eDP-1"));
    }

    /// niri compares identifiers ignoring case, so these two would both
    /// take the first node naming either.
    #[test]
    fn monitors_whose_identifiers_differ_only_in_case_are_named_by_connector() {
        let layout = Layout {
            entries: vec![
                entry(OutputId::new("AOC", "Q27G4", "a1", "DP-1"), "DP-1"),
                entry(OutputId::new("aoc", "q27g4", "A1", "DP-2"), "DP-2"),
            ],
            ..Default::default()
        };
        for (entry, connector) in layout.entries.iter().zip(["DP-1", "DP-2"]) {
            assert_eq!(output_name(&layout, entry).as_deref(), Some(connector));
        }
    }

    /// The winit output of a nested niri 26.04 reports make `Smithay`, model
    /// `Winit` and no serial; `niri msg outputs` calls it
    /// `Smithay Winit Unknown`, and a node by that name, in any case,
    /// applied to it.
    #[test]
    fn niri_matches_a_connector_or_an_identifier_ignoring_case() {
        let mut winit = entry(OutputId::Connector("winit".into()), "winit");
        let output = winit.output.as_mut().unwrap();
        (output.make, output.model) = ("Smithay".into(), "Winit".into());
        for name in [
            "winit",
            "WINIT",
            "Smithay Winit Unknown",
            "smithay winit unknown",
        ] {
            assert!(names(name, &winit), "{name}");
        }
        for name in ["Smithay Winit", "winit-1", ""] {
            assert!(!names(name, &winit), "{name}");
        }
    }

    #[test]
    fn a_monitor_reporting_nothing_has_no_identifier() {
        let bare = entry(OutputId::Connector("HEADLESS-1".into()), "HEADLESS-1");
        assert_eq!(identifier(&bare), None);
        assert!(!names("Unknown Unknown Unknown", &bare));
        assert!(names("headless-1", &bare));
    }

    #[test]
    fn a_name_is_always_quoted_with_its_quotes_and_backslashes_escaped() {
        assert_eq!(quoted("eDP-1").as_deref(), Some("\"eDP-1\""));
        assert_eq!(
            quoted(r#"Acme 27" a\b"#).as_deref(),
            Some(r#""Acme 27\" a\\b""#)
        );
        assert_eq!(quoted("a\nb"), None);
    }

    #[test]
    fn a_line_splits_into_kdl_tokens() {
        let word = |w: &str| Token::Word(w.into());
        let text = |t: &str| Token::Text(t.into());
        assert_eq!(
            tokens(r#"  output "AOC \"Q\" \\x" { off; } // note"#).unwrap(),
            [
                word("output"),
                text(r#"AOC "Q" \x"#),
                Token::Open,
                word("off"),
                Token::Semicolon,
                Token::Close,
            ]
        );
        assert_eq!(
            tokens(r#"include optional=true "outputs.kdl""#).unwrap(),
            [word("include"), word("optional=true"), text("outputs.kdl")]
        );
        assert_eq!(
            tokens(r#"Mod+T hotkey-overlay-title="Open a Terminal" { spawn "x"; }"#).unwrap()[1],
            word("hotkey-overlay-title=Open a Terminal")
        );
        assert_eq!(
            tokens(r#"/-output "x" {"#).unwrap()[..2],
            [Token::SlashDash, word("output")]
        );
        assert_eq!(tokens("// all comment").unwrap(), []);
        for unread in [
            r#"output "open"#,
            "/* block */",
            r##"include r#"raw"#"##,
            r#"x "\u{41}""#,
            "mode \\",
        ] {
            assert_eq!(tokens(unread), None, "{unread}");
        }
    }
}
