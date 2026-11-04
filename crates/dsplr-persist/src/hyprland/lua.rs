//! The part of Lua a `monitors.lua` uses: comments, and `hl.monitor({ … })`
//! calls whose fields are strings, numbers or booleans. Anything else is a
//! syntax error naming the line, never skipped, because a file dsplr cannot
//! fully read is one it must not overwrite as if it had.

/// One `hl.monitor({ … })` call.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The line the call starts on, counting from 1.
    pub line: usize,
    /// The call as written, from `hl` to the closing `)`.
    pub text: String,
    pub fields: Vec<Field>,
}

/// One `key = value` in a call's table.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub line: usize,
    pub key: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Number(f64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub line: usize,
    pub message: String,
}

/// Every call in `source`, in order.
pub fn parse(source: &str) -> Result<Vec<Call>, SyntaxError> {
    let mut parser = Parser {
        source,
        pos: 0,
        line: 1,
    };
    let mut calls = Vec::new();
    loop {
        parser.skip_trivia()?;
        if parser.rest().is_empty() {
            return Ok(calls);
        }
        calls.push(parser.call()?);
    }
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
    line: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.source[self.pos..]
    }

    fn error<T>(&self, message: impl Into<String>) -> Result<T, SyntaxError> {
        Err(SyntaxError {
            line: self.line,
            message: message.into(),
        })
    }

    /// Moves past `len` bytes, counting the newlines in them.
    fn advance(&mut self, len: usize) {
        self.line += self.rest()[..len].matches('\n').count();
        self.pos += len;
    }

    /// Skips whitespace and comments: `--` to the end of the line, or a
    /// `--[[ … ]]` block.
    fn skip_trivia(&mut self) -> Result<(), SyntaxError> {
        loop {
            let trimmed = self.rest().trim_start();
            self.advance(self.rest().len() - trimmed.len());
            if self.rest().starts_with("--[[") {
                let Some(end) = self.rest().find("]]") else {
                    return self.error("a `--[[` comment is never closed");
                };
                self.advance(end + 2);
            } else if self.rest().starts_with("--") {
                let end = self.rest().find('\n').unwrap_or_else(|| self.rest().len());
                self.advance(end);
            } else {
                return Ok(());
            }
        }
    }

    /// Skips trivia, then `token` or a syntax error naming it.
    fn expect(&mut self, token: &str) -> Result<(), SyntaxError> {
        self.skip_trivia()?;
        if self.rest().starts_with(token) {
            self.advance(token.len());
            Ok(())
        } else {
            self.error(format!("expected `{token}`, found {}", self.found()))
        }
    }

    /// Skips trivia, then takes `token` if it is next.
    fn eat(&mut self, token: &str) -> Result<bool, SyntaxError> {
        self.skip_trivia()?;
        let next = self.rest().starts_with(token);
        if next {
            self.advance(token.len());
        }
        Ok(next)
    }

    /// What is next, for an error message.
    fn found(&self) -> String {
        match self.rest().split_whitespace().next() {
            Some(word) => format!("`{}`", word.chars().take(20).collect::<String>()),
            None => "the end of the file".to_owned(),
        }
    }

    fn call(&mut self) -> Result<Call, SyntaxError> {
        let (start, line) = (self.pos, self.line);
        if !self.rest().starts_with("hl") {
            return self.error(format!(
                "expected an `hl.monitor({{ … }})` call, found {}",
                self.found()
            ));
        }
        self.advance("hl".len());
        for token in [".", "monitor", "(", "{"] {
            self.expect(token)?;
        }
        let mut fields = Vec::new();
        loop {
            if self.eat("}")? {
                break;
            }
            fields.push(self.field()?);
            if !self.eat(",")? && !self.eat(";")? {
                self.expect("}")?;
                break;
            }
        }
        self.expect(")")?;
        let text = self.source[start..self.pos].to_owned();
        // A statement may end in `;`.
        self.eat(";")?;
        Ok(Call { line, text, fields })
    }

    fn field(&mut self) -> Result<Field, SyntaxError> {
        self.skip_trivia()?;
        let line = self.line;
        let len = self
            .rest()
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or_else(|| self.rest().len());
        if len == 0 || self.rest().starts_with(|c: char| c.is_ascii_digit()) {
            return self.error(format!("expected a field name, found {}", self.found()));
        }
        let key = self.rest()[..len].to_owned();
        self.advance(len);
        self.expect("=")?;
        let value = self.value()?;
        Ok(Field { line, key, value })
    }

    fn value(&mut self) -> Result<Value, SyntaxError> {
        self.skip_trivia()?;
        let rest = self.rest();
        if rest.starts_with('"') || rest.starts_with('\'') {
            return self.string().map(Value::Str);
        }
        for (word, value) in [("true", true), ("false", false)] {
            let after = rest.get(word.len()..).and_then(|s| s.chars().next());
            if rest.starts_with(word)
                && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                self.advance(word.len());
                return Ok(Value::Bool(value));
            }
        }
        let len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
            .unwrap_or(rest.len());
        match rest[..len].parse::<f64>() {
            Ok(number) if len > 0 && number.is_finite() => {
                self.advance(len);
                Ok(Value::Number(number))
            }
            _ => self.error(format!(
                "expected a string, number or boolean, found {}",
                self.found()
            )),
        }
    }

    /// A quoted string, its escapes decoded.
    fn string(&mut self) -> Result<String, SyntaxError> {
        let quote = self.rest().chars().next().unwrap_or('"');
        self.advance(1);
        let mut bytes = Vec::new();
        loop {
            let Some(c) = self.rest().chars().next() else {
                return self.error("a string is never closed");
            };
            if c == '\n' {
                return self.error("a string runs past the end of its line");
            }
            self.advance(c.len_utf8());
            if c == quote {
                break;
            }
            if c != '\\' {
                bytes.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                continue;
            }
            let Some(escaped) = self.rest().chars().next() else {
                return self.error("a string is never closed");
            };
            let byte = match escaped {
                'n' => b'\n',
                'r' => b'\r',
                't' => b'\t',
                '\\' => b'\\',
                '"' => b'"',
                '\'' => b'\'',
                '0'..='9' => {
                    let digits = self
                        .rest()
                        .chars()
                        .take(3)
                        .take_while(char::is_ascii_digit)
                        .count();
                    let Ok(byte) = self.rest()[..digits].parse::<u8>() else {
                        return self.error("a `\\ddd` escape is above 255");
                    };
                    self.advance(digits - 1);
                    byte
                }
                other => return self.error(format!("the escape `\\{other}` is not supported")),
            };
            self.advance(1);
            bytes.push(byte);
        }
        String::from_utf8(bytes).or_else(|_| self.error("a string is not valid UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Call, Field, SyntaxError, Value, parse};

    #[test]
    fn calls_comments_and_every_value_kind_parse() {
        let source = "-- a comment\n--[[ a\nblock ]]\nhl.monitor({\n    output = \"eDP-1\",\n    disabled = true\n})\nhl.monitor({ output = 'x', scale = 1.0, transform = -1, }); -- trailing\n";
        let calls = parse(source).unwrap();
        assert_eq!(
            calls[0],
            Call {
                line: 4,
                text: "hl.monitor({\n    output = \"eDP-1\",\n    disabled = true\n})".into(),
                fields: vec![
                    Field {
                        line: 5,
                        key: "output".into(),
                        value: Value::Str("eDP-1".into()),
                    },
                    Field {
                        line: 6,
                        key: "disabled".into(),
                        value: Value::Bool(true),
                    },
                ],
            }
        );
        assert_eq!(calls[1].line, 8);
        let values: Vec<_> = calls[1].fields.iter().map(|f| f.value.clone()).collect();
        assert_eq!(
            values,
            [
                Value::Str("x".into()),
                Value::Number(1.0),
                Value::Number(-1.0)
            ]
        );
    }

    #[test]
    fn string_escapes_decode() {
        let calls = parse(r#"hl.monitor({ output = "a\"b\\c\n\0079\t" })"#).unwrap();
        assert_eq!(
            calls[0].fields[0].value,
            Value::Str("a\"b\\c\n\u{7}9\t".into())
        );
    }

    #[test]
    fn anything_else_is_an_error_naming_its_line() {
        let error = |source: &str| parse(source).unwrap_err();
        assert_eq!(
            error("\nlocal x = 1"),
            SyntaxError {
                line: 2,
                message: "expected an `hl.monitor({ … })` call, found `local`".into(),
            }
        );
        assert_eq!(error("hl.monitor({ output = foo })").line, 1);
        assert_eq!(error("hl.monitor({\n output = \"a\" scale = 1 })").line, 2);
        assert_eq!(error("hl.monitor({ output = \"a })").line, 1);
        assert_eq!(error("hl.monitor({ output = \"a\" }").line, 1);
        assert_eq!(error("hl.monitor({ output = \"\\q\" })").line, 1);
        assert_eq!(error("hl.monitor({ output = \"\\300\" })").line, 1);
        assert_eq!(error("--[[ never closed").line, 1);
        assert_eq!(error("hl.monitor({ 1x = 2 })").line, 1);
    }
}
