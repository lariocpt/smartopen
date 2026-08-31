//! What you picked before, so it floats to the top next time.
//!
//! A small TOML file under the state directory records, per command label, how many
//! times it was run and when it was last run. When the picker's query is empty, entries
//! are ordered by frecency — recent AND frequent — with the config's own order as the
//! tiebreak. Typing a query switches to fuzzy order and history stops mattering.
//!
//! Nothing here is ever fatal: an unreadable or corrupt file is treated as empty, and a
//! failed save is reported on stderr once and otherwise ignored. A launcher that refuses
//! to launch because its bookkeeping file is odd would have its priorities backwards.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::tomlio;

const SECONDS_PER_DAY: f64 = 86_400.0;
/// Frecency half-life: a pick from a week ago counts half as much as one from now.
const HALF_LIFE_DAYS: f64 = 7.0;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub count: u32,
    /// Unix seconds.
    pub last_used: u64,
}

impl History {
    /// An empty history that never saves — `--no-history`.
    pub fn disabled() -> History {
        History::default()
    }

    /// Load from `path`; missing or unparsable is simply empty.
    pub fn load(path: PathBuf) -> History {
        let mut history: History = fs::read_to_string(&path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default();
        history.path = Some(path);
        history
    }

    /// How strongly `label` should float up when the query is empty. Zero when unseen.
    pub fn frecency(&self, label: &str) -> f64 {
        self.frecency_at(label, now())
    }

    fn frecency_at(&self, label: &str, now_secs: u64) -> f64 {
        let Some(entry) = self.entries.get(&key(label)) else {
            return 0.0;
        };
        let age_days = now_secs.saturating_sub(entry.last_used) as f64 / SECONDS_PER_DAY;
        f64::from(entry.count) * 0.5f64.powf(age_days / HALF_LIFE_DAYS)
    }

    /// Note that `label` was just run. Persists immediately when backed by a file.
    pub fn record(&mut self, label: &str) {
        let entry = self.entries.entry(key(label)).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_used = now();

        if let Some(path) = &self.path
            && let Err(error) = self.save_to(path)
        {
            eprintln!(
                "smartopen: could not save history to {}: {error:#}",
                path.display()
            );
        }
    }

    fn save_to(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = toml::to_string(self)?;
        tomlio::atomic_write(path, &body)
    }
}

/// Labels are matched case-insensitively everywhere else, so the store agrees.
fn key(label: &str) -> String {
    label.trim().to_lowercase()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(label: &str, count: u32, last_used: u64) -> History {
        let mut history = History::default();
        history
            .entries
            .insert(key(label), Entry { count, last_used });
        history
    }

    #[test]
    fn unseen_labels_have_no_frecency() {
        assert_eq!(History::default().frecency("anything"), 0.0);
    }

    #[test]
    fn frecency_decays_with_a_one_week_half_life() {
        let now_secs = 1_000_000_000;
        let fresh = with("Edit", 4, now_secs);
        let week_old = with("Edit", 4, now_secs - 7 * 86_400);
        assert_eq!(fresh.frecency_at("Edit", now_secs), 4.0);
        assert!((week_old.frecency_at("Edit", now_secs) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn labels_are_case_and_whitespace_insensitive() {
        let history = with("Edit", 1, 1_000_000_000);
        assert!(history.frecency_at(" edit ", 1_000_000_000) > 0.0);
        assert!(history.frecency_at("EDIT", 1_000_000_000) > 0.0);
    }

    #[test]
    fn record_round_trips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("smartopen-history-{}", std::process::id()));
        let path = dir.join("state.toml");
        let _ = fs::remove_dir_all(&dir);

        let mut history = History::load(path.clone());
        history.record("Cargo test");
        history.record("Cargo test");
        history.record("Broot");

        let reloaded = History::load(path.clone());
        assert_eq!(reloaded.entries[&key("Cargo test")].count, 2);
        assert_eq!(reloaded.entries[&key("Broot")].count, 1);
        assert!(reloaded.frecency("Cargo test") > reloaded.frecency("Broot"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_is_treated_as_empty() {
        let dir =
            std::env::temp_dir().join(format!("smartopen-history-bad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.toml");
        fs::write(&path, "this is not = [toml").unwrap();

        let history = History::load(path);
        assert_eq!(history.frecency("anything"), 0.0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_history_records_without_writing() {
        let mut history = History::disabled();
        history.record("Edit");
        assert!(history.frecency("Edit") > 0.0);
        assert!(history.path.is_none());
    }
}
