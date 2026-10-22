//! Task / quest system.
//!
//! Tasks are optional objectives attached to an account:
//!
//! * **accept** a task when its level / region / prerequisite / time-window
//!   requirements are met,
//! * **progress** metric objectives through [`crate::Game::record_event`]
//!   (item, skin and achievement objectives are evaluated from live state),
//! * **claim** the reward once every objective is complete,
//! * optionally **abandon** a task and retry later.
//!
//! Tasks can be one-shot, have a time limit, and repeat on a cooldown
//! (including 24-hour / 7-day helpers). Completed-but-unclaimed tasks keep
//! their completion even if the required items are later consumed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, AchievementId, ItemId, RegionId, SkinId, TaskId};
use crate::reward::Reward;

/// A single completion condition.
///
/// Metric objectives progress through `Game::record_event`; the other variants
/// are evaluated from live inventory / cosmetics / achievements.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskObjective {
    /// Advance a named counter, e.g. `kills`, `boss_clears`.
    Metric { metric: String, target: u64 },
    /// Hold at least `quantity` of an item in the inventory.
    Item { item: ItemId, quantity: u64 },
    /// Own a cosmetic skin.
    Skin { skin: SkinId },
    /// Complete an achievement.
    Achievement { achievement: AchievementId },
}

impl TaskObjective {
    pub fn metric(metric: impl Into<String>, target: u64) -> Self {
        TaskObjective::Metric {
            metric: metric.into(),
            target,
        }
    }

    pub fn item(item: impl Into<ItemId>, quantity: u64) -> Self {
        TaskObjective::Item {
            item: item.into(),
            quantity,
        }
    }

    pub fn skin(skin: impl Into<SkinId>) -> Self {
        TaskObjective::Skin { skin: skin.into() }
    }

    pub fn achievement(achievement: impl Into<AchievementId>) -> Self {
        TaskObjective::Achievement {
            achievement: achievement.into(),
        }
    }

    pub fn target(&self) -> u64 {
        match self {
            TaskObjective::Metric { target, .. } => *target,
            TaskObjective::Item { quantity, .. } => *quantity,
            TaskObjective::Skin { .. } | TaskObjective::Achievement { .. } => 1,
        }
    }

    pub fn metric_key(&self) -> Option<&str> {
        match self {
            TaskObjective::Metric { metric, .. } => Some(metric),
            _ => None,
        }
    }

    pub fn is_complete(&self, progress: u64) -> bool {
        progress >= self.target()
    }
}

/// How a task may be repeated after claiming its reward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskRepeat {
    #[default]
    Once,
    /// Available again once this duration has elapsed since the last claim.
    Cooldown { duration_ms: i64 },
}

const DAY_MS: i64 = 86_400_000;

impl TaskRepeat {
    pub fn cooldown(duration_ms: i64) -> Self {
        TaskRepeat::Cooldown { duration_ms }
    }

    /// Repeat 24 hours after the previous claim.
    pub fn daily() -> Self {
        TaskRepeat::Cooldown {
            duration_ms: DAY_MS,
        }
    }

    /// Repeat 7 days after the previous claim.
    pub fn weekly() -> Self {
        TaskRepeat::Cooldown {
            duration_ms: 7 * DAY_MS,
        }
    }

    pub fn is_repeatable(&self) -> bool {
        matches!(self, TaskRepeat::Cooldown { .. })
    }

    pub fn cooldown_ms(&self) -> Option<i64> {
        match self {
            TaskRepeat::Once => None,
            TaskRepeat::Cooldown { duration_ms } => Some(*duration_ms),
        }
    }

    pub fn next_available_at(&self, claimed_at_ms: i64) -> Option<i64> {
        self.cooldown_ms()
            .map(|duration| claimed_at_ms.saturating_add(duration))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDef {
    pub id: TaskId,
    pub name: String,
    pub description: String,
    /// Free-form grouping used by clients (`main`, `daily`, `event`, ...).
    pub category: String,
    pub objectives: Vec<TaskObjective>,
    pub reward: Reward,
    /// Tasks that must have been completed before this one unlocks.
    pub requires: Vec<TaskId>,
    pub min_level: Option<u32>,
    pub repeat: TaskRepeat,
    /// Deadline measured from acceptance. `None` means no deadline.
    pub time_limit_ms: Option<i64>,
    pub starts_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
    /// Restrict availability to these regions. `None` means all regions.
    pub regions: Option<BTreeSet<RegionId>>,
    /// Hidden tasks are omitted from list endpoints until they have progress.
    pub hidden: bool,
    pub tags: BTreeSet<String>,
}

impl TaskDef {
    pub fn new(id: impl Into<TaskId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            category: "general".into(),
            objectives: Vec::new(),
            reward: Reward::new(),
            requires: Vec::new(),
            min_level: None,
            repeat: TaskRepeat::Once,
            time_limit_ms: None,
            starts_at_ms: None,
            ends_at_ms: None,
            regions: None,
            hidden: false,
            tags: BTreeSet::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    pub fn with_objective(mut self, objective: TaskObjective) -> Self {
        self.objectives.push(objective);
        self
    }

    pub fn with_objectives(mut self, objectives: impl IntoIterator<Item = TaskObjective>) -> Self {
        self.objectives.extend(objectives);
        self
    }

    pub fn with_reward(mut self, reward: Reward) -> Self {
        self.reward = reward;
        self
    }

    pub fn with_prerequisite(mut self, task: impl Into<TaskId>) -> Self {
        self.requires.push(task.into());
        self
    }

    pub fn with_min_level(mut self, level: u32) -> Self {
        self.min_level = Some(level);
        self
    }

    pub fn with_repeat(mut self, repeat: TaskRepeat) -> Self {
        self.repeat = repeat;
        self
    }

    pub fn with_time_limit_ms(mut self, time_limit_ms: i64) -> Self {
        self.time_limit_ms = Some(time_limit_ms);
        self
    }

    pub fn with_window(mut self, starts_at_ms: Option<i64>, ends_at_ms: Option<i64>) -> Self {
        self.starts_at_ms = starts_at_ms;
        self.ends_at_ms = ends_at_ms;
        self
    }

    pub fn available_in(mut self, regions: impl IntoIterator<Item = RegionId>) -> Self {
        self.regions = Some(regions.into_iter().collect());
        self
    }

    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.insert(tag.into());
        self
    }
}

/// Lifecycle state of a task for one account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Requirements met, not accepted yet.
    Available,
    /// Requirements not met (level, region, prerequisite, time window).
    Locked,
    /// Accepted and in progress.
    Active,
    /// All objectives satisfied; reward not claimed yet.
    Completed,
    /// Reward claimed. Repeatable tasks become available again later.
    Claimed,
    /// Deadline passed before completion; may be accepted again.
    Expired,
}

/// Persisted per-account progress for one task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskProgress {
    pub accepted_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    /// Metric counters for this acceptance cycle.
    pub counters: BTreeMap<String, u64>,
    pub completed_at_ms: Option<i64>,
    pub claimed_at_ms: Option<i64>,
}

impl TaskProgress {
    pub fn new(accepted_at_ms: i64, expires_at_ms: Option<i64>) -> Self {
        Self {
            accepted_at_ms,
            expires_at_ms,
            counters: BTreeMap::new(),
            completed_at_ms: None,
            claimed_at_ms: None,
        }
    }

    pub fn counter(&self, metric: &str) -> u64 {
        self.counters.get(metric).copied().unwrap_or(0)
    }

    pub fn is_completed(&self) -> bool {
        self.completed_at_ms.is_some()
    }

    pub fn is_claimed(&self) -> bool {
        self.claimed_at_ms.is_some()
    }

    pub fn is_expired(&self, now_ms: i64) -> bool {
        self.expires_at_ms
            .is_some_and(|deadline| now_ms >= deadline)
    }

    /// When a claimed repeatable task becomes available again.
    pub fn next_available_at(&self, repeat: TaskRepeat) -> Option<i64> {
        self.claimed_at_ms
            .and_then(|claimed_at| repeat.next_available_at(claimed_at))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskObjectiveView {
    pub objective: TaskObjective,
    pub progress: u64,
    pub target: u64,
    pub complete: bool,
}

/// Client-facing snapshot of one task for an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskView {
    pub id: TaskId,
    pub name: String,
    pub description: String,
    pub category: String,
    pub status: TaskStatus,
    pub objectives: Vec<TaskObjectiveView>,
    pub reward: Reward,
    pub accepted_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub claimed_at_ms: Option<i64>,
    pub next_available_at_ms: Option<i64>,
    pub repeat: TaskRepeat,
    pub min_level: Option<u32>,
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskBook {
    defs: BTreeMap<TaskId, TaskDef>,
    accounts: BTreeMap<AccountId, BTreeMap<TaskId, TaskProgress>>,
}

impl TaskBook {
    pub fn register(&mut self, def: TaskDef) -> Result<()> {
        if def.name.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "task name must not be empty".into(),
            ));
        }
        if def.category.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "task category must not be empty".into(),
            ));
        }
        if def.objectives.is_empty() {
            return Err(GameError::InvalidInput(
                "task must contain at least one objective".into(),
            ));
        }
        for objective in &def.objectives {
            match objective {
                TaskObjective::Metric { metric, target } => {
                    if metric.trim().is_empty() {
                        return Err(GameError::InvalidInput(
                            "task metric must not be empty".into(),
                        ));
                    }
                    if *target == 0 {
                        return Err(GameError::InvalidInput(
                            "task metric target must be greater than zero".into(),
                        ));
                    }
                }
                TaskObjective::Item { quantity, .. } => {
                    if *quantity == 0 {
                        return Err(GameError::InvalidInput(
                            "task item quantity must be greater than zero".into(),
                        ));
                    }
                }
                TaskObjective::Skin { .. } | TaskObjective::Achievement { .. } => {}
            }
        }
        let mut prerequisites = BTreeSet::new();
        for prerequisite in &def.requires {
            if *prerequisite == def.id {
                return Err(GameError::InvalidInput(
                    "a task cannot require itself".into(),
                ));
            }
            if !prerequisites.insert(prerequisite.clone()) {
                return Err(GameError::InvalidInput(
                    "task prerequisites must be unique".into(),
                ));
            }
        }
        if let Some(limit) = def.time_limit_ms {
            if limit <= 0 {
                return Err(GameError::InvalidInput(
                    "task time limit must be greater than zero".into(),
                ));
            }
        }
        if let Some(duration) = def.repeat.cooldown_ms() {
            if duration <= 0 {
                return Err(GameError::InvalidInput(
                    "task cooldown must be greater than zero".into(),
                ));
            }
        }
        if let (Some(start), Some(end)) = (def.starts_at_ms, def.ends_at_ms) {
            if end <= start {
                return Err(GameError::InvalidInput(
                    "task availability window is empty".into(),
                ));
            }
        }
        if self.defs.contains_key(&def.id) {
            return Err(GameError::TaskAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn get(&self, id: &TaskId) -> Option<&TaskDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &TaskId) -> Result<&TaskDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::TaskNotFound(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &TaskDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn progress(&self, account: &AccountId, task: &TaskId) -> Option<&TaskProgress> {
        self.accounts.get(account).and_then(|tasks| tasks.get(task))
    }

    pub fn account_progress(&self, account: &AccountId) -> Vec<(TaskId, TaskProgress)> {
        self.accounts
            .get(account)
            .map(|tasks| {
                tasks
                    .iter()
                    .map(|(id, progress)| (id.clone(), progress.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_progress(&mut self, account: &AccountId, task: TaskId, progress: TaskProgress) {
        self.accounts
            .entry(account.clone())
            .or_default()
            .insert(task, progress);
    }

    pub(crate) fn progress_mut(
        &mut self,
        account: &AccountId,
        task: &TaskId,
    ) -> Option<&mut TaskProgress> {
        self.accounts.get_mut(account)?.get_mut(task)
    }

    pub fn remove_progress(&mut self, account: &AccountId, task: &TaskId) -> Option<TaskProgress> {
        let (removed, empty) = {
            let tasks = self.accounts.get_mut(account)?;
            let removed = tasks.remove(task);
            (removed, tasks.is_empty())
        };
        if empty {
            self.accounts.remove(account);
        }
        removed
    }

    /// True when the task has ever been completed or claimed for the account.
    pub fn is_completed(&self, account: &AccountId, task: &TaskId) -> bool {
        self.progress(account, task)
            .is_some_and(|progress| progress.is_completed() || progress.is_claimed())
    }

    pub fn progress_count(&self, account: &AccountId) -> usize {
        self.accounts.get(account).map_or(0, BTreeMap::len)
    }
}
