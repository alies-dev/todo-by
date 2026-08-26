//! Minimal JSON reader, sized for exactly one job: reading a GitHub
//! GraphQL response. Adding a JSON crate for this would be the first
//! dependency past `ignore`, on a grammar that has not changed since 2013,
//! so the reader is hand-rolled like the TOML subset in `config.rs`.
//!
//! Numbers are parsed but never inspected (nothing this tool reads from a
//! response is numeric); they exist so a response carrying one still
//! parses. Objects keep insertion order in a `Vec` rather than a map:
//! GraphQL responses are small and flat enough that a linear `get` is
//! faster than hashing, and duplicate keys (which GraphQL never emits)
//! resolve to the first occurrence rather than silently to the last.

/// Nesting cap. A GitHub response is 5 levels deep; the limit only exists
/// so a hostile or corrupted body can't recurse the parser into a stack
/// overflow, which would abort the process instead of reporting an error.
const MAX_DEPTH: usize = 32;

#[derive(Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Field lookup on an object; None for any other variant.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

/// Parses a complete JSON document. None on any syntax error or trailing
/// content; the caller has no use for a byte offset, since a malformed
/// response is reported as "unreadable response" either way.
pub fn parse(text: &str) -> Option<Json> {
    let mut p = Parser {
        text,
        bytes: text.as_bytes(),
        i: 0,
    };
    let value = p.value(0)?;
    p.space();
    if p.i == p.bytes.len() {
        Some(value)
    } else {
        None
    }
}

struct Parser<'a> {
    /// The same input as `bytes`, kept so a multi-byte character can be
    /// decoded without re-validating the rest of the document. Reading it
    /// through `from_utf8` on every such character made a response with a
    /// long non-ASCII string quadratic.
    text: &'a str,
    bytes: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.i).copied()
    }

    fn literal(&mut self, word: &str) -> bool {
        if self.bytes[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.space();
        match self.peek()? {
            b'n' => self.literal("null").then_some(Json::Null),
            b't' => self.literal("true").then_some(Json::Bool(true)),
            b'f' => self.literal("false").then_some(Json::Bool(false)),
            b'"' => self.string().map(Json::Str),
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            _ => self.number(),
        }
    }

    fn array(&mut self, depth: usize) -> Option<Json> {
        self.i += 1; // '['
        let mut items = Vec::new();
        self.space();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Some(Json::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.space();
            match self.peek()? {
                b',' => self.i += 1,
                b']' => {
                    self.i += 1;
                    return Some(Json::Arr(items));
                }
                _ => return None,
            }
        }
    }

    fn object(&mut self, depth: usize) -> Option<Json> {
        self.i += 1; // '{'
        let mut pairs = Vec::new();
        self.space();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Some(Json::Obj(pairs));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            if self.peek()? != b':' {
                return None;
            }
            self.i += 1;
            pairs.push((key, self.value(depth + 1)?));
            self.space();
            match self.peek()? {
                b',' => self.i += 1,
                b'}' => {
                    self.i += 1;
                    return Some(Json::Obj(pairs));
                }
                _ => return None,
            }
        }
    }

    /// Numbers are consumed by charset rather than by grammar: the value is
    /// never read, so the only requirement is landing on the byte after the
    /// number. `f64::from_str` still rejects the malformed shapes that
    /// charset alone would admit (`1.2.3`, `-`, `1e`).
    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E'))
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.i])
            .ok()?
            .parse()
            .ok()
            .map(Json::Num)
    }

    /// Decodes a double-quoted string, including `\uXXXX` and surrogate
    /// pairs. Full decoding is needed because `errors[].message` is shown
    /// to a human; skipping escapes without decoding them would be enough
    /// for the parser itself but would print `’` at someone.
    fn string(&mut self) -> Option<String> {
        self.i += 1; // '"'
        let mut out = String::new();
        loop {
            match self.peek()? {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.i += 1;
                    let esc = self.peek()?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return None,
                    }
                }
                // Control bytes are illegal unescaped in JSON, but this
                // reader accepts them: the payload is a server response,
                // not user input to validate, and rejecting the whole
                // document over a stray byte would turn a readable answer
                // into "unreadable response".
                _ => {
                    let ch = self.text.get(self.i..)?.chars().next()?;
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }

    /// Reads the four hex digits after `\u`, joining a leading surrogate
    /// with the trailing `\uXXXX` that must follow it. An unpaired
    /// surrogate becomes U+FFFD rather than failing the parse, matching
    /// how the scanner treats invalid UTF-8 in a scanned file.
    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex4()?;
        if !(0xD800..0xDC00).contains(&first) {
            return Some(char::from_u32(first).unwrap_or('\u{FFFD}'));
        }
        if self.peek() == Some(b'\\') && self.bytes.get(self.i + 1) == Some(&b'u') {
            // Rewind if the escape that follows isn't the low half: it is a
            // character in its own right, and consuming it would swallow the
            // "A" out of "\ud800\u0041".
            let before = self.i;
            self.i += 2;
            match self.hex4() {
                Some(second) if (0xDC00..0xE000).contains(&second) => {
                    let combined = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                    return Some(char::from_u32(combined).unwrap_or('\u{FFFD}'));
                }
                _ => self.i = before,
            }
        }
        Some('\u{FFFD}')
    }

    fn hex4(&mut self) -> Option<u32> {
        let end = self.i + 4;
        let digits = std::str::from_utf8(self.bytes.get(self.i..end)?).ok()?;
        let value = u32::from_str_radix(digits, 16).ok()?;
        self.i = end;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_graphql_shaped_response() {
        let text = r#"{"data":{"r0":{"n1":{"state":"CLOSED"},"n2":null}},
                       "errors":[{"type":"NOT_FOUND","path":["r0","n2"],"message":"nope"}]}"#;
        let json = parse(text).expect("valid");
        let state = json
            .get("data")
            .and_then(|d| d.get("r0"))
            .and_then(|r| r.get("n1"))
            .and_then(|n| n.get("state"))
            .and_then(Json::as_str);
        assert_eq!(state, Some("CLOSED"));
        let errors = json.get("errors").and_then(Json::as_arr).expect("array");
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].get("message").and_then(Json::as_str),
            Some("nope")
        );
        let path: Vec<&str> = errors[0]
            .get("path")
            .and_then(Json::as_arr)
            .expect("array")
            .iter()
            .filter_map(Json::as_str)
            .collect();
        assert_eq!(path, ["r0", "n2"]);
    }

    #[test]
    fn scalars_and_containers() {
        assert_eq!(parse("null"), Some(Json::Null));
        assert_eq!(parse(" true "), Some(Json::Bool(true)));
        assert_eq!(parse("false"), Some(Json::Bool(false)));
        assert_eq!(parse("-1.5e3"), Some(Json::Num(-1500.0)));
        assert_eq!(parse("[]"), Some(Json::Arr(Vec::new())));
        assert_eq!(parse("{}"), Some(Json::Obj(Vec::new())));
        assert_eq!(parse(r#""x""#), Some(Json::Str("x".to_string())));
    }

    #[test]
    fn decodes_escapes_and_surrogate_pairs() {
        assert_eq!(
            parse(r#""a\"b\\c\/d\n\tAé""#),
            Some(Json::Str("a\"b\\c/d\n\tAé".to_string()))
        );
        // U+1F600, as a surrogate pair.
        assert_eq!(parse(r#""😀""#), Some(Json::Str("😀".to_string())));
        // Unpaired leading surrogate degrades instead of failing, and the
        // escape after it survives rather than being swallowed.
        assert_eq!(
            parse(r#""\ud83d!""#),
            Some(Json::Str("\u{FFFD}!".to_string()))
        );
        assert_eq!(
            parse(r#""\ud800\u0041""#),
            Some(Json::Str("\u{FFFD}A".to_string()))
        );
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "",
            "{",
            "[1,]",
            "{\"a\"}",
            "{\"a\":1,}",
            "nul",
            "\"unterminated",
            "{} trailing",
            "1.2.3",
            r#"{"a":1}{"b":2}"#,
        ] {
            assert_eq!(parse(bad), None, "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn rejects_nesting_past_the_depth_cap() {
        let deep = format!("{}1{}", "[".repeat(64), "]".repeat(64));
        assert_eq!(parse(&deep), None);
        let shallow = format!("{}1{}", "[".repeat(8), "]".repeat(8));
        assert!(parse(&shallow).is_some());
    }

    #[test]
    fn get_and_accessors_are_typed() {
        let json = parse(r#"{"a":"x","b":[1],"a":"dup"}"#).expect("valid");
        // First occurrence wins, so a duplicate key can't silently override.
        assert_eq!(json.get("a").and_then(Json::as_str), Some("x"));
        assert_eq!(json.get("missing"), None);
        assert_eq!(json.get("b").and_then(Json::as_str), None);
        assert_eq!(json.as_arr(), None);
    }
}
