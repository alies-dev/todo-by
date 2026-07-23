//! Hand-rolled parser for a small TOML subset used by `todo-by.toml` /
//! `.todo-by.toml` config files.
//!
//! Supported grammar:
//! - UTF-8 text, line-oriented. Blank lines and full-line or trailing `#`
//!   comments are allowed (a `#` inside a quoted string is content, not a
//!   comment).
//! - Entries look like `key = value` with bare keys matching `[A-Za-z0-9_-]+`.
//! - Values are one of:
//!   - an unsigned integer (must fit in `u32` for `warn`),
//!   - a basic double-quoted string with escapes `\"`, `\\`, `\n`, `\t`
//!     (any other escape is an error),
//!   - an array of basic strings. Arrays may span multiple lines until the
//!     closing `]`; a trailing comma is allowed, and comments are allowed
//!     inside a multi-line array.
//! - Anything else is an error: table/section headers (a line starting with
//!   `[`), duplicate keys, literal single-quoted strings, and unknown keys.
//!
//! Recognized keys: `warn` (integer), `exclude` (string array), `tags`
//! (string array), `version-cmd` (string). Errors are formatted as
//! `label:LINE: message` with 1-based line numbers.

use std::path::{Path, PathBuf};

/// The two config filenames searched by [`load`], in per-directory priority
/// order: `todo-by.toml` wins over `.todo-by.toml` in the same directory.
const CONFIG_FILENAMES: [&str; 2] = ["todo-by.toml", ".todo-by.toml"];

const VALID_KEYS: &str = "warn, exclude, tags, version-cmd";

#[derive(Debug)]
pub struct Config {
    /// Days before a deadline at which tags start reporting as warnings.
    pub warn: Option<u32>,
    /// Shell command (run via `sh -c`) whose trimmed stdout is the current
    /// version, for resolving version-constraint triggers.
    pub version_cmd: Option<String>,
    /// gitignore-style globs excluded on top of .gitignore.
    pub exclude: Vec<String>,
    /// Tags to match. Replaces the default entirely when set in the file.
    pub tags: Vec<String>,
    /// Config file the values came from; None means built-in defaults.
    pub source: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            warn: None,
            version_cmd: None,
            exclude: Vec::new(),
            tags: vec!["todo-by".to_string()],
            source: None,
        }
    }
}

/// Searches `start` and its ancestors for `todo-by.toml`, then
/// `.todo-by.toml` (per directory, `todo-by.toml` wins). The first hit is
/// parsed; no hit returns defaults. Errors on an unreadable or invalid file.
pub fn load(start: &Path) -> Result<Config, String> {
    for dir in start.ancestors() {
        for filename in CONFIG_FILENAMES {
            let candidate = dir.join(filename);
            if !candidate.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&candidate)
                .map_err(|err| format!("{}: {err}", candidate.display()))?;
            let label = candidate.display().to_string();
            let mut cfg = parse(&text, &label)?;
            cfg.source = Some(candidate);
            return Ok(cfg);
        }
    }
    Ok(Config::default())
}

/// A single parsed scalar or array value, before it is assigned to a
/// recognized field.
enum Value {
    Int(u32),
    /// A basic double-quoted string; currently only `version-cmd` accepts
    /// this shape.
    Str(String),
    Array(Vec<String>),
}

/// Parses config text. `label` prefixes error messages (e.g. "todo-by.toml").
pub fn parse(text: &str, label: &str) -> Result<Config, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut warn: Option<u32> = None;
    let mut version_cmd: Option<String> = None;
    let mut exclude: Option<Vec<String>> = None;
    let mut tags: Option<Vec<String>> = None;
    let mut seen_keys: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let line_no = i + 1;
        let content = strip_comment(lines[i]).trim();
        if content.is_empty() {
            i += 1;
            continue;
        }
        if content.starts_with('[') {
            return Err(format!("{label}:{line_no}: tables are not supported"));
        }

        let eq_idx = content
            .find('=')
            .ok_or_else(|| format!("{label}:{line_no}: expected 'key = value'"))?;
        let key = content[..eq_idx].trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(format!(
                "{label}:{line_no}: invalid key {key:?} (bare keys must match [A-Za-z0-9_-]+)"
            ));
        }
        if seen_keys.contains(&key) {
            return Err(format!("{label}:{line_no}: duplicate key {key:?}"));
        }

        let value_part = content[eq_idx + 1..].trim_start();
        if value_part.is_empty() {
            return Err(format!("{label}:{line_no}: missing value for {key:?}"));
        }

        let (value, next_i) = if let Some(after_bracket) = value_part.strip_prefix('[') {
            let (items, end_idx) = parse_array(&lines, i, after_bracket, label)?;
            (Value::Array(items), end_idx + 1)
        } else {
            (parse_scalar(value_part, label, line_no)?, i + 1)
        };

        match key {
            "warn" => {
                let Value::Int(n) = value else {
                    return Err(format!("{label}:{line_no}: warn must be an integer"));
                };
                warn = Some(n);
            }
            "version-cmd" => {
                let Value::Str(s) = value else {
                    return Err(format!("{label}:{line_no}: version-cmd must be a string"));
                };
                if s.is_empty() {
                    return Err(format!("{label}:{line_no}: version-cmd must not be empty"));
                }
                version_cmd = Some(s);
            }
            "exclude" => {
                let Value::Array(items) = value else {
                    return Err(format!(
                        "{label}:{line_no}: exclude must be an array of strings"
                    ));
                };
                for item in &items {
                    if item.is_empty() {
                        return Err(format!(
                            "{label}:{line_no}: exclude entries must not be empty"
                        ));
                    }
                }
                exclude = Some(items);
            }
            "tags" => {
                let Value::Array(items) = value else {
                    return Err(format!(
                        "{label}:{line_no}: tags must be an array of strings"
                    ));
                };
                if items.is_empty() {
                    return Err(format!("{label}:{line_no}: tags must not be empty"));
                }
                for tag in &items {
                    if !is_valid_tag(tag) {
                        return Err(format!(
                            "{label}:{line_no}: invalid tag {tag:?} (must be non-empty ASCII \
                             without whitespace, ':', or '#')"
                        ));
                    }
                }
                tags = Some(items);
            }
            other => {
                return Err(format!(
                    "{label}:{line_no}: unknown key {other:?} (valid keys: {VALID_KEYS})"
                ));
            }
        }

        seen_keys.push(key);
        i = next_i;
    }

    Ok(Config {
        warn,
        version_cmd,
        exclude: exclude.unwrap_or_default(),
        tags: tags.unwrap_or_else(|| vec!["todo-by".to_string()]),
        source: None,
    })
}

fn is_valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.is_ascii()
        && !tag.contains(char::is_whitespace)
        && !tag.contains(':')
        && !tag.contains('#')
}

/// Returns the prefix of `s` before the first unquoted `#`, i.e. strips a
/// trailing comment while treating `#` inside a double-quoted string (even a
/// malformed one) as content, not a comment marker.
fn strip_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if in_string => i += 2,
            b'"' => {
                in_string = !in_string;
                i += 1;
            }
            b'#' if !in_string => return &s[..i],
            _ => i += 1,
        }
    }
    s
}

/// Parses a non-array scalar: an unsigned integer or a double-quoted string,
/// followed only by optional whitespace and/or a comment.
fn parse_scalar(value_part: &str, label: &str, line_no: usize) -> Result<Value, String> {
    let bytes = value_part.as_bytes();
    if bytes[0] == b'"' {
        let (s, end) = parse_quoted_string(value_part, 0, label, line_no)?;
        let after = value_part[end..].trim_start();
        if !after.is_empty() && !after.starts_with('#') {
            return Err(format!(
                "{label}:{line_no}: unexpected trailing content after string"
            ));
        }
        return Ok(Value::Str(s));
    }
    if bytes[0] == b'\'' {
        return Err(format!(
            "{label}:{line_no}: single-quoted strings are not supported, use double quotes"
        ));
    }
    if bytes[0].is_ascii_digit() {
        let end = value_part
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(value_part.len());
        let digits = &value_part[..end];
        let after = value_part[end..].trim_start();
        if !after.is_empty() && !after.starts_with('#') {
            return Err(format!(
                "{label}:{line_no}: unexpected trailing content after integer"
            ));
        }
        let n: u32 = digits
            .parse()
            .map_err(|_| format!("{label}:{line_no}: integer {digits} out of range"))?;
        return Ok(Value::Int(n));
    }
    Err(format!(
        "{label}:{line_no}: expected a string, integer, or array"
    ))
}

/// Parses a double-quoted string starting at byte offset `start` (which must
/// point at the opening `"`). Returns the decoded string and the byte offset
/// just past the closing `"`.
fn parse_quoted_string(
    s: &str,
    start: usize,
    label: &str,
    line_no: usize,
) -> Result<(String, usize), String> {
    let bytes = s.as_bytes();
    let mut i = start + 1;
    let mut out = String::new();
    loop {
        match bytes.get(i) {
            None => return Err(format!("{label}:{line_no}: unterminated string")),
            Some(b'"') => return Ok((out, i + 1)),
            Some(b'\\') => {
                let esc = bytes
                    .get(i + 1)
                    .ok_or_else(|| format!("{label}:{line_no}: unterminated string"))?;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    other => {
                        return Err(format!(
                            "{label}:{line_no}: unsupported escape '\\{}'",
                            *other as char
                        ))
                    }
                }
                i += 2;
            }
            Some(_) => {
                let ch = s[i..].chars().next().expect("byte index within bounds");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
}

/// Parses the contents of a `[...]` array of strings, which may span
/// multiple lines. `start_idx` is the 0-based line index of the opening
/// `[`, and `after_bracket` is the remainder of that line following it.
/// Returns the parsed items and the 0-based line index where `]` was found.
fn parse_array(
    lines: &[&str],
    start_idx: usize,
    after_bracket: &str,
    label: &str,
) -> Result<(Vec<String>, usize), String> {
    let mut items = Vec::new();
    let mut idx = start_idx;
    let mut rest = after_bracket;
    let mut expect_comma = false;
    loop {
        let bytes = rest.as_bytes();
        let mut i = 0;
        loop {
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'#' => break,
                b']' => {
                    let after = rest[i + 1..].trim_start();
                    if !after.is_empty() && !after.starts_with('#') {
                        return Err(format!(
                            "{label}:{}: unexpected trailing content after ']'",
                            idx + 1
                        ));
                    }
                    return Ok((items, idx));
                }
                b',' => {
                    if !expect_comma {
                        return Err(format!("{label}:{}: unexpected ',' in array", idx + 1));
                    }
                    expect_comma = false;
                    i += 1;
                }
                b'"' => {
                    if expect_comma {
                        return Err(format!("{label}:{}: expected ',' or ']'", idx + 1));
                    }
                    let (s, new_i) = parse_quoted_string(rest, i, label, idx + 1)?;
                    items.push(s);
                    expect_comma = true;
                    i = new_i;
                }
                b'\'' => {
                    return Err(format!(
                        "{label}:{}: single-quoted strings are not supported, use double quotes",
                        idx + 1
                    ));
                }
                _ => {
                    return Err(format!(
                        "{label}:{}: expected a string, ',' or ']' in array",
                        idx + 1
                    ));
                }
            }
        }
        idx += 1;
        if idx >= lines.len() {
            return Err(format!("{label}:{}: unterminated array, missing ']'", idx));
        }
        rest = lines[idx];
    }
}

/// Renders the effective config as TOML, with a leading `# source: <path>`
/// or `# source: built-in defaults` comment.
pub fn dump(cfg: &Config) -> String {
    let mut out = String::new();
    match &cfg.source {
        Some(path) => out.push_str(&format!("# source: {}\n", path.display())),
        None => out.push_str("# source: built-in defaults\n"),
    }
    match cfg.warn {
        Some(n) => out.push_str(&format!("warn = {n}\n")),
        None => out.push_str("# warn = (not set)\n"),
    }
    match &cfg.version_cmd {
        Some(cmd) => out.push_str(&format!("version-cmd = \"{}\"\n", escape_str(cmd))),
        None => out.push_str("# version-cmd = (not set)\n"),
    }
    out.push_str(&format!("exclude = {}\n", dump_array(&cfg.exclude)));
    out.push_str(&format!("tags = {}\n", dump_array(&cfg.tags)));
    out
}

fn dump_array(items: &[String]) -> String {
    let rendered: Vec<String> = items
        .iter()
        .map(|s| format!("\"{}\"", escape_str(s)))
        .collect();
    format!("[{}]", rendered.join(", "))
}

fn escape_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn empty_text_yields_defaults() {
        let cfg = parse("", "test").unwrap();
        assert_eq!(cfg.warn, None);
        assert!(cfg.exclude.is_empty());
        assert_eq!(cfg.tags, vec!["todo-by".to_string()]);
        assert_eq!(cfg.source, None);
    }

    #[test]
    fn parses_all_three_keys_with_multiline_array() {
        let text = "\
warn = 7
exclude = [\"vendor/**\", \"*.gen.go\"]
tags = [
    \"todo-by\",
    \"fixme-by\",
]
";
        let cfg = parse(text, "test").unwrap();
        assert_eq!(cfg.warn, Some(7));
        assert_eq!(
            cfg.exclude,
            vec!["vendor/**".to_string(), "*.gen.go".to_string()]
        );
        assert_eq!(
            cfg.tags,
            vec!["todo-by".to_string(), "fixme-by".to_string()]
        );
    }

    #[test]
    fn trailing_comma_is_allowed() {
        let cfg = parse("tags = [\"todo-by\",]\n", "test").unwrap();
        assert_eq!(cfg.tags, vec!["todo-by".to_string()]);
    }

    #[test]
    fn trailing_content_after_array_close_is_rejected() {
        let err = parse("tags = [\"todo-by\"] garbage\n", "test").unwrap_err();
        assert!(err.contains("trailing content"), "{err}");
        // A trailing comment after ']' stays fine.
        assert!(parse("tags = [\"todo-by\"] # ok\n", "test").is_ok());
    }

    #[test]
    fn comments_and_hash_inside_strings_are_handled() {
        let text = "\
# a full-line comment
tags = [\"todo-by\"] # trailing comment
exclude = [\"weird#name\"] # another comment
";
        let cfg = parse(text, "test").unwrap();
        assert_eq!(cfg.tags, vec!["todo-by".to_string()]);
        assert_eq!(cfg.exclude, vec!["weird#name".to_string()]);
    }

    #[test]
    fn multiline_array_allows_comments_between_items() {
        let text = "\
tags = [
    \"todo-by\", # keep
    # a full-line comment
    \"fixme-by\",
]
";
        let cfg = parse(text, "test").unwrap();
        assert_eq!(
            cfg.tags,
            vec!["todo-by".to_string(), "fixme-by".to_string()]
        );
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let err = parse("warn = 1\nwarn = 2\n", "cfg").unwrap_err();
        assert_eq!(err, "cfg:2: duplicate key \"warn\"");
    }

    #[test]
    fn unknown_key_error_lists_valid_keys() {
        let err = parse("nope = 1\n", "cfg").unwrap_err();
        assert!(err.contains("unknown key"), "{err}");
        assert!(err.contains("warn"), "{err}");
        assert!(err.contains("exclude"), "{err}");
        assert!(err.contains("tags"), "{err}");
        assert!(err.contains("version-cmd"), "{err}");
    }

    #[test]
    fn version_cmd_parses_as_string() {
        let cfg = parse("version-cmd = \"git describe --tags\"\n", "cfg").unwrap();
        assert_eq!(cfg.version_cmd, Some("git describe --tags".to_string()));
    }

    #[test]
    fn empty_version_cmd_is_rejected() {
        let err = parse("version-cmd = \"\"\n", "cfg").unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn non_string_version_cmd_is_rejected() {
        let err = parse("version-cmd = 5\n", "cfg").unwrap_err();
        assert!(err.contains("must be a string"), "{err}");
    }

    #[test]
    fn table_header_is_rejected() {
        let err = parse("[section]\n", "cfg").unwrap_err();
        assert_eq!(err, "cfg:1: tables are not supported");
    }

    #[test]
    fn bad_escape_is_rejected() {
        let err = parse("exclude = [\"\\q\"]\n", "cfg").unwrap_err();
        assert_eq!(err, "cfg:1: unsupported escape '\\q'");
    }

    #[test]
    fn warn_overflow_is_rejected() {
        let err = parse("warn = 999999999999\n", "cfg").unwrap_err();
        assert!(err.contains("out of range"), "{err}");
    }

    #[test]
    fn tags_with_whitespace_are_rejected() {
        let err = parse("tags = [\"has space\"]\n", "cfg").unwrap_err();
        assert!(err.contains("invalid tag"), "{err}");
    }

    #[test]
    fn tags_with_colon_or_hash_are_rejected() {
        assert!(parse("tags = [\"a:b\"]\n", "cfg").is_err());
        assert!(parse("tags = [\"a#b\"]\n", "cfg").is_err());
    }

    #[test]
    fn empty_tags_array_is_rejected() {
        let err = parse("tags = []\n", "cfg").unwrap_err();
        assert_eq!(err, "cfg:1: tags must not be empty");
    }

    #[test]
    fn empty_exclude_entry_is_rejected() {
        let err = parse("exclude = [\"\"]\n", "cfg").unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn single_quoted_strings_are_rejected() {
        assert!(parse("tags = ['todo-by']\n", "cfg").is_err());
    }

    #[test]
    fn dump_round_trip_contains_values_and_source() {
        let text = "\
warn = 3
exclude = [\"vendor/**\"]
tags = [\"todo-by\"]
";
        let mut cfg = parse(text, "cfg").unwrap();
        cfg.source = Some(PathBuf::from("/tmp/todo-by.toml"));
        let dumped = dump(&cfg);
        assert!(dumped.contains("# source: /tmp/todo-by.toml"), "{dumped}");
        assert!(dumped.contains("warn = 3"), "{dumped}");
        assert!(dumped.contains("vendor/**"), "{dumped}");
        assert!(dumped.contains("todo-by"), "{dumped}");
    }

    #[test]
    fn dump_renders_unset_warn() {
        let cfg = Config::default();
        let dumped = dump(&cfg);
        assert!(dumped.contains("# source: built-in defaults"), "{dumped}");
        assert!(dumped.contains("# warn = (not set)"), "{dumped}");
        assert!(dumped.contains("# version-cmd = (not set)"), "{dumped}");
    }

    #[test]
    fn dump_includes_set_version_cmd() {
        let cfg = Config {
            version_cmd: Some("jq -r .version package.json".to_string()),
            ..Config::default()
        };
        let dumped = dump(&cfg);
        assert!(
            dumped.contains("version-cmd = \"jq -r .version package.json\""),
            "{dumped}"
        );
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("todo-by-config-test-{nanos}-{n}-{tag}"));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    #[test]
    fn load_finds_config_in_ancestor_directory() {
        let root = unique_temp_dir("ancestor");
        let child = root.join("nested/deeper");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.join("todo-by.toml"), "tags = [\"fixme-by\"]\n").unwrap();

        let cfg = load(&child).unwrap();
        assert_eq!(cfg.tags, vec!["fixme-by".to_string()]);
        assert_eq!(cfg.source, Some(root.join("todo-by.toml")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_prefers_todo_by_toml_over_dotfile() {
        let root = unique_temp_dir("both-names");
        std::fs::write(root.join("todo-by.toml"), "tags = [\"a\"]\n").unwrap();
        std::fs::write(root.join(".todo-by.toml"), "tags = [\"b\"]\n").unwrap();

        let cfg = load(&root).unwrap();
        assert_eq!(cfg.tags, vec!["a".to_string()]);
        assert_eq!(cfg.source, Some(root.join("todo-by.toml")));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_with_no_file_yields_defaults() {
        let root = unique_temp_dir("no-file");

        let cfg = load(&root).unwrap();
        assert_eq!(cfg.tags, vec!["todo-by".to_string()]);
        assert_eq!(cfg.warn, None);
        assert!(cfg.exclude.is_empty());
        assert_eq!(cfg.source, None);

        std::fs::remove_dir_all(&root).ok();
    }
}
