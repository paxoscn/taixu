//! Leaderboards with tie-aware ranks, optional top-N trimming and seasons.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, LeaderboardId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreOrder {
    /// Highest score wins (kills, points, ...).
    #[default]
    HigherIsBetter,
    /// Lowest score wins (clear time, deaths, ...).
    LowerIsBetter,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreEntry {
    /// 1-based rank. Tied scores share a rank.
    pub rank: usize,
    pub account: AccountId,
    pub score: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Leaderboard {
    pub id: LeaderboardId,
    pub name: String,
    pub description: String,
    pub order: ScoreOrder,
    /// Optional entry cap. Only the best entries are retained.
    pub max_entries: Option<usize>,
    pub season: u64,
    pub entries: BTreeMap<AccountId, i64>,
    pub updated_at_ms: i64,
}

impl Leaderboard {
    pub fn new(id: impl Into<LeaderboardId>, name: impl Into<String>, order: ScoreOrder) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            order,
            max_entries: None,
            season: 1,
            entries: BTreeMap::new(),
            updated_at_ms: 0,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = Some(max_entries.max(1));
        self
    }

    pub fn score_of(&self, account: &AccountId) -> Option<i64> {
        self.entries.get(account).copied()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn set_score(&mut self, account: AccountId, score: i64, now_ms: i64) {
        self.entries.insert(account, score);
        self.updated_at_ms = now_ms;
        self.trim();
    }

    pub fn add_score(&mut self, account: &AccountId, delta: i64, now_ms: i64) -> Result<i64> {
        let current = self.score_of(account).unwrap_or(0);
        let next = current
            .checked_add(delta)
            .ok_or(GameError::AmountOverflow)?;
        self.set_score(account.clone(), next, now_ms);
        Ok(next)
    }

    pub fn remove(&mut self, account: &AccountId) -> bool {
        self.entries.remove(account).is_some()
    }

    /// 1-based rank, or `None` when the account is not on the board.
    pub fn rank_of(&self, account: &AccountId) -> Option<usize> {
        let score = self.score_of(account)?;
        Some(1 + self.count_better_than(score))
    }

    /// Best entries first, with tie-aware ranks.
    pub fn top(&self, limit: usize) -> Vec<ScoreEntry> {
        let mut entries: Vec<(AccountId, i64)> = self
            .entries
            .iter()
            .map(|(account, score)| (account.clone(), *score))
            .collect();
        self.sort_entries(&mut entries);
        entries
            .into_iter()
            .take(limit)
            .map(|(account, score)| ScoreEntry {
                rank: 1 + self.count_better_than(score),
                account,
                score,
            })
            .collect()
    }

    /// Start a new season, clearing every score.
    pub fn reset_season(&mut self, now_ms: i64) -> u64 {
        self.entries.clear();
        self.season = self.season.saturating_add(1);
        self.updated_at_ms = now_ms;
        self.season
    }

    fn count_better_than(&self, score: i64) -> usize {
        match self.order {
            ScoreOrder::HigherIsBetter => self.entries.values().filter(|s| **s > score).count(),
            ScoreOrder::LowerIsBetter => self.entries.values().filter(|s| **s < score).count(),
        }
    }

    fn sort_entries(&self, entries: &mut [(AccountId, i64)]) {
        match self.order {
            ScoreOrder::HigherIsBetter => {
                entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)))
            }
            ScoreOrder::LowerIsBetter => {
                entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
            }
        }
    }

    fn trim(&mut self) {
        let Some(max) = self.max_entries else {
            return;
        };
        if self.entries.len() <= max {
            return;
        }
        let mut entries: Vec<(AccountId, i64)> = self
            .entries
            .iter()
            .map(|(account, score)| (account.clone(), *score))
            .collect();
        self.sort_entries(&mut entries);
        let keep: std::collections::BTreeSet<AccountId> = entries
            .into_iter()
            .take(max)
            .map(|(account, _)| account)
            .collect();
        self.entries.retain(|account, _| keep.contains(account));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_tie_aware() {
        let mut board = Leaderboard::new("lb", "Board", ScoreOrder::HigherIsBetter);
        for (account, score) in [("a", 100), ("b", 200), ("c", 200), ("d", 50)] {
            board.set_score(account.into(), score, 0);
        }
        assert_eq!(board.rank_of(&"a".into()), Some(3));
        assert_eq!(board.rank_of(&"b".into()), Some(1));
        assert_eq!(board.rank_of(&"c".into()), Some(1));
        let top = board.top(3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].rank, 1);
        assert_eq!(top[1].rank, 1);
        assert_eq!(top[2].rank, 3);
    }

    #[test]
    fn max_entries_keeps_only_the_best() {
        let mut board =
            Leaderboard::new("lb", "Board", ScoreOrder::HigherIsBetter).with_max_entries(2);
        board.set_score("a".into(), 10, 0);
        board.set_score("b".into(), 30, 0);
        board.set_score("c".into(), 20, 0);
        assert_eq!(board.len(), 2);
        assert!(board.entries.contains_key(&"b".into()));
        assert!(board.entries.contains_key(&"c".into()));
    }

    #[test]
    fn lower_is_better_orders_ascending() {
        let mut board = Leaderboard::new("lb", "Speed", ScoreOrder::LowerIsBetter);
        board.set_score("slow".into(), 900, 0);
        board.set_score("fast".into(), 120, 0);
        let top = board.top(2);
        assert_eq!(top[0].account, "fast".into());
        assert_eq!(top[0].rank, 1);
    }
}
