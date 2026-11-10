//! scfg, the file format kanshi's config is written in: a transcription of
//! libscfg's parser (`parser.c`, codeberg.org/emersion/libscfg, commit
//! `2fe3162e`), which kanshi 1.9.0 reads its config with.
//!
//! A file is a block of directives. A directive is a name and parameters on
//! one line, each word bare, `"double-quoted"` (a backslash escapes the next
//! character) or `'single-quoted'` (nothing escaped), optionally followed
//! by `{`, a newline, a block of child directives and `}`. A bare word takes
//! a backslash escape too, and cannot hold a quote or a brace. A line whose
//! first word starts with `#` is a comment; a `#` later on a line is part of
//! a word.

use std::iter::Peekable;
use std::str::Chars;

/// How deep blocks may nest. libscfg sets no limit, and kanshi's config
/// nests two deep (a profile, an output's block), so deeper is no config of
/// kanshi's; the limit keeps a hostile file from exhausting the stack.
pub const MAX_NESTING: usize = 16;

/// One directive, with its children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub name: String,
    pub params: Vec<String>,
    pub children: Vec<Self>,
    /// One-based, where the directive starts.
    pub line: usize,
}

/// Why a file is not scfg, where libscfg would refuse it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {reason}")]
pub struct ScfgError {
    pub line: usize,
    pub reason: String,
}

/// The directives of `text`.
///
/// # Errors
///
/// Where libscfg refuses the text: a quote left open, a quote or brace in a
/// bare word, an escaped newline, a `{` not followed by a newline, a block
/// left open, or a `}` with no block to close.
pub fn parse(text: &str) -> Result<Vec<Directive>, ScfgError> {
    let mut parser = Parser {
        chars: text.chars().peekable(),
        line: 1,
        depth: 0,
    };
    let (block, closed) = parser.block()?;
    if closed {
        return Err(parser.error("unexpected '}'"));
    }
    Ok(block)
}

struct Parser<'a> {
    chars: Peekable<Chars<'a>>,
    line: usize,
    /// How many blocks are open.
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, reason: impl Into<String>) -> ScfgError {
        ScfgError {
            line: self.line,
            reason: reason.into(),
        }
    }

    fn next(&mut self) -> Option<char> {
        let c = self.chars.next();
        if c == Some('\n') {
            self.line += 1;
        }
        c
    }

    fn skip_blanks(&mut self) {
        while matches!(self.chars.peek(), Some(' ' | '\t')) {
            self.next();
        }
    }

    /// A block's directives, up to a `}` (the bool says it was one) or the
    /// end of the text.
    fn block(&mut self) -> Result<(Vec<Directive>, bool), ScfgError> {
        let mut directives = Vec::new();
        loop {
            self.skip_blanks();
            match self.chars.peek() {
                None => return Ok((directives, false)),
                Some('\n') => {
                    self.next();
                }
                Some('#') => while self.next().is_some_and(|c| c != '\n') {},
                Some('}') => {
                    self.next();
                    return Ok((directives, true));
                }
                Some(_) => directives.push(self.directive()?),
            }
        }
    }

    fn directive(&mut self) -> Result<Directive, ScfgError> {
        let line = self.line;
        let name = self.word()?;
        self.skip_blanks();
        let mut params = Vec::new();
        let mut children = Vec::new();
        loop {
            match self.chars.peek() {
                None => break,
                Some('\n') => {
                    self.next();
                    break;
                }
                Some('{') => {
                    self.next();
                    if self.depth == MAX_NESTING {
                        return Err(self.error(format!("blocks nest more than {MAX_NESTING} deep")));
                    }
                    self.skip_blanks();
                    if self.next() != Some('\n') {
                        return Err(self.error("expected a newline after '{'"));
                    }
                    self.depth += 1;
                    let (block, closed) = self.block()?;
                    self.depth -= 1;
                    if !closed {
                        return Err(self.error("expected '}'"));
                    }
                    children = block;
                    break;
                }
                Some('}') => return Err(self.error("unexpected '}'")),
                Some(_) => {
                    params.push(self.word()?);
                    self.skip_blanks();
                }
            }
        }
        Ok(Directive {
            name,
            params,
            children,
            line,
        })
    }

    fn word(&mut self) -> Result<String, ScfgError> {
        match self.chars.peek() {
            Some('"') => {
                self.next();
                self.quoted('"', true)
            }
            Some('\'') => {
                self.next();
                self.quoted('\'', false)
            }
            _ => self.atom(),
        }
    }

    fn quoted(&mut self, quote: char, escapes: bool) -> Result<String, ScfgError> {
        let mut word = String::new();
        loop {
            match self.chars.peek() {
                None | Some('\n') => return Err(self.error("a quoted string left open")),
                Some(&c) if c == quote => {
                    self.next();
                    return Ok(word);
                }
                Some('\\') if escapes => {
                    self.next();
                    word.push(self.escaped()?);
                }
                Some(&c) => {
                    self.next();
                    word.push(c);
                }
            }
        }
    }

    fn atom(&mut self) -> Result<String, ScfgError> {
        let mut word = String::new();
        loop {
            match self.chars.peek() {
                None | Some(' ' | '\t' | '\n') => return Ok(word),
                Some(&c @ ('"' | '\'' | '{' | '}')) => {
                    return Err(self.error(format!("unexpected '{c}' in a word")));
                }
                Some('\\') => {
                    self.next();
                    word.push(self.escaped()?);
                }
                Some(&c) => {
                    self.next();
                    word.push(c);
                }
            }
        }
    }

    /// The character after a backslash: any but a newline, which libscfg
    /// refuses to escape. At the end of the text, nothing.
    fn escaped(&mut self) -> Result<char, ScfgError> {
        match self.chars.peek() {
            Some('\n') => Err(self.error("a backslash before a newline")),
            Some(&c) => {
                self.next();
                Ok(c)
            }
            None => Err(self.error("a backslash at the end of the file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Directive, MAX_NESTING, ScfgError, parse};

    fn directive(name: &str, params: &[&str], line: usize) -> Directive {
        Directive {
            name: name.into(),
            params: params.iter().map(|&p| p.into()).collect(),
            children: Vec::new(),
            line,
        }
    }

    #[test]
    fn directives_blocks_quotes_and_comments() {
        let text = "# a comment\n\
                    include /etc/kanshi/config.d/*\n\
                    \n\
                    profile {\n\
                    \toutput LVDS-1 disable\n\
                    \toutput \"Some Company ASDF 4242\" {\n\
                    \t\tmode 1600x900\n\
                    \t\t# inside too\n\
                    \t\tposition 0,0\n\
                    \t}\n\
                    }\n\
                    profile 'nomad' {\n\
                    \toutput LVDS-1 enable scale 2\n\
                    }\n";
        let parsed = parse(text).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(
            parsed[0],
            directive("include", &["/etc/kanshi/config.d/*"], 2)
        );
        let profile = &parsed[1];
        assert_eq!(
            (profile.name.as_str(), profile.params.len()),
            ("profile", 0)
        );
        assert_eq!(
            profile.children[0],
            directive("output", &["LVDS-1", "disable"], 5)
        );
        let block = &profile.children[1];
        assert_eq!(block.params, ["Some Company ASDF 4242"]);
        assert_eq!(
            block.children,
            [
                directive("mode", &["1600x900"], 7),
                directive("position", &["0,0"], 9),
            ]
        );
        assert_eq!(parsed[2].params, ["nomad"]);
        assert_eq!(
            parsed[2].children[0].params,
            ["LVDS-1", "enable", "scale", "2"]
        );
    }

    #[test]
    fn blocks_nest_no_deeper_than_the_limit() {
        let nested = |depth: usize| "a {\n".repeat(depth) + &"}\n".repeat(depth);
        assert!(parse(&nested(MAX_NESTING)).is_ok());
        let error = parse(&nested(MAX_NESTING + 1)).expect_err("too deep");
        assert_eq!(error.line, MAX_NESTING + 1, "{error}");
        assert!(error.reason.contains("nest"), "{error}");
    }

    #[test]
    fn escapes_follow_libscfg() {
        let parsed = parse(r#"a b\ c "d \"e\"" 'f \g' h#i"#).unwrap();
        assert_eq!(parsed[0].params, ["b c", "d \"e\"", r"f \g", "h#i"]);
    }

    #[test]
    fn what_libscfg_refuses_is_refused_with_its_line() {
        for (text, line) in [
            ("a \"open\n", 1),
            ("a 'open", 1),
            ("a b\"c\n", 1),
            ("a b{\n", 1),
            ("a {b\n}\n", 1),
            ("a {\nb\n", 3),
            ("}\n", 1),
            ("a }\n", 1),
            ("a b\\\nc\n", 1),
        ] {
            let error: ScfgError = parse(text).expect_err(text);
            assert_eq!(error.line, line, "{text:?}: {error}");
        }
    }
}
