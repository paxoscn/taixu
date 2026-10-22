//! Time-windowed, region-targeted announcements with per-account read state.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, AnnouncementId, RegionId};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementKind {
    #[default]
    News,
    Event,
    Maintenance,
    Promotion,
    System,
}

/// Fields supplied when publishing an announcement.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnnouncementDraft {
    pub title: String,
    pub body: String,
    pub kind: AnnouncementKind,
    /// Higher priority announcements are listed first.
    pub priority: i32,
    pub starts_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
    /// `None` targets all regions. An empty set targets nobody.
    pub regions: Option<BTreeSet<RegionId>>,
    pub enabled: bool,
}

impl AnnouncementDraft {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            enabled: true,
            ..Self::default()
        }
    }

    pub fn with_kind(mut self, kind: AnnouncementKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_window(mut self, starts_at_ms: Option<i64>, ends_at_ms: Option<i64>) -> Self {
        self.starts_at_ms = starts_at_ms;
        self.ends_at_ms = ends_at_ms;
        self
    }

    pub fn with_regions(mut self, regions: impl IntoIterator<Item = RegionId>) -> Self {
        self.regions = Some(regions.into_iter().collect());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Announcement {
    pub id: AnnouncementId,
    pub title: String,
    pub body: String,
    pub kind: AnnouncementKind,
    pub priority: i32,
    pub starts_at_ms: i64,
    pub ends_at_ms: Option<i64>,
    pub regions: Option<BTreeSet<RegionId>>,
    pub enabled: bool,
    pub created_at_ms: i64,
}

impl Announcement {
    pub fn is_active(&self, now_ms: i64) -> bool {
        self.enabled
            && now_ms >= self.starts_at_ms
            && self.ends_at_ms.map_or(true, |end| now_ms < end)
    }

    pub fn targets_region(&self, region: &RegionId) -> bool {
        self.regions
            .as_ref()
            .map_or(true, |regions| regions.contains(region))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnnouncementBoard {
    announcements: BTreeMap<AnnouncementId, Announcement>,
    reads: BTreeMap<AnnouncementId, BTreeSet<AccountId>>,
}

impl AnnouncementBoard {
    pub fn publish(&mut self, draft: AnnouncementDraft, now_ms: i64) -> Result<Announcement> {
        let title = draft.title.trim();
        let body = draft.body.trim();
        if title.is_empty() || body.is_empty() {
            return Err(GameError::EmptyContent);
        }
        let starts_at_ms = draft.starts_at_ms.unwrap_or(now_ms);
        if draft.ends_at_ms.is_some_and(|end| end <= starts_at_ms) {
            return Err(GameError::InvalidAnnouncementWindow);
        }
        let announcement = Announcement {
            id: AnnouncementId::new(),
            title: title.to_owned(),
            body: body.to_owned(),
            kind: draft.kind,
            priority: draft.priority,
            starts_at_ms,
            ends_at_ms: draft.ends_at_ms,
            regions: draft.regions,
            enabled: draft.enabled,
            created_at_ms: now_ms,
        };
        self.announcements
            .insert(announcement.id.clone(), announcement.clone());
        Ok(announcement)
    }

    pub fn get(&self, id: &AnnouncementId) -> Option<&Announcement> {
        self.announcements.get(id)
    }

    pub fn require(&self, id: &AnnouncementId) -> Result<&Announcement> {
        self.announcements
            .get(id)
            .ok_or_else(|| GameError::AnnouncementNotFound(id.clone()))
    }

    pub fn remove(&mut self, id: &AnnouncementId) -> Option<Announcement> {
        self.reads.remove(id);
        self.announcements.remove(id)
    }

    pub fn set_enabled(&mut self, id: &AnnouncementId, enabled: bool) -> Result<()> {
        let announcement = self
            .announcements
            .get_mut(id)
            .ok_or_else(|| GameError::AnnouncementNotFound(id.clone()))?;
        announcement.enabled = enabled;
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Announcement> {
        self.announcements.values()
    }

    pub fn len(&self) -> usize {
        self.announcements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.announcements.is_empty()
    }

    /// Active announcements for a region, highest priority first.
    pub fn active(&self, region: &RegionId, now_ms: i64) -> Vec<Announcement> {
        let mut active: Vec<_> = self
            .announcements
            .values()
            .filter(|announcement| {
                announcement.is_active(now_ms) && announcement.targets_region(region)
            })
            .cloned()
            .collect();
        active.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.created_at_ms.cmp(&a.created_at_ms))
                .then_with(|| b.id.cmp(&a.id))
        });
        active
    }

    pub fn mark_read(&mut self, account: &AccountId, id: &AnnouncementId) -> Result<()> {
        if !self.announcements.contains_key(id) {
            return Err(GameError::AnnouncementNotFound(id.clone()));
        }
        self.reads
            .entry(id.clone())
            .or_default()
            .insert(account.clone());
        Ok(())
    }

    pub fn is_read(&self, account: &AccountId, id: &AnnouncementId) -> bool {
        self.reads
            .get(id)
            .is_some_and(|readers| readers.contains(account))
    }

    pub fn unread(&self, account: &AccountId, region: &RegionId, now_ms: i64) -> Vec<Announcement> {
        self.active(region, now_ms)
            .into_iter()
            .filter(|announcement| !self.is_read(account, &announcement.id))
            .collect()
    }
}
