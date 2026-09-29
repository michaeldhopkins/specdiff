//! The abstraction both sides are reduced to, the comparison, and the known-differentials list.
//!
//! A test is identified by the file it is written in (relative to the project root, `/`
//! separated) and the chain of names from the file down to the test: enclosing groups
//! (modules, classes, describe blocks) then the test itself, each in the display form the
//! framework's `normalization` produces. A parametrised test is one identifier carrying its
//! case count, not one per case. Both sides are compared as multisets of these identifiers.

use std::collections::BTreeMap;

use specdiff::parse::registry::NormalizationDef;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TestId {
    pub file: String,
    pub path: Vec<String>,
    pub cases: Option<usize>,
}

impl TestId {
    pub fn key(&self) -> String {
        let mut key = self.file.clone();
        for segment in &self.path {
            key.push_str(" > ");
            key.push_str(segment);
        }
        if let Some(n) = self.cases {
            key.push_str(&format!(" [{n} cases]"));
        }
        key
    }
}

/// The framework's name normalisation, written again here rather than borrowed from the
/// engine, so a disagreement between the two shows up as a difference instead of hiding.
pub fn normalize(name: &str, norm: Option<&NormalizationDef>) -> String {
    let Some(norm) = norm else { return name.to_string() };
    if norm.raw {
        return name.to_string();
    }
    let mut out = name;
    if norm.strip_camel_test_prefix
        && let Some(rest) = out.strip_prefix("Test")
        && rest.starts_with(char::is_uppercase)
    {
        out = rest;
    }
    if let Some(rest) = norm.strip_prefixes.iter().find_map(|p| out.strip_prefix(p.as_str())) {
        out = rest;
    }
    if let Some(rest) = norm.strip_suffixes.iter().find_map(|s| out.strip_suffix(s.as_str())) {
        out = rest;
    }
    if norm.underscore_to_space { out.replace('_', " ") } else { out.to_string() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// The framework lists it; specdiff's outline does not have it.
    MissingInSpecdiff,
    /// specdiff's outline has it; the framework does not list it.
    ExtraInSpecdiff,
}

impl Side {
    fn parse(s: &str) -> Option<Side> {
        match s {
            "missing" => Some(Side::MissingInSpecdiff),
            "extra" => Some(Side::ExtraInSpecdiff),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Side::MissingInSpecdiff => "missing in specdiff",
            Side::ExtraInSpecdiff => "extra in specdiff",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    pub side: Side,
    pub key: String,
}

/// Every identifier whose count differs between the two multisets, once per unit of
/// difference.
pub fn compare(reference: &[TestId], specdiff: &[TestId]) -> Vec<Difference> {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for id in reference {
        *counts.entry(id.key()).or_default() += 1;
    }
    for id in specdiff {
        *counts.entry(id.key()).or_default() -= 1;
    }
    let mut out = Vec::new();
    for (key, n) in counts {
        let side = if n > 0 { Side::MissingInSpecdiff } else { Side::ExtraInSpecdiff };
        for _ in 0..n.unsigned_abs() {
            out.push(Difference { side, key: key.clone() });
        }
    }
    out
}

#[derive(Debug)]
pub struct Known {
    pub class: String,
    pub family: String,
    pub side: Side,
    pub pattern: glob::Pattern,
    pub reason: String,
}

pub fn load_known(text: &str) -> Vec<Known> {
    let table: toml::Table = text.parse().unwrap_or_else(|e| panic!("known.toml does not parse: {e}"));
    let entries = table.get("known").and_then(toml::Value::as_array).cloned().unwrap_or_default();
    entries
        .iter()
        .map(|entry| {
            let field = |name: &str| {
                entry
                    .get(name)
                    .and_then(toml::Value::as_str)
                    .unwrap_or_else(|| panic!("known.toml entry {entry} has no `{name}`"))
                    .to_string()
            };
            let reason = field("reason");
            assert!(!reason.trim().is_empty(), "known.toml entry {} has an empty reason", field("class"));
            Known {
                class: field("class"),
                family: field("family"),
                side: Side::parse(&field("side"))
                    .unwrap_or_else(|| panic!("known.toml: side must be `missing` or `extra` in {entry}")),
                pattern: glob::Pattern::new(&field("match")).unwrap_or_else(|e| panic!("known.toml: bad glob: {e}")),
                reason,
            }
        })
        .collect()
}

/// The class a difference belongs to: the first known entry that matches it, or `None` when
/// nothing explains it.
pub fn classify<'a>(family: &str, diff: &Difference, known: &'a [Known]) -> Option<&'a Known> {
    known
        .iter()
        .find(|k| k.family == family && k.side == diff.side && k.pattern.matches(&diff.key))
}

#[derive(Debug, Default)]
pub struct Report {
    /// (family, side, class) to count; class is `UNEXPLAINED` when nothing matched.
    pub buckets: BTreeMap<(String, Side, String), usize>,
    pub unexplained: Vec<(String, Difference)>,
    pub compared: BTreeMap<String, (usize, usize)>,
}

pub const UNEXPLAINED: &str = "UNEXPLAINED";

impl Report {
    pub fn add(&mut self, family: &str, reference: &[TestId], specdiff: &[TestId], known: &[Known]) {
        let entry = self.compared.entry(family.to_string()).or_default();
        entry.0 += reference.len();
        entry.1 += specdiff.len();
        for diff in compare(reference, specdiff) {
            let class = classify(family, &diff, known).map_or(UNEXPLAINED, |k| k.class.as_str());
            *self.buckets.entry((family.to_string(), diff.side, class.to_string())).or_default() += 1;
            if class == UNEXPLAINED {
                self.unexplained.push((family.to_string(), diff));
            }
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::from("differential: tests listed by the framework / outlined by specdiff\n");
        for (family, (r, s)) in &self.compared {
            out.push_str(&format!("  {family}: {r} / {s}\n"));
        }
        out.push_str("differences by class:\n");
        for ((family, side, class), n) in &self.buckets {
            out.push_str(&format!("  {family:<12} {:<20} {class:<32} {n}\n", side.label()));
        }
        for (family, diff) in &self.unexplained {
            out.push_str(&format!("  UNEXPLAINED {family} {}: {}\n", diff.side.label(), diff.key));
        }
        out
    }

    /// Known classes for the given families that explained nothing this run.
    pub fn unused<'a>(&self, families: &[&str], known: &'a [Known]) -> Vec<&'a Known> {
        known
            .iter()
            .filter(|k| families.contains(&k.family.as_str()))
            .filter(|k| !self.buckets.keys().any(|(f, s, c)| *f == k.family && *s == k.side && *c == k.class))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(file: &str, path: &[&str], cases: Option<usize>) -> TestId {
        TestId { file: file.into(), path: path.iter().map(|s| (*s).to_string()).collect(), cases }
    }

    #[test]
    fn compare_counts_each_unit_of_difference_on_its_side() {
        let a = id("f", &["g", "t"], None);
        let b = id("f", &["g", "u"], Some(2));
        let diffs = compare(&[a.clone(), a.clone(), b.clone()], &[a.clone(), id("f", &["g", "u"], Some(3))]);
        let keys: Vec<(Side, String)> = diffs.into_iter().map(|d| (d.side, d.key)).collect();
        assert_eq!(
            keys,
            [
                (Side::MissingInSpecdiff, "f > g > t".to_string()),
                (Side::MissingInSpecdiff, "f > g > u [2 cases]".to_string()),
                (Side::ExtraInSpecdiff, "f > g > u [3 cases]".to_string()),
            ]
        );
    }

    #[test]
    fn normalize_follows_each_rule_once() {
        let norm = NormalizationDef {
            strip_prefixes: vec!["test_".into()],
            strip_suffixes: vec!["Test".into()],
            underscore_to_space: true,
            strip_camel_test_prefix: true,
            raw: false,
        };
        assert_eq!(normalize("test_a_b", Some(&norm)), "a b");
        assert_eq!(normalize("TestUser", Some(&norm)), "User");
        assert_eq!(normalize("Testing", Some(&norm)), "Testing");
        assert_eq!(normalize("UserTest", Some(&norm)), "User");
        assert_eq!(normalize("x_y", None), "x_y");
    }

    #[test]
    fn a_difference_is_explained_only_by_its_family_and_side() {
        let known = load_known(
            "[[known]]\nclass = \"c\"\nfamily = \"go\"\nside = \"extra\"\nmatch = \"* > * > *\"\nreason = \"r\"\n",
        );
        let extra = Difference { side: Side::ExtraInSpecdiff, key: "p > T > sub".into() };
        let missing = Difference { side: Side::MissingInSpecdiff, key: "p > T > sub".into() };
        assert!(classify("go", &extra, &known).is_some());
        assert!(classify("go", &missing, &known).is_none());
        assert!(classify("pytest", &extra, &known).is_none());
    }
}
