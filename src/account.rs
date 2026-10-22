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
    /// Externally authenticated, stable account id.
    pub id: AccountId,
    /// Player-facing name (1..=GameConfig::max_name_len characters).
    pub display_name: String,
    /// Assigned region / server node.
    pub region: RegionId,
    /// Host-defined level; the library does not auto-level from XP.
    pub level: u32,
    /// Cumulative experience; rewards add to this value.
    pub xp: u64,
    /// Account creation time (Unix ms).
    pub created_at_ms: i64,
    /// Last login/heartbeat/logout time (Unix ms). Used by hosts for presence GC.
    pub last_seen_ms: i64,
    /// Set by `login`/`heartbeat`, cleared by `logout`; never auto-expired.
    pub online: bool,
    /// Avatar image/title/frame/badge. Frame and badge mirror equipped skins.
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
