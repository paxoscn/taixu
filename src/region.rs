//! Regions represent physical server nodes / shards.
//!
//! An account belongs to exactly one region. Rooms, shops and announcements
//! may be scoped to regions. Moving regions is handled by
//! [`crate::Game::set_account_region`], which keeps room membership and
//! capacity accounting consistent.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, RegionId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionStatus {
    #[default]
    Open,
    /// Existing players stay active, but no new accounts can be assigned and
    /// no accounts can move into the region.
    Maintenance,
    /// New entry is blocked, and existing players cannot start new player
    /// activity: rooms, chat, purchases, trades, friend requests and task
    /// acceptance are rejected with [`crate::GameError::RegionNotOpen`].
    ///
    /// Read-only queries, reward claims, abandoning trades/tasks, and profile
    /// or cosmetic edits remain available.
    Closed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Region {
    /// Stable region / shard id.
    pub id: RegionId,
    /// Display name.
    pub name: String,
    /// Connect endpoint advertised to clients (`wss://...`, host:port, ...).
    pub endpoint: Option<String>,
    /// Maximum number of assigned accounts. `None` is unbounded.
    pub capacity: Option<usize>,
    /// Availability and activity gate; see [`RegionStatus`].
    pub status: RegionStatus,
    /// Accounts currently assigned to this region (not just online accounts).
    pub players: BTreeSet<AccountId>,
    /// Free-form shard metadata (build, datacenter, feature flags, ...).
    pub metadata: BTreeMap<String, String>,
    /// Region creation time (Unix ms).
    pub created_at_ms: i64,
}

impl Region {
    pub fn new(
        id: RegionId,
        name: impl Into<String>,
        capacity: Option<usize>,
        endpoint: Option<String>,
        now_ms: i64,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            endpoint,
            capacity,
            status: RegionStatus::Open,
            players: BTreeSet::new(),
            metadata: BTreeMap::new(),
            created_at_ms: now_ms,
        }
    }

    pub fn is_open(&self) -> bool {
        self.status == RegionStatus::Open
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub fn is_full(&self) -> bool {
        self.capacity
            .is_some_and(|capacity| self.players.len() >= capacity)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RegionRegistry {
    regions: BTreeMap<RegionId, Region>,
}

impl RegionRegistry {
    pub fn register(&mut self, region: Region) -> Result<()> {
        if self.regions.contains_key(&region.id) {
            return Err(GameError::RegionAlreadyExists(region.id));
        }
        self.regions.insert(region.id.clone(), region);
        Ok(())
    }

    pub fn get(&self, id: &RegionId) -> Option<&Region> {
        self.regions.get(id)
    }

    pub fn get_mut(&mut self, id: &RegionId) -> Option<&mut Region> {
        self.regions.get_mut(id)
    }

    pub fn require(&self, id: &RegionId) -> Result<&Region> {
        self.regions
            .get(id)
            .ok_or_else(|| GameError::RegionNotFound(id.clone()))
    }

    pub fn contain(&self, id: &RegionId) -> bool {
        self.regions.contains_key(id)
    }

    /// Add an account to a region, enforcing open status and capacity.
    pub fn add_player(&mut self, region_id: &RegionId, account: &AccountId) -> Result<()> {
        let region = self
            .regions
            .get_mut(region_id)
            .ok_or_else(|| GameError::RegionNotFound(region_id.clone()))?;
        if region.status != RegionStatus::Open {
            return Err(GameError::RegionNotOpen(region_id.clone()));
        }
        if region.players.contains(account) {
            return Ok(());
        }
        if region.is_full() {
            return Err(GameError::RegionFull(region_id.clone()));
        }
        region.players.insert(account.clone());
        Ok(())
    }

    pub fn remove_player(&mut self, region_id: &RegionId, account: &AccountId) {
        if let Some(region) = self.regions.get_mut(region_id) {
            region.players.remove(account);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Region> {
        self.regions.values()
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}
