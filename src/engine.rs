//! Open strategy: turn the base [`Spec`] into the concrete spec to render.
//!
//! `Rules` (`--rules`) passes the spec through unchanged — explicit per-file-type openers.
//! `Smartopen` (the default) replaces them with one rule that hands everything, directories
//! included, to the `smartopen` binary: its `[[folder]]` and file associations decide from
//! there. Nothing sits in front of the delegate. An earlier version listed `carbonyl`
//! ahead of it for images and video, and Enter on a PNG in yazi ran carbonyl while the
//! menu never appeared — the review caught the README promising otherwise.

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
    let smart = OpenerDef {
        name: "smartopen".to_string(),
        doc: Some(format!("Delegate opening to the `{bin}` menu.")),
        runs: vec![OpenerRun {
            run: format!(r#"{bin} "$@""#),
            desc: Some("smartopen".to_string()),
            block: true,
            orphan: false,
            // The delegate is this cross-platform binary: no `for`, or yazi would drop
            // the rule on every platform it did not name.
            for_platform: None,
        }],
    };
    let rules = vec![OpenRule {
        matcher: Matcher::Mime("*".to_string()),
        use_openers: vec!["smartopen".to_string()],
        doc: Some(
            "everything, directories included -> smartopen (it runs its own menu)".to_string(),
        ),
    }];
    Spec {
        openers: vec![smart],
        prepend_rules: rules,
    }
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
    fn smartopen_engine_is_the_delegate_and_nothing_in_front_of_it() {
        let s = effective(&Spec::builtin(), Engine::Smartopen, "smartopen");
        assert_eq!(s.openers.len(), 1);
        assert_eq!(s.prepend_rules.len(), 1);
        assert_eq!(s.openers[0].name, "smartopen");
        assert_eq!(s.prepend_rules[0].matcher, Matcher::Mime("*".to_string()));
        assert_eq!(
            s.prepend_rules[0].use_openers,
            vec!["smartopen".to_string()]
        );
        s.validate().expect("smartopen spec must validate");
    }

    #[test]
    fn every_smartopen_rule_has_an_opener_for_each_platform() {
        // yazi drops an opener whose `for` does not match the OS, and a rule left with
        // none is dead there: `yazi apply` on Windows used to produce exactly that. The
        // delegate is this cross-platform binary, so it carries no `for` at all.
        let s = effective(&Spec::builtin(), Engine::Smartopen, "smartopen");
        for platform in ["linux", "macos", "windows"] {
            for rule in &s.prepend_rules {
                let runnable = rule.use_openers.iter().any(|name| {
                    s.openers.iter().find(|o| &o.name == name).is_some_and(|o| {
                        o.runs.iter().any(|r| match r.for_platform.as_deref() {
                            None => true,
                            Some("unix") => platform != "windows",
                            Some(p) => p == platform,
                        })
                    })
                });
                assert!(
                    runnable,
                    "{platform}: rule {:?} has no runnable opener",
                    rule.matcher
                );
            }
        }
    }

    #[test]
    fn smartopen_bin_is_configurable() {
        let s = effective(
            &Spec::builtin(),
            Engine::Smartopen,
            "/usr/local/bin/smartopen",
        );
        let smart = s.openers.iter().find(|o| o.name == "smartopen").unwrap();
        assert!(smart.runs[0].run.starts_with("/usr/local/bin/smartopen "));
    }
}
