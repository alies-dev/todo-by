//! Semantic-version-shaped parsing and comparison for version-constraint
//! triggers, e.g. a tag written `>=v2.0` firing once the project reaches 2.0.
//!
//! Versions written in a tag carry a mandatory `v` marker; versions the
//! tool resolves for itself (a git tag, a `version-cmd`) do not. See
//! [`marked_version`] for the split.

use std::cmp::Ordering;
use std::fmt;

/// A parsed version: a numeric core (1 to 3 dot-separated components) plus
/// an optional pre-release suffix. Build metadata (`+...`) is accepted by
/// [`Version::parse`] but discarded immediately: per semver it never affects
/// ordering, so keeping it around would just be dead weight.
#[derive(Clone, Eq, Debug)]
pub struct Version {
    parts: Vec<u64>,
    pre: Option<String>,
}

/// A single dot-separated pre-release or build identifier must be
/// non-empty and use only semver's identifier charset (ASCII
/// alphanumerics and `-`). Rejecting anything else (an underscore, a
/// stray `+`) means a typo surfaces as an invalid trigger instead of
/// silently parsing into a different, unintended version.
fn is_valid_identifier(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl Version {
    /// Parses `[v]<core>[-<pre>][+<build>]`, where `<core>` is 1 to 3
    /// dot-separated ASCII-digit components each fitting a `u64`.
    ///
    /// Rejects rather than degrades on anything ambiguous: an empty
    /// component (`2..0`, `.2.0`), an empty pre-release or build after the
    /// separator (`2.0-`, `2.0+`), an empty dot-separated identifier inside
    /// either (`2.0-alpha..1`, `2.0+build..1`, `2.0-alpha.`), or an
    /// identifier containing anything outside ASCII alphanumerics and `-`
    /// (`2.0-rc_1`; `2.0+build+other`, where the second `+` lands inside
    /// the build identifiers) all return `None` instead of silently
    /// parsing as a shorter, different version. That mirrors
    /// `date::deadline`'s stance on malformed tokens: a typo should surface
    /// as an invalid trigger, not quietly mean something else.
    pub fn parse(s: &str) -> Option<Self> {
        // Lowercase only: `v2.0` is the near-universal spelling, and
        // accepting `V2.0` as well would mean two ways to write one thing
        // for no gain. The scanner still recognizes an uppercase `V` as a
        // version position, so `V2.0` fails here and is reported invalid
        // rather than passing by unnoticed.
        let s = s.strip_prefix('v').unwrap_or(s);
        let core_and_pre = match s.split_once('+') {
            Some((head, build)) => {
                if !build.split('.').all(is_valid_identifier) {
                    return None;
                }
                head
            }
            None => s,
        };
        let (core, pre) = match core_and_pre.split_once('-') {
            Some((core, pre)) => {
                if !pre.split('.').all(is_valid_identifier) {
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

/// Renders the parsed core (as many components as were written, not
/// zero-padded) plus `-pre` when present. Build metadata isn't stored, so
/// it never round-trips: `Version::parse("v2.1+build")` displays as `2.1`.
/// This is what lets `main.rs` derive a current-version display string
/// straight from a parsed `Version` instead of hand-stripping a `v`/`V`
/// prefix itself.
impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{part}")?;
        }
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
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
    match (is_numeric_identifier(a), is_numeric_identifier(b)) {
        (true, true) => compare_numeric_identifiers(a, b),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.cmp(b),
    }
}

/// Compares two all-digit identifiers as arbitrary-precision numbers,
/// without parsing into a `u64`: an identifier can legally be longer than
/// 20 digits (nothing in the grammar caps it), so parsing would overflow
/// and silently fall back to lexicographic order, which disagrees with
/// numeric order for identifiers of different lengths (`"9...9"`, 22
/// nines, is numerically less than `"1" + "0"*22`, but sorts after it
/// byte-wise). Stripping leading zeros first turns the comparison into
/// (length, then lexicographic): equal-length digit strings with no
/// leading zeros compare the same both ways, and stripping preserves the
/// documented leading-zero equality (`01` == `1`, since both strip to
/// `1`; `0` strips to the empty string, which still compares correctly
/// against another all-zero identifier or against `1`).
fn compare_numeric_identifiers(a: &str, b: &str) -> Ordering {
    let a = a.trim_start_matches('0');
    let b = b.trim_start_matches('0');
    (a.len(), a).cmp(&(b.len(), b))
}

/// Leading zeroes are accepted and compared numerically (`01` == `1`):
/// deliberate leniency, consistent with this parser accepting 1 or 2
/// component cores and a leading `v`. Strict semver (section 9) would
/// instead deem the whole version invalid; reinterpreting `01` as
/// alphanumeric would rank it above every numeric identifier, which is
/// worse than either option.
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Constraint {
    pub cmp: Cmp,
    pub version: Version,
}

impl Constraint {
    /// Parses a comparator-prefixed constraint such as `>=v2.0` or
    /// `>v1.4.0-rc.1`. Only `>=` and `>` are recognized here; `<`, `<=`, `=`,
    /// and `==` are syntactically version-like but return `None` because
    /// this tool has no "before version X" semantics to give them (see
    /// [`unsupported_comparator`] for surfacing that distinctly from a
    /// plain parse failure).
    ///
    /// The `v` is required, same as in [`Constraint::parse_bare`]; see
    /// [`marked_version`] for why.
    pub fn parse(written: &str) -> Option<Constraint> {
        let (cmp, rest) = if let Some(rest) = written.strip_prefix(">=") {
            (Cmp::Ge, rest)
        } else {
            (Cmp::Gt, written.strip_prefix('>')?)
        };
        // `rest` may carry the space from a `>= v2.0` spelling, which the
        // scanner accepts (see its `parse_version_span`).
        let version = marked_version(rest.trim_start())?;
        Some(Constraint { cmp, version })
    }

    /// Parses a comparator-less constraint (`v2.0`, `v2026.01`), which means
    /// exactly what `>=` means: fire once the project reaches that version.
    /// `>=` is what nearly every real tag wants, so it gets the short
    /// spelling; anything else has to say so explicitly.
    ///
    /// Still requires the `v` (see [`marked_version`]), so a digit-leading
    /// token never reaches a `Constraint` at all. The scanner routes those
    /// here anyway, precisely so this rejects them and they surface as
    /// invalid triggers rather than going unreported.
    pub fn parse_bare(written: &str) -> Option<Constraint> {
        Some(Constraint {
            cmp: Cmp::Ge,
            version: marked_version(written)?,
        })
    }

    pub fn satisfied_by(&self, current: &Version) -> bool {
        match self.cmp {
            Cmp::Ge => current >= &self.version,
            Cmp::Gt => current > &self.version,
        }
    }
}

/// Parses a version written in a tag, where the leading `v` is mandatory:
/// `v2.0` yes, `2.0` no.
///
/// The marker is what makes a trigger self-describing. A tag sits in prose,
/// next to a sentence, so an unmarked number is ambiguous three ways at
/// once: `2026.09.01` reads as both a dotted deadline and a calendar
/// version, `3.5` reads as both a constraint and the start of "3.5 hours of
/// work", and `12.5.2026` reads as a day-first date and a three-component
/// version. Every one of those needs a rule to resolve, and a rule that
/// guesses wrong either fires a tag nobody wrote or, worse, quietly holds
/// one back. Requiring the marker deletes the whole class: dates are the
/// ones with dashes, versions are the ones with a `v`, and anything else is
/// reported so the author can say which they meant.
///
/// Deliberately NOT applied to a resolved current version. Those come from
/// `git describe`, a `version-cmd`, `TODO_BY_VERSION`, or
/// `--current-version`, none of which the tag author controls, and where a
/// bare `1.2.3` is the norm. `main.rs` calls [`Version::parse`] directly for
/// that path, which keeps the `v` optional.
fn marked_version(written: &str) -> Option<Version> {
    // Checked here, then handed on WITH the `v` still attached, since
    // `Version::parse` strips one itself. Stripping it here too would let
    // `vv2.0` through: this check would see the first `v`, and the parser
    // would eat the second.
    if !written.starts_with('v') {
        return None;
    }
    Version::parse(written)
}

/// The correctly marked spelling of a trigger written without a usable `v`,
/// when such a spelling exists: `2.0` -> `v2.0`, `>=2.0` -> `>=v2.0`,
/// `>= 2.0` -> `>= v2.0`, and an uppercase `V2.0` -> `v2.0`. Returns None
/// when adding the marker wouldn't help anyway (`>=2.x`, `2.0.5.1`), so
/// those keep the generic invalid-constraint wording instead of being told
/// to write something that is also invalid.
///
/// Unsupported comparators are excluded because they have their own, more
/// specific remedy: `<2.0` is not one `v` away from working.
pub fn missing_v_marker(written: &str) -> Option<String> {
    if unsupported_comparator(written).is_some() {
        return None;
    }
    let at = written.bytes().position(|b| b.is_ascii_digit())?;
    let head = &written[at.saturating_sub(1)..at];
    // A correctly placed lowercase `v` means the marker isn't what's wrong
    // here, so say nothing and let the generic wording take it. Covers both
    // an already-marked token whose version is malformed (`>=v2.x`) and a
    // doubled marker (`vv2.0`), neither of which another `v` would fix.
    if head == "v" {
        return None;
    }
    // Everything before the first digit is a comparator, any spaces after
    // it, and possibly a `v` that failed only by being uppercase.
    let head = written[..at].trim_end_matches(['v', 'V']);
    let fixed = format!("{head}v{}", &written[at..]);
    let usable = if head.is_empty() {
        Constraint::parse_bare(&fixed).is_some()
    } else {
        Constraint::parse(&fixed).is_some()
    };
    usable.then_some(fixed)
}

/// All comparators the scanner should recognize as "this position might be
/// a version constraint": the union of what `Constraint::parse` accepts
/// (`>=`, `>`) and what `unsupported_comparator` rejects (`==`, `<=`, `<`,
/// `=`). Ordered longest-prefix-first so `>=`/`<=`/`==` are matched before
/// their single-character prefixes `>`/`<`/`=` (matching `>` first on
/// `>=2.0` would wrongly leave `=2.0` as the "version" token).
///
/// The scanner's span detection consumes directly from this list, so a
/// comparator missing here is never even recognized as a trigger position:
/// that's a silent false negative (never reported at all, not even as
/// `InvalidTrigger`), not a loud rejection. Keep this in sync with
/// `Constraint::parse` and [`UNSUPPORTED_COMPARATORS`].
pub const COMPARATORS: [&str; 8] = [">=", "<=", "==", ">", "<", "=", "^", "~"];

/// Comparators that read as "before version X" (`<1.0`, `<=1.0`, `=1.0`,
/// `==1.0`), a natural thing to reach for, plus the range operators every
/// package manager uses (`^1.0`, `~1.0`), which pin an upper bound this
/// tool has no way to act on. Silently treating any of them as unparsable
/// would be worse than useless: unlike a plain typo, these read as valid
/// intent that would otherwise never fire, postponing the chore forever.
/// Ordered longest-prefix-first so `==`/`<=` are found before `=`/`<`.
const UNSUPPORTED_COMPARATORS: [&str; 6] = ["==", "<=", "<", "=", "^", "~"];

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
    }

    #[test]
    fn rejects_uppercase_v_prefix() {
        assert!(Version::parse("V2.0.5").is_none());
        assert!(Version::parse("V3").is_none());
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
    fn pre_and_build_identifiers_reject_characters_outside_the_semver_charset() {
        assert_eq!(Version::parse("1.0.0-rc_1"), None, "underscore not allowed");
        assert_eq!(
            Version::parse("1.0.0+build+other"),
            None,
            "second '+' lands inside the build identifiers"
        );
    }

    #[test]
    fn hyphenated_pre_release_identifier_is_legal() {
        assert!(Version::parse("1.0.0-rc-1").is_some());
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
    fn leading_zero_numeric_identifiers_compare_numerically() {
        // Deliberate leniency: strict SemVer rejects a leading zero here
        // outright, but this parser treats "01" as the number 1.
        assert_eq!(v("1.0.0-alpha.01"), v("1.0.0-alpha.1"));
        assert!(v("1.0.0-alpha.010") < v("1.0.0-alpha.11"));
    }

    #[test]
    fn numeric_pre_release_identifiers_beyond_u64_compare_correctly() {
        // Both identifiers exceed u64::MAX's 20 digits, so naive u64
        // parsing would overflow. A lexicographic fallback disagrees with
        // numeric order here: '9' > '1' byte-wise, even though the
        // 23-digit number is numerically larger than the 22-digit one.
        let nines = "9".repeat(22);
        let one_and_zeros = format!("1{}", "0".repeat(22));
        assert!(nines.parse::<u64>().is_err(), "fixture must overflow u64");
        assert!(
            one_and_zeros.parse::<u64>().is_err(),
            "fixture must overflow u64"
        );
        assert!(v(&format!("1.0.0-{nines}")) < v(&format!("1.0.0-{one_and_zeros}")));
    }

    #[test]
    fn constraint_parses_ge_and_gt_only() {
        let c = Constraint::parse(">=v2.0").unwrap();
        assert_eq!(c.cmp, Cmp::Ge);
        assert_eq!(c.version, v("2.0"));

        let c = Constraint::parse(">v1.4.0-rc.1").unwrap();
        assert_eq!(c.cmp, Cmp::Gt);
        assert_eq!(c.version, v("1.4.0-rc.1"));

        assert!(Constraint::parse("<v1.0").is_none());
        assert!(Constraint::parse("<=v1.0").is_none());
        assert!(Constraint::parse("=v1.0").is_none());
        assert!(Constraint::parse("==v1.0").is_none());
    }

    #[test]
    fn constraint_parse_rejects_malformed_version_after_valid_comparator() {
        assert!(Constraint::parse(">=v2.x").is_none());
        assert!(Constraint::parse(">=").is_none());
    }

    #[test]
    fn constraint_requires_the_v_marker() {
        // The whole disambiguation rule: an unmarked number is never a
        // constraint, in either spelling.
        assert!(Constraint::parse_bare("2.0").is_none());
        assert!(Constraint::parse_bare("2026.01").is_none());
        assert!(Constraint::parse(">=2.0").is_none());
        assert!(Constraint::parse(">2.0").is_none());
        assert!(Constraint::parse(">= 2.0").is_none());
        // Lowercase only, and only one of them: `Version::parse` strips a
        // `v` itself, so the marker check must not strip a second.
        assert!(Constraint::parse_bare("V2.0").is_none());
        assert!(Constraint::parse_bare("vv2.0").is_none());
        // Marked spellings, including the spaced comparator form.
        assert!(Constraint::parse_bare("v2.0").is_some());
        assert!(Constraint::parse(">=v2.0").is_some());
        assert!(Constraint::parse(">= v2.0").is_some());
    }

    #[test]
    fn resolved_current_versions_do_not_need_the_marker() {
        // `--current-version`, `TODO_BY_VERSION`, `version-cmd`, and `git
        // describe` all produce strings the tag author doesn't control, and
        // a bare `1.2.3` is the norm there. main.rs parses those with
        // `Version::parse`, which must stay marker-optional.
        assert!(Version::parse("1.2.3").is_some());
        assert!(Version::parse("v1.2.3").is_some());
    }

    #[test]
    fn missing_v_marker_names_the_fix_only_when_one_exists() {
        assert_eq!(missing_v_marker("2.0").as_deref(), Some("v2.0"));
        assert_eq!(
            missing_v_marker("2026.09.01").as_deref(),
            Some("v2026.09.01")
        );
        assert_eq!(missing_v_marker(">=2.0").as_deref(), Some(">=v2.0"));
        assert_eq!(missing_v_marker(">2.0").as_deref(), Some(">v2.0"));
        assert_eq!(missing_v_marker(">= 2.0").as_deref(), Some(">= v2.0"));
        // An uppercase V failed only on case, so the fix is still a marker.
        assert_eq!(missing_v_marker("V2.0").as_deref(), Some("v2.0"));
        assert_eq!(missing_v_marker(">=V2.0").as_deref(), Some(">=v2.0"));
        // Already marked, so a different error entirely.
        assert_eq!(missing_v_marker("v2.0"), None);
        // No marker would rescue these, so they keep the generic wording
        // rather than being told to write something also invalid.
        assert_eq!(missing_v_marker("2.0.5.1"), None, "four components");
        assert_eq!(missing_v_marker(">=2.x"), None);
        assert_eq!(missing_v_marker("2.0_rc"), None);
        // Unsupported comparators have their own, more specific remedy.
        assert_eq!(missing_v_marker("<1.0"), None);
        assert_eq!(missing_v_marker("^1.0"), None);
    }

    #[test]
    fn satisfied_by_uses_the_right_comparator() {
        let ge = Constraint::parse(">=v2.0").unwrap();
        assert!(ge.satisfied_by(&v("2.0")));
        assert!(ge.satisfied_by(&v("2.0.5")));
        assert!(!ge.satisfied_by(&v("1.9.9")));

        let gt = Constraint::parse(">v2.0").unwrap();
        assert!(!gt.satisfied_by(&v("2.0")));
        assert!(gt.satisfied_by(&v("2.0.1")));
    }

    #[test]
    fn display_renders_parsed_core_and_pre_without_v_prefix_or_build() {
        assert_eq!(Version::parse("v2.1").unwrap().to_string(), "2.1");
        assert_eq!(Version::parse("2").unwrap().to_string(), "2");
        assert_eq!(
            Version::parse("2.0.0-rc.1").unwrap().to_string(),
            "2.0.0-rc.1"
        );
        assert_eq!(
            Version::parse("v3.4.5+build.1").unwrap().to_string(),
            "3.4.5"
        );
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
