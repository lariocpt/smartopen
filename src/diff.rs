//! Unified diff between the current file and the would-be result (for `diff`/`--dry-run`).

use similar::TextDiff;

/// A unified diff of `old` -> `new`. Empty string when they are identical.
pub fn unified(old: &str, new: &str) -> String {
    if old == new {
        return String::new();
    }
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .header("a/yazi.toml", "b/yazi.toml")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_yields_empty() {
        assert!(unified("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn change_is_reported() {
        let d = unified("a\nb\n", "a\nc\n");
        assert!(d.contains("-b"));
        assert!(d.contains("+c"));
    }
}
