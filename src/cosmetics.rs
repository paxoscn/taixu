//! Avatars, cosmetic skins and per-account ownership.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, SkinId};
use crate::item::Rarity;

/// Equipment slot for a cosmetic. One skin per slot can be equipped.
///
/// Serialized as a compact string so it can also serve as a JSON map key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkinCategory {
    Character,
    Weapon,
    Avatar,
    Frame,
    Badge,
    Background,
    Emote,
    Other(String),
}

impl fmt::Display for SkinCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkinCategory::Character => f.write_str("character"),
            SkinCategory::Weapon => f.write_str("weapon"),
            SkinCategory::Avatar => f.write_str("avatar"),
            SkinCategory::Frame => f.write_str("frame"),
            SkinCategory::Badge => f.write_str("badge"),
            SkinCategory::Background => f.write_str("background"),
            SkinCategory::Emote => f.write_str("emote"),
            SkinCategory::Other(name) => write!(f, "other:{name}"),
        }
    }
}

impl Serialize for SkinCategory {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SkinCategory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "character" => SkinCategory::Character,
            "weapon" => SkinCategory::Weapon,
            "avatar" => SkinCategory::Avatar,
            "frame" => SkinCategory::Frame,
            "badge" => SkinCategory::Badge,
            "background" => SkinCategory::Background,
            "emote" => SkinCategory::Emote,
            other => {
                let name = other
                    .strip_prefix("other:")
                    .ok_or_else(|| D::Error::custom(format!("unknown skin category: {other}")))?;
                SkinCategory::Other(name.to_owned())
            }
        })
    }
}

/// Static definition of a cosmetic skin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinDef {
    pub id: SkinId,
    pub name: String,
    pub description: String,
    pub category: SkinCategory,
    pub rarity: Rarity,
}

impl SkinDef {
    pub fn new(id: impl Into<SkinId>, name: impl Into<String>, category: SkinCategory) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            category,
            rarity: Rarity::Common,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_rarity(mut self, rarity: Rarity) -> Self {
        self.rarity = rarity;
        self
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkinCatalog {
    defs: BTreeMap<SkinId, SkinDef>,
}

impl SkinCatalog {
    pub fn register(&mut self, def: SkinDef) -> Result<()> {
        if self.defs.contains_key(&def.id) {
            return Err(GameError::SkinAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn get(&self, id: &SkinId) -> Option<&SkinDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &SkinId) -> Result<&SkinDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::SkinNotFound(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &SkinDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Skins owned and equipped by one account.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OwnedSkins {
    owned: BTreeSet<SkinId>,
    equipped: BTreeMap<SkinCategory, SkinId>,
}

impl OwnedSkins {
    pub fn owns(&self, skin: &SkinId) -> bool {
        self.owned.contains(skin)
    }

    pub fn owned(&self) -> &BTreeSet<SkinId> {
        &self.owned
    }

    pub fn equipped(&self) -> &BTreeMap<SkinCategory, SkinId> {
        &self.equipped
    }

    pub fn equipped_in(&self, category: &SkinCategory) -> Option<&SkinId> {
        self.equipped.get(category)
    }

    pub(crate) fn grant(&mut self, skin: SkinId) -> bool {
        self.owned.insert(skin)
    }

    pub(crate) fn equip(&mut self, skin: SkinId, category: SkinCategory) -> Result<Option<SkinId>> {
        if !self.owned.contains(&skin) {
            return Err(GameError::SkinNotOwned(skin));
        }
        Ok(self.equipped.insert(category, skin))
    }

    pub(crate) fn unequip(&mut self, category: &SkinCategory) -> Option<SkinId> {
        self.equipped.remove(category)
    }
}

/// All accounts' skin ownership / loadouts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cosmetics {
    accounts: BTreeMap<AccountId, OwnedSkins>,
}

impl Cosmetics {
    pub fn owned(&self, account: &AccountId) -> Option<&OwnedSkins> {
        self.accounts.get(account)
    }

    pub fn owned_mut(&mut self, account: &AccountId) -> &mut OwnedSkins {
        self.accounts.entry(account.clone()).or_default()
    }

    pub fn owns(&self, account: &AccountId, skin: &SkinId) -> bool {
        self.accounts
            .get(account)
            .is_some_and(|owned| owned.owns(skin))
    }

    /// Returns `true` when the skin was newly granted.
    pub fn grant(&mut self, account: &AccountId, skin: SkinId) -> bool {
        self.owned_mut(account).grant(skin)
    }

    /// Equip a skin for its category.
    pub fn equip(
        &mut self,
        account: &AccountId,
        skin: SkinId,
        category: SkinCategory,
    ) -> Result<Option<SkinId>> {
        self.owned_mut(account).equip(skin, category)
    }

    pub fn unequip(&mut self, account: &AccountId, category: &SkinCategory) -> Option<SkinId> {
        self.accounts
            .get_mut(account)
            .and_then(|owned| owned.unequip(category))
    }

    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }
}

/// Player-facing avatar configuration.
///
/// `frame` and `badge` must reference owned skins of the matching category;
/// [`crate::Game::update_avatar`] enforces this.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    /// Opaque host-defined image / preset identifier (e.g. a URL or asset id).
    pub image: Option<String>,
    pub frame: Option<SkinId>,
    pub badge: Option<SkinId>,
    pub title: Option<String>,
}

impl Avatar {
    pub fn new() -> Self {
        Self::default()
    }
}
