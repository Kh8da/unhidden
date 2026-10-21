//! Minimal, lossless reader/writer for Valve's text KeyValues (VDF) format.
//!
//! Strings are kept exactly as they appear in the file (escape sequences are
//! not decoded), so writing a parsed file back produces the same content with
//! only whitespace normalised. Use [`unescape`] when a value is shown to a user.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Obj(Obj),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Obj {
    pub entries: Vec<(String, Value)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (line {})", self.message, self.line)
    }
}

impl std::error::Error for ParseError {}

impl Obj {
    /// Case-insensitive lookup; Steam has used both `MostRecent` and `mostrecent`.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::Str(s)) => Some(s),
            _ => None,
        }
    }

    pub fn get_obj(&self, key: &str) -> Option<&Obj> {
        match self.get(key) {
            Some(Value::Obj(o)) => Some(o),
            _ => None,
        }
    }

    pub fn get_obj_mut(&mut self, key: &str) -> Option<&mut Obj> {
        match self.get_mut(key) {
            Some(Value::Obj(o)) => Some(o),
            _ => None,
        }
    }

    /// Sets a string value, keeping the existing key's spelling and position
    /// if it exists, otherwise appending it.
    pub fn set_str(&mut self, key: &str, value: &str) {
        match self.get_mut(key) {
            Some(v) => *v = Value::Str(value.to_string()),
            None => self
                .entries
                .push((key.to_string(), Value::Str(value.to_string()))),
        }
    }

    /// Removes every entry with this key. Returns true if anything was removed.
    pub fn remove(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        self.entries.len() != before
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
}

enum Token {
    Str(String),
    Open,
    Close,
}

impl<'a> Parser<'a> {
    fn err(&self, message: &str) -> ParseError {
        ParseError {
            message: message.to_string(),
            line: self.line,
        }
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next();
        if c == Some('\n') {
            self.line += 1;
        }
        c
    }

    fn skip_trivia(&mut self) {
        loop {
            match self.chars.peek() {
                Some(c) if c.is_whitespace() || *c == '\u{feff}' => {
                    self.bump();
                }
                Some('/') => {
                    // `//` line comment. A single `/` is part of an unquoted token.
                    let mut ahead = self.chars.clone();
                    ahead.next();
                    if ahead.peek() == Some(&'/') {
                        while let Some(c) = self.bump() {
                            if c == '\n' {
                                break;
                            }
                        }
                    } else {
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_trivia();
        let Some(&c) = self.chars.peek() else {
            return Ok(None);
        };
        match c {
            '{' => {
                self.bump();
                Ok(Some(Token::Open))
            }
            '}' => {
                self.bump();
                Ok(Some(Token::Close))
            }
            '"' => {
                self.bump();
                let mut s = String::new();
                loop {
                    match self.bump() {
                        None => return Err(self.err("unterminated string")),
                        Some('"') => break,
                        Some('\\') => {
                            s.push('\\');
                            match self.bump() {
                                Some(n) => s.push(n),
                                None => return Err(self.err("unterminated string")),
                            }
                        }
                        Some(ch) => s.push(ch),
                    }
                }
                Ok(Some(Token::Str(s)))
            }
            _ => {
                let mut s = String::new();
                while let Some(&ch) = self.chars.peek() {
                    if ch.is_whitespace() || ch == '{' || ch == '}' || ch == '"' {
                        break;
                    }
                    s.push(ch);
                    self.bump();
                }
                Ok(Some(Token::Str(s)))
            }
        }
    }

    /// Skips a platform conditional like `[$WIN32]` that may follow a value.
    fn skip_conditional(&mut self) {
        self.skip_trivia();
        if self.chars.peek() == Some(&'[') {
            while let Some(c) = self.bump() {
                if c == ']' || c == '\n' {
                    break;
                }
            }
        }
    }

    fn parse_obj(&mut self, top_level: bool) -> Result<Obj, ParseError> {
        let mut obj = Obj::default();
        loop {
            let key = match self.next_token()? {
                None if top_level => return Ok(obj),
                None => return Err(self.err("missing closing brace")),
                Some(Token::Close) if !top_level => return Ok(obj),
                Some(Token::Close) => return Err(self.err("unexpected closing brace")),
                Some(Token::Open) => return Err(self.err("expected a key, found an opening brace")),
                Some(Token::Str(k)) => k,
            };
            self.skip_conditional();
            let value = match self.next_token()? {
                None => return Err(self.err("key without a value")),
                Some(Token::Close) => return Err(self.err("key without a value")),
                Some(Token::Open) => Value::Obj(self.parse_obj(false)?),
                Some(Token::Str(v)) => {
                    self.skip_conditional();
                    Value::Str(v)
                }
            };
            obj.entries.push((key, value));
        }
    }
}

pub fn parse(text: &str) -> Result<Obj, ParseError> {
    let mut p = Parser {
        chars: text.chars().peekable(),
        line: 1,
    };
    p.parse_obj(true)
}

fn write_obj(obj: &Obj, depth: usize, out: &mut String) {
    for (key, value) in &obj.entries {
        let indent = "\t".repeat(depth);
        match value {
            Value::Str(s) => {
                out.push_str(&format!("{indent}\"{key}\"\t\t\"{s}\"\n"));
            }
            Value::Obj(child) => {
                out.push_str(&format!("{indent}\"{key}\"\n{indent}{{\n"));
                write_obj(child, depth + 1, out);
                out.push_str(&format!("{indent}}}\n"));
            }
        }
    }
}

pub fn write(obj: &Obj) -> String {
    let mut out = String::new();
    write_obj(obj, 0, &mut out);
    out
}

/// Decodes VDF escape sequences for display. Unknown escapes are kept as-is.
pub fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_objects_and_escapes() {
        let text = "\"users\"\n{\n\t\"1\"\n\t{\n\t\t\"Name\"\t\t\"a \\\"quoted\\\" \\\\ name\"\n\t}\n}\n";
        let root = parse(text).unwrap();
        let users = root.get_obj("users").unwrap();
        let one = users.get_obj("1").unwrap();
        assert_eq!(one.get_str("name"), Some("a \\\"quoted\\\" \\\\ name"));
        assert_eq!(unescape(one.get_str("name").unwrap()), "a \"quoted\" \\ name");
    }

    #[test]
    fn round_trips_byte_for_byte_on_steam_formatting() {
        let text = "\"users\"\n{\n\t\"76561198000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"alpha\"\n\t\t\"PersonaName\"\t\t\"Al \\\"pha\\\"\"\n\t}\n}\n";
        assert_eq!(write(&parse(text).unwrap()), text);
    }

    #[test]
    fn handles_comments_unquoted_tokens_bom_and_conditionals() {
        let text = "\u{feff}// header\nroot\n{\n\tkey value [$WIN32]\n\t\"path\" \"c:/a/b\" // trailing\n\tempty \"\"\n}\n";
        let root = parse(text).unwrap();
        let r = root.get_obj("root").unwrap();
        assert_eq!(r.get_str("key"), Some("value"));
        assert_eq!(r.get_str("path"), Some("c:/a/b"));
        assert_eq!(r.get_str("empty"), Some(""));
    }

    #[test]
    fn rejects_broken_files() {
        assert!(parse("\"a\"\n{\n\t\"b\"\t\"c\"\n").is_err());
        assert!(parse("\"a\" \"unterminated").is_err());
        assert!(parse("\"a\"").is_err());
        assert!(parse("}").is_err());
    }

    #[test]
    fn set_and_remove_are_case_insensitive() {
        let mut o = parse("\"mostrecent\" \"0\"").unwrap();
        o.set_str("MostRecent", "1");
        o.set_str("AllowAutoLogin", "1");
        assert_eq!(o.entries[0], ("mostrecent".into(), Value::Str("1".into())));
        assert_eq!(o.get_str("allowautologin"), Some("1"));
        assert!(o.remove("MOSTRECENT"));
        assert!(!o.remove("missing"));
    }
}
