//! Reading an `outputs.kdl` back, in the form dsplr writes it: `output`
//! nodes holding only settings dsplr writes ([`super::SETTINGS`]), each node
//! starting a line and ending one, with blank and `//` comment lines between
//! them. Anything else is refused rather than read past, since saving over
//! it would lose it.

use super::{SETTINGS, Token, on_demand, tokens};

/// One `output` node of an `outputs.kdl`: the name it is for, and its text
/// as it stands in the file, from the line it starts on to the line its `}`
/// is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputsNode {
    pub name: String,
    pub text: String,
    /// Whether it sets `variable-refresh-rate on-demand=true`, which
    /// wlr-output-management reads only as on or off.
    pub on_demand_vrr: bool,
}

/// Why an `outputs.kdl` is not in the form dsplr reads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct ReadError {
    /// One-based.
    pub line: usize,
    pub reason: String,
}

/// The `output` nodes of `text`, an `outputs.kdl` with or without dsplr's
/// header (a `//` comment like any other), in file order.
///
/// # Errors
///
/// At the first line that is not a blank line, a comment, or part of an
/// `output` node holding only what dsplr writes, each with the arguments it
/// takes: another node or setting, `/-` or `/*` comments, raw or multi-line
/// strings, a node that does not start or end its line, or one left open.
pub fn read_outputs(text: &str) -> Result<Vec<OutputsNode>, ReadError> {
    let mut nodes = Vec::new();
    // The node being read: its name, its text so far, its first line, and
    // whether it sets on-demand adaptive sync.
    let mut open: Option<(String, String, usize, bool)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |reason: String| Err(ReadError { line, reason });
        let Some(tokens) = tokens(raw) else {
            return fail(
                "a `/*` comment, a raw or unclosed string, an escape or a line continuation, which dsplr does not write"
                    .to_owned(),
            );
        };
        if tokens.contains(&Token::SlashDash) {
            return fail("a `/-` comment, which dsplr does not write".to_owned());
        }
        let mut rest = tokens.as_slice();
        if open.is_none() {
            if rest.is_empty() {
                continue;
            }
            if let [Token::Word(node), ..] = rest
                && node != "output"
            {
                return fail(format!("`{node}`, where dsplr writes only `output` nodes"));
            }
            let [_, Token::Text(name), Token::Open, after @ ..] = rest else {
                return fail(
                    "not the start of an `output \"<name>\" {` node, where dsplr writes only those"
                        .to_owned(),
                );
            };
            open = Some((name.clone(), String::new(), line, false));
            rest = after;
        }
        let (_, text, _, on_demand_vrr) = open.as_mut().expect("a node is open");
        text.push_str(raw);
        text.push('\n');
        // The settings on this line, up to the `}` that closes the node.
        let closed = rest.iter().position(|token| *token == Token::Close);
        let (inside, after) = match closed {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, &[][..]),
        };
        for setting in inside.split(|token| *token == Token::Semicolon) {
            if let Err(reason) = check(setting) {
                return fail(reason);
            }
            *on_demand_vrr |= on_demand(setting);
        }
        if closed.is_some() {
            if after.iter().any(|token| *token != Token::Semicolon) {
                return fail("more after an `output` node's `}` on its line".to_owned());
            }
            let (name, text, _, on_demand_vrr) = open.take().expect("a node is open");
            nodes.push(OutputsNode {
                name,
                text,
                on_demand_vrr,
            });
        }
    }
    if let Some((name, _, line, _)) = open {
        return Err(ReadError {
            line,
            reason: format!("the node for {name} is never closed"),
        });
    }
    Ok(nodes)
}

/// Checks that `setting`, the tokens of one node inside an `output` node,
/// is empty or a setting dsplr writes with the arguments it takes:
/// properties (`custom=true`, `x=0`) where they belong, and a value for
/// `mode`, `scale` and `transform`. The values are not checked: dsplr either
/// replaces the whole node or writes it back as it was.
fn check(setting: &[Token]) -> Result<(), String> {
    let Some((first, arguments)) = setting.split_first() else {
        return Ok(());
    };
    let Token::Word(name) = first else {
        return Err("something other than a setting inside an `output` node".to_owned());
    };
    if !SETTINGS.contains(&name.as_str()) {
        return Err(format!("`{name}`, a setting dsplr does not write"));
    }
    let property = |token: &Token| matches!(token, Token::Word(word) if word.contains('='));
    let values = arguments.iter().filter(|token| !property(token)).count();
    let properties = arguments.len() - values;
    let (takes_values, takes_properties) = match name.as_str() {
        "off" => (0, false),
        "mode" => (1, true),
        "scale" | "transform" => (1, false),
        "position" => (0, true),
        _ => (0, true),
    };
    if values != takes_values || (properties > 0 && !takes_properties) {
        return Err(format!("`{name}` with arguments dsplr does not write"));
    }
    if arguments.contains(&Token::Open) {
        return Err(format!("`{name}` with a block of its own"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OutputsNode, read_outputs};
    use crate::managed::{Comment, with_header};
    use crate::niri::output_nodes;
    use crate::niri::write::tests::{OWNERS_NODES, owners_layout};

    #[test]
    fn what_dsplr_writes_reads_back_node_by_node() {
        let file = with_header(OWNERS_NODES, Comment::Slashes);
        let nodes = read_outputs(&file).unwrap();
        let (panel, aoc) = OWNERS_NODES.split_once("\n\n").unwrap();
        assert_eq!(
            nodes,
            [
                OutputsNode {
                    name: "eDP-1".into(),
                    text: format!("{panel}\n"),
                    on_demand_vrr: false,
                },
                OutputsNode {
                    name: "AOC Q27G4 SERIAL0000001".into(),
                    text: aoc.into(),
                    on_demand_vrr: false,
                },
            ]
        );
        assert_eq!(output_nodes(&owners_layout()).unwrap(), OWNERS_NODES);
    }

    #[test]
    fn one_line_nodes_and_every_setting_s_form_are_read_too() {
        let text = "// by hand\noutput \"DP-9\" { off; }\n\noutput \"a \\\"b\\\"\" {\n    mode custom=true \"1920x1080@59.94\"; scale 2\n    transform \"90\"\n    position x=10 y=-5\n    variable-refresh-rate on-demand=true\n};\n";
        let nodes = read_outputs(text).unwrap();
        let names: Vec<&str> = nodes.iter().map(|node| node.name.as_str()).collect();
        assert_eq!(names, ["DP-9", "a \"b\""]);
        assert_eq!(nodes[0].text, "output \"DP-9\" { off; }\n");
        assert!(!nodes[0].on_demand_vrr);
        assert!(nodes[1].on_demand_vrr, "{}", nodes[1].text);
        assert!(nodes[1].text.starts_with("output \"a"), "{}", nodes[1].text);
        assert!(nodes[1].text.ends_with("};\n"), "{}", nodes[1].text);
    }

    #[test]
    fn anything_dsplr_does_not_write_is_refused_naming_its_line() {
        for (text, line, reason) in [
            (
                "output \"DP-1\" {\n    focus-at-startup\n}\n",
                2,
                "`focus-at-startup`",
            ),
            (
                "output \"DP-1\" {\n    backdrop-color \"#000\"\n}\n",
                2,
                "`backdrop-color`",
            ),
            ("layout {\n}\n", 1, "`layout`"),
            ("output DP-1 {\n}\n", 1, "not the start"),
            ("output \"DP-1\"\n", 1, "not the start"),
            (
                "output \"DP-1\" {\n    off 1\n}\n",
                2,
                "`off` with arguments",
            ),
            (
                "output \"DP-1\" {\n    scale\n}\n",
                2,
                "`scale` with arguments",
            ),
            (
                "output \"DP-1\" {\n    scale x=1\n}\n",
                2,
                "`scale` with arguments",
            ),
            (
                "output \"DP-1\" {\n    position 0 0\n}\n",
                2,
                "`position` with arguments",
            ),
            ("output \"DP-1\" {\n    /-off\n}\n", 2, "`/-`"),
            ("/* off */\n", 1, "`/*`"),
            (
                "output \"DP-1\" { off; } output \"DP-2\" { off; }\n",
                1,
                "after",
            ),
            ("output \"DP-1\" {\n    off\n", 1, "never closed"),
            ("}\n", 1, "not the start"),
            ("output \"open {\n", 1, "unclosed string"),
        ] {
            let error = read_outputs(text).unwrap_err();
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.reason.contains(reason), "{text:?}: {error}");
        }
    }

    #[test]
    fn an_empty_file_has_no_nodes() {
        assert_eq!(read_outputs(""), Ok(vec![]));
        assert_eq!(read_outputs("\n// nothing\n"), Ok(vec![]));
    }
}
