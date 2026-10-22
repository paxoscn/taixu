//! Player account profiles.
//!
//! Authentication is intentionally **not** part of this crate. The host game
//! authenticates the player and passes a trusted [`AccountId`] into every
//! operation. These records hold game-side profile data only.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::cosmetics::Avatar;
use crate::error::{GameError, Result};
use crate::ids::{AccountId, RegionId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub display_name: String,
    pub region: RegionId,
    pub level: u32,
    pub xp: u64,
    pub created_at_ms: i64,
    pub last_seen_ms: i64,
    pub online: bool,
    pub avatar: Avatar,
    /// Free-form profile data owned by the host game.
    pub metadata: BTreeMap<String, String>,
}

impl Account {
    pub fn new(id: AccountId, display_name: String, region: RegionId, now_ms: i64) -> Self {
        Self {
            id,
            display_name,
            region,
            level: 1,
            xp: 0,
            created_at_ms: now_ms,
            last_seen_ms: now_ms,
            online: false,
            avatar: Avatar::default(),
            metadata: BTreeMap::new(),
        }
    }
}

/// Trim, validate and return a display name.
pub fn validate_display_name(name: &str, max_len: usize) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(GameError::InvalidName("must not be empty".into()));
    }
    if trimmed.chars().count() > max_len {
        return Err(GameError::InvalidName(format!(
            "must be at most {max_len} characters"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(GameError::InvalidName(
            "must not contain control characters".into(),
        ));
    }
    Ok(trimmed.to_owned())
}
