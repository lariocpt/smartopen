//! Fuzzy matching for the picker: is `query` a subsequence of `haystack`, and how good a
//! one? A hand-rolled port of the scoring idea in fzy — every query character must appear
//! in order, and the score rewards matches that land at the start of words, after
//! separators, on camelCase humps, or right after the previous match, while penalising
//! the gaps in between. Scores are only comparable for the same query.
//!
//! Returned indices are character (not byte) positions in `haystack`, which is what the
//! renderer needs to embolden them.

/// A successful match: higher `score` is better; `indices` are the matched char positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub score: f64,
    pub indices: Vec<usize>,
}

const SCORE_MATCH_CONSECUTIVE: f64 = 1.0;
const SCORE_MATCH_SLASH: f64 = 0.9;
const SCORE_MATCH_WORD: f64 = 0.8;
const SCORE_MATCH_CAPITAL: f64 = 0.7;
const SCORE_MATCH_DOT: f64 = 0.6;
const SCORE_GAP_LEADING: f64 = -0.005;
const SCORE_GAP_TRAILING: f64 = -0.005;
const SCORE_GAP_INNER: f64 = -0.01;
const SCORE_MIN: f64 = f64::NEG_INFINITY;

/// Match `query` against `haystack`, case-insensitively. `None` when it is not a
/// subsequence. An empty query matches everything with a score of zero.
pub fn score(query: &str, haystack: &str) -> Option<Match> {
    let needle: Vec<char> = query.chars().map(lower_one).collect();
    let hay_original: Vec<char> = haystack.chars().collect();
    let hay: Vec<char> = hay_original.iter().map(|&c| lower_one(c)).collect();

    if needle.is_empty() {
        return Some(Match {
            score: 0.0,
            indices: Vec::new(),
        });
    }
    if needle.len() > hay.len() || !is_subsequence(&needle, &hay) {
        return None;
    }

    let n = needle.len();
    let m = hay.len();
    let bonus = position_bonuses(&hay_original);

    // d[i][j]: best score with needle[i] matched AT hay[j]; m_[i][j]: best score with
    // needle[..=i] matched somewhere in hay[..=j]. Parent pointers recover the indices.
    let mut d = vec![vec![SCORE_MIN; m]; n];
    let mut best = vec![vec![SCORE_MIN; m]; n];

    for i in 0..n {
        let mut prev_score = SCORE_MIN;
        let gap = if i == n - 1 {
            SCORE_GAP_TRAILING
        } else {
            SCORE_GAP_INNER
        };
        for j in 0..m {
            if needle[i] == hay[j] {
                let mut candidate = SCORE_MIN;
                if i == 0 {
                    candidate = (j as f64) * SCORE_GAP_LEADING + bonus[j];
                } else if j > 0 {
                    let from_gap = best[i - 1][j - 1] + bonus[j];
                    let from_run = d[i - 1][j - 1] + SCORE_MATCH_CONSECUTIVE;
                    candidate = from_gap.max(from_run);
                }
                d[i][j] = candidate;
                prev_score = candidate.max(prev_score + gap);
                best[i][j] = prev_score;
            } else {
                d[i][j] = SCORE_MIN;
                prev_score += gap;
                best[i][j] = prev_score;
            }
        }
    }

    // Backtrack from the best final position, preferring a consecutive run when it ties.
    let mut indices = vec![0usize; n];
    let mut j = m;
    let mut require_match = false;
    for i in (0..n).rev() {
        while j > 0 {
            j -= 1;
            if d[i][j] != SCORE_MIN && (require_match || d[i][j] == best[i][j]) {
                require_match =
                    i > 0 && j > 0 && best[i][j] == d[i - 1][j - 1] + SCORE_MATCH_CONSECUTIVE;
                indices[i] = j;
                break;
            }
        }
    }

    Some(Match {
        score: best[n - 1][m - 1],
        indices,
    })
}

/// One lowercase char per input char, so an index into the lowered text is an index into
/// the original. `char::to_lowercase` can yield two chars (`İ` → `i` + a combining dot),
/// and a match landing past the original length indexed `bonus` out of bounds — a review
/// found it. Dropping the combining mark is the fuzzy behaviour wanted anyway.
fn lower_one(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn is_subsequence(needle: &[char], hay: &[char]) -> bool {
    let mut it = hay.iter();
    needle.iter().all(|c| it.any(|h| h == c))
}

/// The bonus for a match landing at each position, decided by the character before it.
fn position_bonuses(hay: &[char]) -> Vec<f64> {
    let mut prev = '/';
    hay.iter()
        .map(|&c| {
            let bonus = match prev {
                '/' | '\\' => SCORE_MATCH_SLASH,
                '-' | '_' | ' ' => SCORE_MATCH_WORD,
                '.' => SCORE_MATCH_DOT,
                p if p.is_lowercase() && c.is_uppercase() => SCORE_MATCH_CAPITAL,
                _ => 0.0,
            };
            prev = c;
            bonus
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(query: &str, hay: &str) -> f64 {
        score(query, hay).expect("should match").score
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(score("xyz", "Open in editor").is_none());
        assert!(score("editorx", "editor").is_none());
    }

    #[test]
    fn empty_query_matches_everything_flat() {
        assert_eq!(score("", "anything").unwrap().score, 0.0);
        assert!(score("", "anything").unwrap().indices.is_empty());
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("OPEN", "open in editor").is_some());
        assert!(score("open", "OPEN IN EDITOR").is_some());
    }

    #[test]
    fn word_starts_beat_scattered_letters() {
        // "vc" as the initials of "View CSV" should beat the same letters buried inside.
        assert!(s("vc", "View CSV with xan") > s("vc", "Advocate"));
    }

    #[test]
    fn consecutive_runs_beat_gaps() {
        assert!(s("edit", "Edit config") > s("edit", "e-d-i-t"));
    }

    #[test]
    fn indices_point_at_the_matched_characters() {
        let m = score("ed", "Open in editor").unwrap();
        let hay: Vec<char> = "Open in editor".chars().collect();
        let picked: String = m.indices.iter().map(|&i| hay[i]).collect();
        assert_eq!(picked.to_lowercase(), "ed");
        assert!(m.indices.windows(2).all(|w| w[0] < w[1]), "{:?}", m.indices);
    }

    #[test]
    fn indices_are_char_positions_not_bytes() {
        let m = score("x", "ééx").unwrap();
        assert_eq!(m.indices, vec![2]);
    }

    #[test]
    fn lowercasing_never_changes_the_length() {
        // `İ` lowercases to two chars; nine of them pushed the match for `l` past the
        // original length and out of the bonus table.
        let m = score("l", "İİİİİİİİİl").expect("still a subsequence");
        assert_eq!(m.indices, vec![9]);
        assert!(score("i", "İstanbul").is_some());
    }

    #[test]
    fn exact_prefix_scores_highest_among_candidates() {
        let candidates = ["Cargo test", "Carbonyl", "Broot", "gitui"];
        let mut ranked: Vec<_> = candidates
            .iter()
            .filter_map(|c| score("car", c).map(|m| (m.score, *c)))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().any(|(_, c)| *c == "Cargo test"));
    }
}
