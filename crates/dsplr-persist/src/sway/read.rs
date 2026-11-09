//! Reading an `outputs` file back, in the form dsplr writes it: `output`
//! commands whose settings are all ones dsplr writes ([`super::SETTINGS`]),
//! as blocks or on one line, with blank and `#` comment lines between them.
//! Anything else is refused rather than read past, since saving over it
//! would lose it.

use super::{SETTINGS, words};

/// One `output` command of an `outputs` file: the name it is for, and its
/// text as it stands in the file, a block from its `{` line to its `}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputsBlock {
    pub name: String,
    pub text: String,
}

/// Why an `outputs` file is not in the form dsplr reads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct ReadError {
    /// One-based.
    pub line: usize,
    pub reason: String,
}

/// The `output` commands of `text`, an `outputs` file with or without
/// dsplr's header (a `#` comment like any other), in file order.
///
/// # Errors
///
/// At the first line that is not a blank line, a comment, or part of an
/// `output` command setting only what dsplr writes, each with its
/// arguments: a backslash (an escape or a line continuation), a `$`
/// variable, a quote left open, another command or setting, a block left
/// open or closed twice.
pub fn read_outputs(text: &str) -> Result<Vec<OutputsBlock>, ReadError> {
    let mut blocks = Vec::new();
    // The block being read: its name, its text so far, and its first line.
    let mut open: Option<(String, String, usize)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |reason: String| Err(ReadError { line, reason });
        let trimmed = raw.trim();
        if let Some((_, text, _)) = &mut open {
            text.push_str(raw);
            text.push('\n');
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.contains('\\') {
            return fail("a backslash, which dsplr does not write".to_owned());
        }
        if trimmed.contains('$') {
            return fail("a `$` variable, which dsplr cannot resolve".to_owned());
        }
        let Some(words) = words(trimmed) else {
            return fail("a quote left open".to_owned());
        };
        if open.is_some() {
            if words == ["}"] {
                let (name, text, _) = open.take().expect("a block is open");
                blocks.push(OutputsBlock { name, text });
                continue;
            }
            if let Err(reason) = settings(&words) {
                return fail(reason);
            }
            continue;
        }
        let (Some("output"), Some(name)) = (words.first().map(String::as_str), words.get(1)) else {
            return fail(format!(
                "`{}`, where dsplr writes only `output` commands",
                words.first().map_or("", String::as_str)
            ));
        };
        if words[2..] == ["{"] {
            open = Some((name.clone(), format!("{raw}\n"), line));
            continue;
        }
        if words.len() == 2 {
            return fail(format!("`output {name}` sets nothing"));
        }
        if let Err(reason) = settings(&words[2..]) {
            return fail(reason);
        }
        blocks.push(OutputsBlock {
            name: name.clone(),
            text: format!("{raw}\n"),
        });
    }
    if let Some((name, _, line)) = open {
        return Err(ReadError {
            line,
            reason: format!("the block for {name} is never closed"),
        });
    }
    Ok(blocks)
}

/// Checks that `words` are settings dsplr writes, each with as many
/// arguments as it takes. The arguments' values are not checked: dsplr
/// either replaces the whole command or writes it back as it was.
fn settings(words: &[String]) -> Result<(), String> {
    let mut rest = words;
    while let Some((setting, after)) = rest.split_first() {
        let setting = setting.as_str();
        if !SETTINGS.contains(&setting) {
            return Err(format!("`{setting}`, a setting dsplr does not write"));
        }
        let takes = match setting {
            "enable" | "disable" => 0,
            "position" | "pos" => 2,
            "mode" | "resolution" | "res" if after.first().is_some_and(|w| w == "--custom") => 2,
            _ => 1,
        };
        if after.len() < takes {
            return Err(format!("`{setting}` without its value"));
        }
        rest = &after[takes..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OutputsBlock, read_outputs};
    use crate::managed::{Comment, with_header};
    use crate::sway::output_blocks;

    #[test]
    fn what_dsplr_writes_reads_back_block_by_block() {
        let body = "output eDP-1 {\n    disable\n}\n\noutput \"AOC Q27G4 SER\" {\n    enable\n    mode --custom 1920x1080@59.94Hz\n    position -1920 240\n    scale 1.25\n    transform 270\n    adaptive_sync on\n}\n";
        let blocks = read_outputs(&with_header(body, Comment::Hash)).unwrap();
        assert_eq!(
            blocks,
            [
                OutputsBlock {
                    name: "eDP-1".into(),
                    text: "output eDP-1 {\n    disable\n}\n".into(),
                },
                OutputsBlock {
                    name: "AOC Q27G4 SER".into(),
                    text: body.split_once("\n\n").unwrap().1.into(),
                },
            ]
        );
    }

    #[test]
    fn the_writer_s_output_reads_back() {
        let layout = dsplr_model::Layout {
            entries: vec![dsplr_model::OutputEntry {
                id: dsplr_model::OutputId::new("AOC", "Q27G4", "S1", "DP-1"),
                output: None,
                state: dsplr_model::OutputState {
                    mode: Some(dsplr_model::Mode::new(2560, 1440, 120_000)),
                    ..dsplr_model::OutputState::default()
                },
            }],
        };
        let text = output_blocks(&layout).unwrap();
        let blocks = read_outputs(&text).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].name, "AOC Q27G4 S1");
        assert_eq!(blocks[0].text, text);
    }

    #[test]
    fn one_line_commands_and_comments_are_read_too() {
        let text = "# by hand\noutput * scale 2\n\noutput HDMI-A-1 res 1920x1080 pos 0 0 transform normal enable\n";
        let blocks = read_outputs(text).unwrap();
        let names: Vec<&str> = blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["*", "HDMI-A-1"]);
        assert_eq!(blocks[0].text, "output * scale 2\n");
    }

    #[test]
    fn anything_dsplr_does_not_write_is_refused_naming_its_line() {
        for (text, line, reason) in [
            ("output DP-1 bg ~/a.png fill\n", 1, "`bg`"),
            (
                "output DP-1 {\n    scale_filter nearest\n}\n",
                2,
                "`scale_filter`",
            ),
            ("output DP-1 {\n    power on\n}\n", 2, "`power`"),
            ("set $mon DP-1\n", 1, "`$` variable"),
            ("workspace 1 output DP-1\n", 1, "`workspace`"),
            ("output DP-1 pos 0\n", 1, "`pos` without its value"),
            ("output DP-1 mode --custom\n", 1, "`mode` without its value"),
            ("output DP-1\n", 1, "sets nothing"),
            ("output \"DP-1 {\n", 1, "quote left open"),
            ("output DP-1 scale \\\n  2\n", 1, "backslash"),
            ("output DP-1 {\n    enable\n", 1, "never closed"),
            ("}\n", 1, "`}`"),
            ("output DP-1 {\n  output DP-2 {\n", 2, "`output`"),
        ] {
            let error = read_outputs(text).unwrap_err();
            assert_eq!(error.line, line, "{text:?}: {error}");
            assert!(error.reason.contains(reason), "{text:?}: {error}");
        }
    }

    #[test]
    fn an_empty_file_has_no_blocks() {
        assert_eq!(read_outputs(""), Ok(vec![]));
        assert_eq!(read_outputs("\n# nothing\n"), Ok(vec![]));
    }
}
