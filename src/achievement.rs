//! Metric-driven achievements with progress tracking and claimable rewards.
//!
//! Achievements are unlocked when a named counter reaches its target. The
//! host game feeds counters through [`crate::Game::record_metric`], e.g.
//! `record_metric(account, "kills", 1)` after a combat event.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, AchievementId};
use crate::reward::Reward;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementDef {
    pub id: AchievementId,
    pub name: String,
    pub description: String,
    /// Counter that drives this achievement (`kills`, `rooms_created`, ...).
    pub metric: String,
    pub target: u64,
    pub reward: Reward,
    /// Hidden achievements can be omitted from client lists until unlocked.
    pub hidden: bool,
}

impl AchievementDef {
    pub fn new(
        id: impl Into<AchievementId>,
        name: impl Into<String>,
        metric: impl Into<String>,
        target: u64,
        reward: Reward,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            metric: metric.into(),
            target,
            reward,
            hidden: false,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountAchievements {
    pub counters: BTreeMap<String, u64>,
    pub completed: BTreeSet<AchievementId>,
    pub claimed: BTreeSet<AchievementId>,
}

impl AccountAchievements {
    pub fn counter(&self, metric: &str) -> u64 {
        self.counters.get(metric).copied().unwrap_or(0)
    }

    pub fn is_completed(&self, achievement: &AchievementId) -> bool {
        self.completed.contains(achievement)
    }

    pub fn is_claimed(&self, achievement: &AchievementId) -> bool {
        self.claimed.contains(achievement)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AchievementBook {
    defs: BTreeMap<AchievementId, AchievementDef>,
    accounts: BTreeMap<AccountId, AccountAchievements>,
}

impl AchievementBook {
    pub fn register(&mut self, def: AchievementDef) -> Result<()> {
        if def.name.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "achievement name must not be empty".into(),
            ));
        }
        if def.metric.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "achievement metric must not be empty".into(),
            ));
        }
        if def.target == 0 {
            return Err(GameError::InvalidInput(
                "achievement target must be greater than zero".into(),
            ));
        }
        if self.defs.contains_key(&def.id) {
            return Err(GameError::AchievementAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn get(&self, id: &AchievementId) -> Option<&AchievementDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &AchievementId) -> Result<&AchievementDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::AchievementNotFound(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &AchievementDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn account(&self, account: &AccountId) -> Option<&AccountAchievements> {
        self.accounts.get(account)
    }

    pub fn account_mut(&mut self, account: &AccountId) -> &mut AccountAchievements {
        self.accounts.entry(account.clone()).or_default()
    }

    /// Increment a metric counter and return achievements newly completed.
    pub fn record_metric(
        &mut self,
        account: &AccountId,
        metric: &str,
        amount: u64,
    ) -> Vec<AchievementId> {
        if amount == 0 {
            return Vec::new();
        }
        let value = {
            let progress = self.account_mut(account);
            let counter = progress.counters.entry(metric.to_owned()).or_insert(0);
            *counter = counter.saturating_add(amount);
            *counter
        };

        let newly_completed: Vec<AchievementId> = self
            .defs
            .values()
            .filter(|def| {
                def.metric == metric
                    && def.target <= value
                    && !self
                        .accounts
                        .get(account)
                        .is_some_and(|progress| progress.completed.contains(&def.id))
            })
            .map(|def| def.id.clone())
            .collect();
        if !newly_completed.is_empty() {
            let progress = self.account_mut(account);
            for id in &newly_completed {
                progress.completed.insert(id.clone());
            }
        }
        newly_completed
    }

    pub fn progress_of(&self, account: &AccountId, def: &AchievementDef) -> u64 {
        self.accounts
            .get(account)
            .map(|progress| progress.counter(&def.metric).min(def.target))
            .unwrap_or(0)
    }

    pub fn is_completed(&self, account: &AccountId, id: &AchievementId) -> bool {
        self.accounts
            .get(account)
            .is_some_and(|progress| progress.is_completed(id))
    }

    pub fn is_claimed(&self, account: &AccountId, id: &AchievementId) -> bool {
        self.accounts
            .get(account)
            .is_some_and(|progress| progress.is_claimed(id))
    }

    pub fn mark_claimed(&mut self, account: &AccountId, id: &AchievementId) {
        self.account_mut(account).claimed.insert(id.clone());
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AchievementView {
    pub id: AchievementId,
    pub name: String,
    pub description: String,
    pub metric: String,
    pub progress: u64,
    pub target: u64,
    pub completed: bool,
    pub claimed: bool,
    pub hidden: bool,
}
