//! The association model and the built-in default spec.
//!
//! A [`Spec`] describes yazi's `[opener]` table (named openers) and the ordered
//! `prepend_rules` of its `[open]` table (file-type -> opener mappings). The order of
//! `prepend_rules` is significant — yazi merges them above its built-ins, first match
//! wins, so e.g. `*.md` must precede the catch-all `text/*`.
//!
//! [`Spec::builtin`] reproduces the niricritty yazi configuration. An external spec file
//! (see [`RawSpec`]) can override it via `--spec`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// yazi built-in openers that rules may reference without us defining them.
pub const BUILTIN_OPENERS: &[&str] = &["reveal", "open", "play"];

/// One command line within a named opener (one inline table in yazi's opener array).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenerRun {
    pub run: String,
    pub desc: Option<String>,
    pub block: bool,
    pub orphan: bool,
    /// yazi's `for` field ("unix" | "linux" | "windows" | "macos").
    pub for_platform: Option<String>,
}

/// A named opener: a key in `[opener]` whose value is an array of [`OpenerRun`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenerDef {
    pub name: String,
    /// Optional rationale rendered as a leading `#` comment.
    pub doc: Option<String>,
    pub runs: Vec<OpenerRun>,
}

/// What an open-rule matches on. Exactly one variant per rule, mirroring yazi.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matcher {
    Mime(String),
    Url(String),
}

/// One entry of `prepend_rules`. Order within [`Spec::prepend_rules`] is significant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRule {
    pub matcher: Matcher,
    pub use_openers: Vec<String>,
    pub doc: Option<String>,
}

/// The complete surface this tool owns: the `[opener]` table and `[open].prepend_rules`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub openers: Vec<OpenerDef>,
    pub prepend_rules: Vec<OpenRule>,
}

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("rule #{0} sets neither `mime` nor `url`")]
    EmptyMatcher(usize),
    #[error("rule #{0} sets both `mime` and `url` (exactly one allowed)")]
    AmbiguousMatcher(usize),
    #[error("rule #{0} has an empty `use` list")]
    EmptyUse(usize),
    #[error("rule #{rule} references undefined opener '{name}' (define it under [opener] or use a yazi built-in)")]
    UnknownOpener { rule: usize, name: String },
    #[error("duplicate opener name '{0}'")]
    DuplicateOpener(String),
}

impl Spec {
    /// Validate internal consistency: every `use` resolves to a defined or built-in
    /// opener, every rule matches on exactly one of mime/url, no empty `use` lists.
    pub fn validate(&self) -> Result<(), SpecError> {
        let mut names = std::collections::HashSet::new();
        for o in &self.openers {
            if !names.insert(o.name.as_str()) {
                return Err(SpecError::DuplicateOpener(o.name.clone()));
            }
        }
        for (i, r) in self.prepend_rules.iter().enumerate() {
            if r.use_openers.is_empty() {
                return Err(SpecError::EmptyUse(i));
            }
            for u in &r.use_openers {
                if !names.contains(u.as_str()) && !BUILTIN_OPENERS.contains(&u.as_str()) {
                    return Err(SpecError::UnknownOpener { rule: i, name: u.clone() });
                }
            }
        }
        Ok(())
    }

    /// The built-in default spec — reproduces niricritty's yazi `[opener]`/`[open]`.
    pub fn builtin() -> Self {
        let openers = vec![
            opener(
                "terminal",
                Some("Open a folder in a NEW ghostty window, cd'd into it (orphan detaches it so it survives yazi exiting)."),
                r#"ghostty --working-directory "$1" >/dev/null 2>&1"#,
                Some("Open in new ghostty"),
                false, true, Some("linux"),
            ),
            opener(
                "browser",
                Some("Open web/HTML in Chromium."),
                r#"chromium "$@""#,
                Some("Chromium"),
                false, true, Some("linux"),
            ),
            opener(
                "edit",
                Some("Open code/text in micro (block hands the terminal to micro)."),
                r#"${EDITOR:-micro} "$@""#,
                Some("micro"),
                true, false, Some("unix"),
            ),
            opener(
                "md-view",
                Some("View rendered Markdown (images + big headers via ghostty graphics) with mdfried."),
                r#"mdfried "$@""#,
                Some("mdfried (rendered markdown)"),
                true, false, Some("unix"),
            ),
            opener(
                "docx",
                Some("View Word .docx documents with doxx (terminal-native Word viewer)."),
                r#"doxx "$@""#,
                Some("doxx (Word viewer)"),
                true, false, Some("unix"),
            ),
            opener(
                "xlsx",
                Some("View Excel .xlsx/.xls with xleak (-i = interactive TUI)."),
                r#"xleak -i "$@""#,
                Some("xleak (Excel viewer)"),
                true, false, Some("unix"),
            ),
            opener(
                "csv",
                Some("View CSV with xan; -p pages via less (scrollable), -A lifts the 100-row cap."),
                r#"xan view -p -A "$@""#,
                Some("xan (CSV viewer)"),
                true, false, Some("unix"),
            ),
            opener(
                "env",
                Some("Edit .env files with lazyenv (TUI). lazyenv scans a DIRECTORY, so open the hovered file's parent."),
                r#"lazyenv "$(dirname "$1")""#,
                Some("lazyenv (.env TUI)"),
                true, false, Some("unix"),
            ),
        ];

        let prepend_rules = vec![
            rule_mime("inode/directory", &["terminal"], Some("directory -> new ghostty window")),
            rule_url("*.md", &["md-view", "edit", "reveal"], Some("markdown -> mdfried (must precede text/*). Press O for micro.")),
            rule_url("*.markdown", &["md-view", "edit", "reveal"], None),
            rule_mime("text/markdown", &["md-view", "edit", "reveal"], None),
            rule_mime(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                &["docx", "reveal"],
                Some("Word .docx -> doxx"),
            ),
            rule_url("*.docx", &["docx", "reveal"], None),
            rule_mime(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                &["xlsx", "reveal"],
                Some("Excel .xlsx/.xls -> xleak"),
            ),
            rule_mime("application/vnd.ms-excel", &["xlsx", "reveal"], None),
            rule_url("*.xlsx", &["xlsx", "reveal"], None),
            rule_url("*.xls", &["xlsx", "reveal"], None),
            rule_mime("text/csv", &["csv", "edit", "reveal"], Some("CSV -> xan (must precede text/*). Press O for micro.")),
            rule_url("*.csv", &["csv", "edit", "reveal"], None),
            rule_url(".env", &["env", "edit", "reveal"], Some(".env files -> lazyenv (must precede text/*). Press O for micro.")),
            rule_url(".env.*", &["env", "edit", "reveal"], None),
            rule_url("*.env", &["env", "edit", "reveal"], None),
            rule_mime("text/html", &["browser", "edit", "reveal"], Some("web / html -> Chromium")),
            rule_mime("application/xhtml+xml", &["browser", "edit", "reveal"], None),
            rule_url("*.html", &["browser", "edit", "reveal"], None),
            rule_url("*.htm", &["browser", "edit", "reveal"], None),
            rule_mime("text/*", &["edit", "reveal"], Some("code / text -> micro")),
            rule_mime("application/json", &["edit", "reveal"], None),
            rule_mime("application/javascript", &["edit", "reveal"], None),
            rule_mime("application/toml", &["edit", "reveal"], None),
            rule_mime("application/x-yaml", &["edit", "reveal"], None),
            rule_mime("application/xml", &["edit", "reveal"], None),
            rule_mime("application/x-shellscript", &["edit", "reveal"], None),
            rule_mime("inode/empty", &["edit", "reveal"], None),
        ];

        Spec { openers, prepend_rules }
    }
}

// ---- helpers for building the built-in spec --------------------------------------

fn opener(
    name: &str,
    doc: Option<&str>,
    run: &str,
    desc: Option<&str>,
    block: bool,
    orphan: bool,
    for_platform: Option<&str>,
) -> OpenerDef {
    OpenerDef {
        name: name.to_string(),
        doc: doc.map(str::to_string),
        runs: vec![OpenerRun {
            run: run.to_string(),
            desc: desc.map(str::to_string),
            block,
            orphan,
            for_platform: for_platform.map(str::to_string),
        }],
    }
}

fn rule_mime(mime: &str, uses: &[&str], doc: Option<&str>) -> OpenRule {
    OpenRule {
        matcher: Matcher::Mime(mime.to_string()),
        use_openers: uses.iter().map(|s| s.to_string()).collect(),
        doc: doc.map(str::to_string),
    }
}

fn rule_url(url: &str, uses: &[&str], doc: Option<&str>) -> OpenRule {
    OpenRule {
        matcher: Matcher::Url(url.to_string()),
        use_openers: uses.iter().map(|s| s.to_string()).collect(),
        doc: doc.map(str::to_string),
    }
}

// ---- external spec file (serde) --------------------------------------------------

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Serialize, Deserialize)]
struct RawOpener {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doc: Option<String>,
    run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desc: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    block: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    orphan: bool,
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    for_platform: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RawRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(rename = "use")]
    use_openers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doc: Option<String>,
}

/// The on-disk spec-file representation (`--spec` / `print-spec`).
#[derive(Debug, Serialize, Deserialize)]
pub struct RawSpec {
    #[serde(default)]
    opener: Vec<RawOpener>,
    #[serde(default)]
    rule: Vec<RawRule>,
}

impl RawSpec {
    /// Parse a spec file into a validated [`Spec`].
    pub fn parse(text: &str) -> anyhow::Result<Spec> {
        let raw: RawSpec = toml::from_str(text)?;
        let spec = raw.into_spec()?;
        spec.validate()?;
        Ok(spec)
    }

    fn into_spec(self) -> Result<Spec, SpecError> {
        let openers = self
            .opener
            .into_iter()
            .map(|o| OpenerDef {
                name: o.name,
                doc: o.doc,
                runs: vec![OpenerRun {
                    run: o.run,
                    desc: o.desc,
                    block: o.block,
                    orphan: o.orphan,
                    for_platform: o.for_platform,
                }],
            })
            .collect();

        let mut prepend_rules = Vec::with_capacity(self.rule.len());
        for (i, r) in self.rule.into_iter().enumerate() {
            let matcher = match (r.mime, r.url) {
                (Some(m), None) => Matcher::Mime(m),
                (None, Some(u)) => Matcher::Url(u),
                (None, None) => return Err(SpecError::EmptyMatcher(i)),
                (Some(_), Some(_)) => return Err(SpecError::AmbiguousMatcher(i)),
            };
            prepend_rules.push(OpenRule { matcher, use_openers: r.use_openers, doc: r.doc });
        }
        Ok(Spec { openers, prepend_rules })
    }
}

/// Serialize a [`Spec`] back to the editable spec-file format (for `print-spec`).
pub fn spec_to_file_string(spec: &Spec) -> anyhow::Result<String> {
    let raw = RawSpec {
        opener: spec
            .openers
            .iter()
            .flat_map(|o| {
                o.runs.iter().map(move |r| RawOpener {
                    name: o.name.clone(),
                    doc: o.doc.clone(),
                    run: r.run.clone(),
                    desc: r.desc.clone(),
                    block: r.block,
                    orphan: r.orphan,
                    for_platform: r.for_platform.clone(),
                })
            })
            .collect(),
        rule: spec
            .prepend_rules
            .iter()
            .map(|r| {
                let (mime, url) = match &r.matcher {
                    Matcher::Mime(m) => (Some(m.clone()), None),
                    Matcher::Url(u) => (None, Some(u.clone())),
                };
                RawRule { mime, url, use_openers: r.use_openers.clone(), doc: r.doc.clone() }
            })
            .collect(),
    };
    Ok(toml::to_string_pretty(&raw)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_expected_counts() {
        let s = Spec::builtin();
        assert_eq!(s.openers.len(), 8, "expected 8 named openers");
        assert_eq!(s.prepend_rules.len(), 27, "expected 27 prepend rules");
    }

    #[test]
    fn builtin_is_valid() {
        Spec::builtin().validate().expect("built-in spec must validate");
    }

    #[test]
    fn builtin_rules_reference_known_openers() {
        let s = Spec::builtin();
        let names: std::collections::HashSet<_> = s.openers.iter().map(|o| o.name.as_str()).collect();
        for r in &s.prepend_rules {
            for u in &r.use_openers {
                assert!(
                    names.contains(u.as_str()) || BUILTIN_OPENERS.contains(&u.as_str()),
                    "rule use '{u}' is neither defined nor a built-in",
                );
            }
        }
    }

    #[test]
    fn validate_rejects_unknown_opener() {
        let mut s = Spec::builtin();
        s.prepend_rules[0].use_openers = vec!["does-not-exist".into()];
        assert!(matches!(s.validate(), Err(SpecError::UnknownOpener { .. })));
    }

    #[test]
    fn spec_file_roundtrips() {
        let s = Spec::builtin();
        let text = spec_to_file_string(&s).unwrap();
        let back = RawSpec::parse(&text).unwrap();
        assert_eq!(s, back, "spec -> file -> spec must round-trip");
    }
}
