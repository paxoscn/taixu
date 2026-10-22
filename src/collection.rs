//! Collection albums ("collect X distinct items / skins / achievements").
//!
//! Progress is derived from live account state — an item counts while it is in
//! the inventory, a skin while it is owned, an achievement while completed.
//! Milestone rewards are claimed explicitly and can only be claimed once.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, AchievementId, CollectionId, ItemId, SkinId};
use crate::reward::Reward;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Collectible {
    Item(ItemId),
    Skin(SkinId),
    Achievement(AchievementId),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionMilestone {
    /// Number of distinct collection entries required.
    pub required: usize,
    pub reward: Reward,
}

impl CollectionMilestone {
    pub fn new(required: usize, reward: Reward) -> Self {
        Self { required, reward }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionDef {
    pub id: CollectionId,
    pub name: String,
    pub description: String,
    pub entries: Vec<Collectible>,
    pub milestones: Vec<CollectionMilestone>,
}

impl CollectionDef {
    pub fn new(id: impl Into<CollectionId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            entries: Vec::new(),
            milestones: Vec::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_entries(mut self, entries: impl IntoIterator<Item = Collectible>) -> Self {
        self.entries = entries.into_iter().collect();
        self
    }

    pub fn with_milestones(
        mut self,
        milestones: impl IntoIterator<Item = CollectionMilestone>,
    ) -> Self {
        self.milestones = milestones.into_iter().collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneProgress {
    pub index: usize,
    pub required: usize,
    pub unlocked: bool,
    pub claimed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionProgress {
    pub collection: CollectionId,
    pub collected: usize,
    pub total: usize,
    pub milestones: Vec<MilestoneProgress>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CollectionBook {
    defs: BTreeMap<CollectionId, CollectionDef>,
    /// account -> claimed (collection, milestone index) pairs
    claims: BTreeMap<AccountId, BTreeSet<(CollectionId, usize)>>,
}

impl CollectionBook {
    pub fn register(&mut self, def: CollectionDef) -> Result<()> {
        if def.name.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "collection name must not be empty".into(),
            ));
        }
        if def.entries.is_empty() {
            return Err(GameError::InvalidInput(
                "collection must contain at least one entry".into(),
            ));
        }
        if def.milestones.is_empty() {
            return Err(GameError::InvalidInput(
                "collection must contain at least one milestone".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for entry in &def.entries {
            if !seen.insert(entry.clone()) {
                return Err(GameError::InvalidInput(
                    "collection contains duplicate entries".into(),
                ));
            }
        }
        let mut previous = 0usize;
        for milestone in &def.milestones {
            if milestone.required == 0 || milestone.required <= previous {
                return Err(GameError::InvalidInput(
                    "milestone requirements must be positive and strictly increasing".into(),
                ));
            }
            if milestone.required > def.entries.len() {
                return Err(GameError::InvalidInput(
                    "milestone requirement exceeds number of entries".into(),
                ));
            }
            previous = milestone.required;
        }
        if self.defs.contains_key(&def.id) {
            return Err(GameError::CollectionAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn get(&self, id: &CollectionId) -> Option<&CollectionDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &CollectionId) -> Result<&CollectionDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::CollectionNotFound(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &CollectionDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    pub fn is_claimed(
        &self,
        account: &AccountId,
        collection: &CollectionId,
        milestone: usize,
    ) -> bool {
        self.claims
            .get(account)
            .is_some_and(|claims| claims.contains(&(collection.clone(), milestone)))
    }

    pub fn mark_claimed(
        &mut self,
        account: &AccountId,
        collection: &CollectionId,
        milestone: usize,
    ) {
        self.claims
            .entry(account.clone())
            .or_default()
            .insert((collection.clone(), milestone));
    }

    pub fn claimed_count(&self, account: &AccountId, collection: &CollectionId) -> usize {
        self.claims
            .get(account)
            .map(|claims| {
                claims
                    .iter()
                    .filter(|(claimed_collection, _)| claimed_collection == collection)
                    .count()
            })
            .unwrap_or(0)
    }
}

impl CollectionDef {
    pub fn milestone(&self, index: usize) -> Result<&CollectionMilestone> {
        self.milestones
            .get(index)
            .ok_or(GameError::MilestoneNotFound(index))
    }
}
