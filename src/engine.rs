//! Open strategy: turn the base [`Spec`] into the concrete spec to render.
//!
//! `Rules` (default) passes the spec through unchanged — explicit per-file-type openers.
//! `Smartopen` replaces them with a single universal rule that delegates every file to the
//! `smartopen` binary (the sibling app), keeping only the directory -> terminal rule.

use crate::spec::{Matcher, OpenRule, OpenerDef, OpenerRun, Spec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Rules,
    Smartopen,
}

/// Produce the spec to render for the chosen engine. Pure transform, no I/O.
pub fn effective(base: &Spec, engine: Engine, smartopen_bin: &str) -> Spec {
    match engine {
        Engine::Rules => base.clone(),
        Engine::Smartopen => smartopen_spec(smartopen_bin),
    }
}

fn smartopen_spec(bin: &str) -> Spec {
    let terminal = OpenerDef {
        name: "terminal".to_string(),
        doc: Some("Open a folder in a new ghostty window.".to_string()),
        runs: vec![OpenerRun {
            run: r#"ghostty --working-directory "$1" >/dev/null 2>&1"#.to_string(),
            desc: Some("Open in new ghostty".to_string()),
            block: false,
            orphan: true,
            for_platform: Some("linux".to_string()),
        }],
    };
    let smart = OpenerDef {
        name: "smartopen".to_string(),
        doc: Some(format!("Delegate file opening to the `{bin}` smart opener.")),
        runs: vec![OpenerRun {
            run: format!(r#"{bin} "$@""#),
            desc: Some("smartopen".to_string()),
            block: true,
            orphan: false,
            for_platform: Some("unix".to_string()),
        }],
    };
    let rules = vec![
        OpenRule {
            matcher: Matcher::Mime("inode/directory".to_string()),
            use_openers: vec!["terminal".to_string()],
            doc: Some("directories still open a terminal".to_string()),
        },
        OpenRule {
            matcher: Matcher::Mime("*".to_string()),
            use_openers: vec!["smartopen".to_string()],
            doc: Some("everything else -> smartopen (it runs its own open-with menu)".to_string()),
        },
    ];
    Spec { openers: vec![terminal, smart], prepend_rules: rules }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_engine_is_identity() {
        let base = Spec::builtin();
        assert_eq!(effective(&base, Engine::Rules, "smartopen"), base);
    }

    #[test]
    fn smartopen_engine_shape() {
        let s = effective(&Spec::builtin(), Engine::Smartopen, "smartopen");
        assert_eq!(s.openers.len(), 2);
        assert_eq!(s.prepend_rules.len(), 2);
        assert!(s.openers.iter().any(|o| o.name == "smartopen"));
        s.validate().expect("smartopen spec must validate");
    }

    #[test]
    fn smartopen_bin_is_configurable() {
        let s = effective(&Spec::builtin(), Engine::Smartopen, "/usr/local/bin/smartopen");
        let smart = s.openers.iter().find(|o| o.name == "smartopen").unwrap();
        assert!(smart.runs[0].run.starts_with("/usr/local/bin/smartopen "));
    }
}
