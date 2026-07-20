//! Semantic-version-shaped parsing and comparison for version-constraint
//! triggers, e.g. a tag written `>=2.0` firing once the project reaches 2.0.

use std::cmp::Ordering;

/// A parsed version: a numeric core (1 to 3 dot-separated components) plus
/// an optional pre-release suffix. Build metadata (`+...`) is accepted by
/// [`Version::parse`] but discarded immediately: per semver it never affects
/// ordering, so keeping it around would just be dead weight.
#[derive(Clone, Eq, Debug)]
pub struct Version {
    parts: Vec<u64>,
    pre: Option<String>,
}

impl Version {
    /// Parses `[v|V]<core>[-<pre>][+<build>]`, where `<core>` is 1 to 3
    /// dot-separated ASCII-digit components each fitting a `u64`.
    ///
    /// Rejects rather than degrades on anything ambiguous: an empty
    /// component (`2..0`, `.2.0`), an empty pre-release or build after the
    /// separator (`2.0-`, `2.0+`), or an empty dot-separated identifier
    /// inside either (`2.0-alpha..1`, `2.0+build..1`, `2.0-alpha.`, all
    /// invalid per semver) return `None` instead of silently parsing as a
    /// shorter, different version. That mirrors
    /// `date::deadline`'s stance on malformed tokens: a typo should surface
    /// as an invalid trigger, not quietly mean something else.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let core_and_pre = match s.split_once('+') {
            Some((head, build)) => {
                if build.split('.').any(str::is_empty) {
                    return None;
                }
                head
            }
            None => s,
        };
        let (core, pre) = match core_and_pre.split_once('-') {
            Some((core, pre)) => {
                if pre.split('.').any(str::is_empty) {
                    return None;
                }
                (core, Some(pre.to_string()))
            }
            None => (core_and_pre, None),
        };
        if core.is_empty() {
            return None;
        }
        let raw_parts: Vec<&str> = core.split('.').collect();
        if raw_parts.len() > 3 {
            return None;
        }
        let mut parts = Vec::with_capacity(raw_parts.len());
        for p in raw_parts {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            parts.push(p.parse::<u64>().ok()?);
        }
        Some(Self { parts, pre })
    }

    /// The numeric core component at `i`, or 0 past the end: lets two cores
    /// of different lengths compare as if zero-padded to the same length
    /// (`2.0` and `2.0.0` must compare equal).
    fn core_at(&self, i: usize) -> u64 {
        self.parts.get(i).copied().unwrap_or(0)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.parts.len().max(other.parts.len());
        for i in 0..len {
            match self.core_at(i).cmp(&other.core_at(i)) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
        // Same core: a release outranks a pre-release of that core
        // (2.0.0 > 2.0.0-rc.1); two pre-releases fall back to semver's
        // dot-identifier precedence rule.
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => compare_pre(a, b),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

/// Semver precedence rule 11: compare pre-release identifiers dot by dot;
/// numeric identifiers compare numerically and always rank below
/// alphanumeric ones; a pre-release with more identifiers outranks a
/// otherwise-equal prefix with fewer (`1.0.0-alpha.1` > `1.0.0-alpha`).
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut a_ids = a.split('.');
    let mut b_ids = b.split('.');
    loop {
        match (a_ids.next(), b_ids.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match compare_pre_identifier(x, y) {
                Ordering::Equal => {}
                ord => return ord,
            },
        }
    }
}

fn compare_pre_identifier(a: &str, b: &str) -> Ordering {
    let a_num = is_numeric_identifier(a)
        .then(|| a.parse::<u64>().ok())
        .flatten();
    let b_num = is_numeric_identifier(b)
        .then(|| b.parse::<u64>().ok())
        .flatten();
    match (a_num, b_num) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

fn is_numeric_identifier(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `>=` or `>`: the only comparators this tool acts on. `Constraint::parse`
/// rejects everything else (see [`unsupported_comparator`]) rather than
/// guessing at a meaning for them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    Ge,
    Gt,
}

#[derive(Clone, Debug)]
pub struct Constraint {
    pub cmp: Cmp,
    pub version: Version,
}

impl Constraint {
    /// Parses a comparator-prefixed constraint such as `>=2.0` or
    /// `>1.4.0-rc.1`. Only `>=` and `>` are recognized here; `<`, `<=`, `=`,
    /// and `==` are syntactically version-like but return `None` because
    /// this tool has no "before version X" semantics to give them (see
    /// [`unsupported_comparator`] for surfacing that distinctly from a
    /// plain parse failure).
    pub fn parse(written: &str) -> Option<Constraint> {
        let (cmp, rest) = if let Some(rest) = written.strip_prefix(">=") {
            (Cmp::Ge, rest)
        } else {
            (Cmp::Gt, written.strip_prefix('>')?)
        };
        let version = Version::parse(rest)?;
        Some(Constraint { cmp, version })
    }

    pub fn satisfied_by(&self, current: &Version) -> bool {
        match self.cmp {
            Cmp::Ge => current >= &self.version,
            Cmp::Gt => current > &self.version,
        }
    }
}

/// Comparators phpstan-todo-by users bring over meaning "before version X"
/// (`<1.0`, `<=1.0`, `=1.0`, `==1.0`). Silently treating them as unparsable
/// would be worse than useless: unlike a plain typo, these read as valid
/// intent that would otherwise never fire, postponing the chore forever.
/// Ordered longest-prefix-first so `==`/`<=` are found before `=`/`<`.
const UNSUPPORTED_COMPARATORS: [&str; 4] = ["==", "<=", "<", "="];

/// When `written` (the full `comparator + version` token as scanned) starts
/// with a comparator this tool intentionally rejects, returns that
/// comparator so callers can build a message explaining why, distinct from
/// a generic "invalid version constraint".
pub fn unsupported_comparator(written: &str) -> Option<&str> {
    UNSUPPORTED_COMPARATORS
        .iter()
        .find(|c| written.starts_with(**c))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_valid_cores() {
        assert!(Version::parse("2").is_some());
        assert!(Version::parse("2.0").is_some());
        assert!(Version::parse("2.0.5").is_some());
        assert!(Version::parse("v2.0.5").is_some());
        assert!(Version::parse("V2.0.5").is_some());
    }

    #[test]
    fn rejects_malformed_cores() {
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("2.0.5.1"), None, "4 components");
        assert_eq!(Version::parse("2.x"), None, "non-digit component");
        assert_eq!(Version::parse("2..0"), None, "empty component");
        assert_eq!(Version::parse(".2.0"), None, "leading empty component");
        assert_eq!(Version::parse("2.0."), None, "trailing empty component");
        assert_eq!(Version::parse("garbage"), None);
    }

    #[test]
    fn rejects_empty_pre_and_build() {
        assert_eq!(Version::parse("2.0-"), None);
        assert_eq!(Version::parse("2.0+"), None);
        assert_eq!(Version::parse("2.0-rc.1+"), None);
    }

    #[test]
    fn rejects_empty_identifiers_inside_pre_and_build() {
        assert_eq!(Version::parse("1.0.0-alpha..1"), None);
        assert_eq!(Version::parse("1.0.0-alpha."), None);
        assert_eq!(Version::parse("1.0.0-.alpha"), None);
        assert_eq!(Version::parse("1.0.0+build..1"), None);
        assert_eq!(Version::parse("1.0.0+build."), None);
        assert!(Version::parse("1.0.0-alpha.1+build.1").is_some());
    }

    #[test]
    fn build_metadata_is_parsed_and_ignored_for_ordering() {
        assert_eq!(v("2.0.0+build.5"), v("2.0.0"));
        assert!(Version::parse("2.0.0-rc.1+build.5").is_some());
    }

    #[test]
    fn u64_bounds() {
        assert!(Version::parse("18446744073709551615").is_some()); // u64::MAX
        assert_eq!(Version::parse("18446744073709551616"), None); // overflow
    }

    #[test]
    fn zero_padding_makes_shorter_cores_equal_to_longer_ones() {
        assert_eq!(v("2.0"), v("2.0.0"));
        assert_eq!(v("2"), v("2.0.0"));
        assert!(v("2.0") < v("2.0.1"));
        assert!(v("2.1") > v("2.0.9"));
    }

    #[test]
    fn release_outranks_prerelease_of_the_same_core() {
        assert!(v("2.0.0-rc.1") < v("2.0.0"));
        assert!(v("2.0.0") > v("2.0.0-rc.1"));
    }

    #[test]
    fn prerelease_precedence_follows_semver() {
        // numeric identifiers compare numerically and rank below alphanumeric ones
        assert!(v("1.0.0-alpha.1") < v("1.0.0-alpha.beta"));
        // more fields outranks an otherwise-equal shorter prefix
        assert!(v("1.0.0-alpha") < v("1.0.0-alpha.1"));
        // alphanumeric identifiers compare lexically
        assert!(v("1.0.0-alpha") < v("1.0.0-beta"));
        // numeric identifiers compare by value, not lexically ("10" > "9")
        assert!(v("1.0.0-alpha.9") < v("1.0.0-alpha.10"));
    }

    #[test]
    fn constraint_parses_ge_and_gt_only() {
        let c = Constraint::parse(">=2.0").unwrap();
        assert_eq!(c.cmp, Cmp::Ge);
        assert_eq!(c.version, v("2.0"));

        let c = Constraint::parse(">1.4.0-rc.1").unwrap();
        assert_eq!(c.cmp, Cmp::Gt);
        assert_eq!(c.version, v("1.4.0-rc.1"));

        assert!(Constraint::parse("<1.0").is_none());
        assert!(Constraint::parse("<=1.0").is_none());
        assert!(Constraint::parse("=1.0").is_none());
        assert!(Constraint::parse("==1.0").is_none());
    }

    #[test]
    fn constraint_parse_rejects_malformed_version_after_valid_comparator() {
        assert!(Constraint::parse(">=2.x").is_none());
        assert!(Constraint::parse(">=").is_none());
    }

    #[test]
    fn satisfied_by_uses_the_right_comparator() {
        let ge = Constraint::parse(">=2.0").unwrap();
        assert!(ge.satisfied_by(&v("2.0")));
        assert!(ge.satisfied_by(&v("2.0.5")));
        assert!(!ge.satisfied_by(&v("1.9.9")));

        let gt = Constraint::parse(">2.0").unwrap();
        assert!(!gt.satisfied_by(&v("2.0")));
        assert!(gt.satisfied_by(&v("2.0.1")));
    }

    #[test]
    fn unsupported_comparator_identifies_rejected_prefixes() {
        assert_eq!(unsupported_comparator("<1.0"), Some("<"));
        assert_eq!(unsupported_comparator("<=1.0"), Some("<="));
        assert_eq!(unsupported_comparator("=1.0"), Some("="));
        assert_eq!(unsupported_comparator("==1.0"), Some("=="));
        assert_eq!(unsupported_comparator(">=1.0"), None);
        assert_eq!(unsupported_comparator(">1.0"), None);
    }
}
