//! Render a [`Spec`] into the `[opener]` and `[open]` TOML sections as text.
//!
//! We emit the text ourselves (rather than via `toml_edit` serialization) so the exact
//! formatting — literal-quoted shell commands, one rule per line, rationale comments — is
//! under our control and stable across runs. [`crate::tomlio`] splices this verbatim.

use crate::spec::{Matcher, OpenRule, OpenerRun, Spec};

/// Render the `[opener]` + `[open]` sections. Always ends with a trailing newline.
pub fn fragment(spec: &Spec) -> String {
    let mut out = String::new();

    out.push_str("[opener]\n");
    for o in &spec.openers {
        if let Some(doc) = &o.doc {
            push_comment(&mut out, "", doc);
        }
        out.push_str(&key(&o.name));
        out.push_str(" = [");
        let runs: Vec<String> = o.runs.iter().map(render_run).collect();
        out.push_str(&runs.join(", "));
        out.push_str("]\n");
    }

    out.push('\n');
    out.push_str("[open]\n");
    out.push_str(
        "# prepend_rules merge ABOVE yazi's built-ins (first match wins); order matters.\n",
    );
    out.push_str("prepend_rules = [\n");
    for r in &spec.prepend_rules {
        if let Some(doc) = &r.doc {
            push_comment(&mut out, "\t", doc);
        }
        out.push('\t');
        out.push_str(&render_rule(r));
        out.push_str(",\n");
    }
    out.push_str("]\n");

    out
}

fn push_comment(out: &mut String, indent: &str, doc: &str) {
    for line in doc.lines() {
        out.push_str(indent);
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
}

fn render_run(r: &OpenerRun) -> String {
    let mut parts = vec![format!("run = {}", toml_string(&yazi_args(&r.run)))];
    if let Some(d) = &r.desc {
        parts.push(format!("desc = {}", basic(d)));
    }
    if r.block {
        parts.push("block = true".to_string());
    }
    if r.orphan {
        parts.push("orphan = true".to_string());
    }
    if let Some(f) = &r.for_platform {
        parts.push(format!("for = {}", basic(f)));
    }
    format!("{{ {} }}", parts.join(", "))
}

fn render_rule(r: &OpenRule) -> String {
    let matcher = match &r.matcher {
        Matcher::Mime(s) => format!("mime = {}", basic(s)),
        Matcher::Url(s) => format!("url = {}", basic(s)),
    };
    let uses: Vec<String> = r.use_openers.iter().map(|u| basic(u)).collect();
    format!("{{ {}, use = [{}] }}", matcher, uses.join(", "))
}

/// yazi 26 hands an opener its files through `%s` (every selected file, shell-escaped),
/// not through `$@`/`$1`: `sh -c <run>` no longer receives them as positional arguments —
/// yazi-core marks that path "TODO: remove" and the navigator test found `"$@"` empty.
/// The spec keeps the POSIX spelling because broot really does pass positionals; this
/// is where the yazi rendering translates it.
fn yazi_args(run: &str) -> String {
    run.replace("\"$@\"", "%s")
        .replace("\"$1\"", "%s")
        .replace("$@", "%s")
        .replace("$1", "%s")
}

/// A TOML bare key if safe, otherwise a quoted basic string.
fn key(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        s.to_string()
    } else {
        basic(s)
    }
}

/// Prefer a TOML literal string (`'...'`) so shell commands with `"`/`$` stay readable;
/// fall back to a basic string when the value contains a single quote or newline.
fn toml_string(s: &str) -> String {
    if !s.contains('\'') && !s.contains('\n') {
        format!("'{s}'")
    } else {
        basic(s)
    }
}

/// A TOML basic (double-quoted) string with the required escapes.
fn basic(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04X}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::DocumentMut;

    #[test]
    fn fragment_is_valid_toml() {
        let frag = fragment(&Spec::builtin());
        let doc: DocumentMut = frag.parse().expect("fragment must be valid TOML");
        assert!(doc.contains_key("opener"));
        assert!(doc.contains_key("open"));
    }

    #[test]
    fn fragment_uses_literal_strings_for_commands() {
        let frag = fragment(&Spec::builtin());
        // shell command keeps its double quotes via a literal single-quoted string
        assert!(frag.contains(r#"run = 'mdfried %s'"#), "{frag}");
        assert!(
            frag.contains(r#"run = 'lazyenv "$(dirname %s)"'"#),
            "{frag}"
        );
        assert!(!frag.contains("$@") && !frag.contains("$1"), "{frag}");
    }

    #[test]
    fn fragment_preserves_rule_order() {
        let frag = fragment(&Spec::builtin());
        let md = frag.find("\"*.md\"").expect("*.md present");
        let txt = frag.find("\"text/*\"").expect("text/* present");
        assert!(md < txt, "*.md must be rendered before text/*");
    }
}
