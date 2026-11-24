//! Dotted numeric versions and the version requirements tools pin
//! (tech-spec §9.7).
//!
//! Deliberately lenient: tools print versions in many shapes (`v24.21.0`,
//! `aiken v1.1.24+bacbeb3`, `go1.26.10`, `17.0.20.1`), and a requirement only
//! needs the numeric core. Pre-release and build suffixes are ignored.

use std::cmp::Ordering;
use std::fmt;

/// A version as its numeric components, compared component-wise with missing
/// trailing components read as zero (`9.8` == `9.8.0`).
#[derive(Debug, Clone)]
pub struct Version(Vec<u64>);

impl Version {
    /// Parse a declared version such as `"0.12.0"`, `"9.8"` or `"v1.1.23"`.
    /// Rejects anything that is not dotted numbers.
    pub fn parse(text: &str) -> Option<Self> {
        let core = text.trim().strip_prefix('v').unwrap_or(text.trim());
        let parts = core
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| part.parse().ok())
                    .flatten()
            })
            .collect::<Option<Vec<u64>>>()?;
        Some(Version(parts))
    }

    /// Find the first dotted version (at least `major.minor`) in a tool's
    /// `--version` output. A bare number is not taken as a version, so words
    /// like `x64` or `(build 17)` are skipped.
    pub fn find_in(output: &str) -> Option<Self> {
        let bytes = output.as_bytes();
        let mut start = 0;
        while start < bytes.len() {
            if !bytes[start].is_ascii_digit() {
                start += 1;
                continue;
            }
            // Greedily take `digits(.digits)*` from here.
            let mut end = start;
            let mut parts = Vec::new();
            loop {
                let digits_start = end;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                parts.push(&output[digits_start..end]);
                let more =
                    end + 1 < bytes.len() && bytes[end] == b'.' && bytes[end + 1].is_ascii_digit();
                if !more {
                    break;
                }
                end += 1;
            }
            if parts.len() >= 2 {
                return Version::parse(&parts.join("."));
            }
            start = end;
        }
        None
    }

    fn component(&self, i: usize) -> u64 {
        self.0.get(i).copied().unwrap_or(0)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (0..self.0.len().max(other.0.len()))
            .map(|i| self.component(i).cmp(&other.component(i)))
            .find(|ord| ord.is_ne())
            .unwrap_or(Ordering::Equal)
    }
}

fn fmt_components(components: &[u64], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for (i, part) in components.iter().enumerate() {
        if i > 0 {
            f.write_str(".")?;
        }
        write!(f, "{part}")?;
    }
    Ok(())
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_components(&self.0, f)
    }
}

impl serde::Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A prefix-exact version requirement: the declared components must match the
/// installed version's leading components. `1.1.24` is exact, `1.1` is
/// `1.1.*`, `1` is `1.*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReq(Vec<u64>);

impl VersionReq {
    /// Parse a declared requirement such as `"1.1.24"`, `"0.12"` or `"1"`.
    pub fn parse(text: &str) -> Option<Self> {
        Version::parse(text).map(|v| VersionReq(v.0))
    }

    pub fn matches(&self, version: &Version) -> bool {
        self.0
            .iter()
            .enumerate()
            .all(|(i, c)| version.component(i) == *c)
    }

    /// `version` is newer than every version this requirement allows
    /// (`1.2.0` or `2.0` against `1.1`).
    pub fn is_below(&self, version: &Version) -> bool {
        self.0
            .iter()
            .enumerate()
            .map(|(i, c)| version.component(i).cmp(c))
            .find(|ord| ord.is_ne())
            == Some(Ordering::Greater)
    }

    /// Human-readable form: `1.1.24` stays as is, a partial pin reads `1.1.x`.
    pub fn label(&self) -> String {
        if self.0.len() >= 3 {
            self.to_string()
        } else {
            format!("{self}.x")
        }
    }

    /// The tighter of two requirements when one refines the other (`1.1` and
    /// `1.1.24` → `1.1.24`); `None` when no version satisfies both.
    pub fn narrow(&self, other: &VersionReq) -> Option<VersionReq> {
        let (short, long) = if self.0.len() <= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        long.0.starts_with(&short.0).then(|| long.clone())
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_components(&self.0, f)
    }
}

impl serde::Serialize for VersionReq {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn parses_declared_versions() {
        assert_eq!(v("0.12.0").to_string(), "0.12.0");
        assert_eq!(v("v1.1.23").to_string(), "1.1.23");
        assert_eq!(v("9.8").to_string(), "9.8");
        assert!(Version::parse("").is_none());
        assert!(Version::parse("1..2").is_none());
        assert!(Version::parse("0.12.0-beta1").is_none());
        assert!(Version::parse("latest").is_none());
    }

    #[test]
    fn missing_components_compare_as_zero() {
        assert_eq!(v("9.8"), v("9.8.0"));
        assert!(v("9.8.4") > v("9.8"));
        assert!(v("0.10.6") < v("0.12.0"));
        assert!(v("1.1.24") > v("1.1.23"));
        assert!(v("0.2.10") > v("0.2.9"));
    }

    #[test]
    fn finds_versions_in_real_tool_output() {
        let cases = [
            ("just 1.58.0", "1.58.0"),
            ("v24.21.0", "24.21.0"),
            ("aiken v1.1.24+bacbeb3", "1.1.24"),
            ("Docker version 29.9.2, build v29.9.2", "29.9.2"),
            ("openjdk 17.0.20.1 2026-08-18\nOpenJDK Runtime", "17.0.20.1"),
            ("go version go1.26.10 linux/amd64", "1.26.10"),
            (
                "The Glorious Glasgow Haskell Compilation System, version 9.8.4",
                "9.8.4",
            ),
            ("cabal-install version 3.10.3.0", "3.10.3.0"),
            ("trix 0.26.2", "0.26.2"),
            ("0.12.0-beta1", "0.12.0"),
        ];
        for (output, expected) in cases {
            assert_eq!(
                Version::find_in(output).map(|v| v.to_string()).as_deref(),
                Some(expected),
                "{output}"
            );
        }
    }

    #[test]
    fn output_without_a_dotted_version_is_none() {
        assert!(Version::find_in("cardano-up devel (commit )").is_none());
        assert!(Version::find_in("linux-x64 build 17").is_none());
        assert!(Version::find_in("Unrecognized option: --version").is_none());
        assert!(Version::find_in("").is_none());
    }

    fn req(text: &str) -> VersionReq {
        VersionReq::parse(text).unwrap()
    }

    #[test]
    fn req_matches_by_prefix() {
        assert!(req("1.1.24").matches(&v("1.1.24")));
        assert!(!req("1.1.24").matches(&v("1.1.25")));
        assert!(req("1.1").matches(&v("1.1.0")));
        assert!(req("1.1").matches(&v("1.1.99")));
        assert!(!req("1.1").matches(&v("1.2.0")));
        assert!(req("1").matches(&v("1.58.0")));
        assert!(!req("1").matches(&v("2.0.0")));
        assert!(!req("1.1.24").matches(&v("1.1")));
        assert!(VersionReq::parse("latest").is_none());
    }

    #[test]
    fn label_marks_partial_pins() {
        assert_eq!(req("1.1.24").label(), "1.1.24");
        assert_eq!(req("0.12").label(), "0.12.x");
        assert_eq!(req("1").label(), "1.x");
    }

    #[test]
    fn is_below_only_when_the_version_is_past_the_pin() {
        assert!(req("1.1.24").is_below(&v("1.2.0")));
        assert!(req("1.1.24").is_below(&v("1.1.25")));
        assert!(req("1").is_below(&v("2.0.1")));
        assert!(req("0.12").is_below(&v("0.13.0")));
        assert!(!req("1.1.24").is_below(&v("1.1.24")));
        assert!(!req("1.1.24").is_below(&v("1.1.21")));
        assert!(!req("1").is_below(&v("1.40.0")));
        assert!(!req("0.12").is_below(&v("0.11.3")));
    }

    #[test]
    fn narrow_keeps_the_more_specific_requirement() {
        assert_eq!(req("1.1").narrow(&req("1.1.24")), Some(req("1.1.24")));
        assert_eq!(req("1.1.24").narrow(&req("1")), Some(req("1.1.24")));
        assert_eq!(req("1.1").narrow(&req("1.1")), Some(req("1.1")));
        assert_eq!(req("1.1").narrow(&req("1.2")), None);
        assert_eq!(req("18").narrow(&req("20")), None);
    }
}
