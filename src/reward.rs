//! Generic bundles of things a player can receive or pay.
//!
//! [`Reward`] is shared by the shop, achievements, collections, announcements
//! and NPC agents so that all grant paths are validated and applied the same
//! way.

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{CurrencyId, ItemId, ResourceId, SkinId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencyGrant {
    pub currency: CurrencyId,
    pub amount: u64,
}

impl CurrencyGrant {
    pub fn new(currency: impl Into<CurrencyId>, amount: u64) -> Self {
        Self {
            currency: currency.into(),
            amount,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceGrant {
    pub resource: ResourceId,
    pub amount: u64,
}

impl ResourceGrant {
    pub fn new(resource: impl Into<ResourceId>, amount: u64) -> Self {
        Self {
            resource: resource.into(),
            amount,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemGrant {
    pub item: ItemId,
    pub quantity: u64,
}

impl ItemGrant {
    pub fn new(item: impl Into<ItemId>, quantity: u64) -> Self {
        Self {
            item: item.into(),
            quantity,
        }
    }
}

/// Everything a player can receive at once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reward {
    #[serde(default)]
    pub currencies: Vec<CurrencyGrant>,
    #[serde(default)]
    pub resources: Vec<ResourceGrant>,
    #[serde(default)]
    pub items: Vec<ItemGrant>,
    #[serde(default)]
    pub skins: Vec<SkinId>,
    #[serde(default)]
    pub xp: u64,
}

impl Reward {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn currency(mut self, currency: impl Into<CurrencyId>, amount: u64) -> Self {
        self.currencies.push(CurrencyGrant::new(currency, amount));
        self
    }

    pub fn resource(mut self, resource: impl Into<ResourceId>, amount: u64) -> Self {
        self.resources.push(ResourceGrant::new(resource, amount));
        self
    }

    pub fn item(mut self, item: impl Into<ItemId>, quantity: u64) -> Self {
        self.items.push(ItemGrant::new(item, quantity));
        self
    }

    pub fn skin(mut self, skin: impl Into<SkinId>) -> Self {
        self.skins.push(skin.into());
        self
    }

    pub fn xp(mut self, amount: u64) -> Self {
        self.xp = self.xp.saturating_add(amount);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.currencies.is_empty()
            && self.resources.is_empty()
            && self.items.is_empty()
            && self.skins.is_empty()
            && self.xp == 0
    }

    /// Multiply a reward by a purchase quantity.
    ///
    /// Skins represent unique ownership and are never duplicated by scaling:
    /// buying five copies of an offer still grants a skin at most once.
    pub fn scaled(&self, quantity: u64) -> Result<Reward> {
        let currencies = self
            .currencies
            .iter()
            .map(|g| {
                Ok(CurrencyGrant {
                    currency: g.currency.clone(),
                    amount: g
                        .amount
                        .checked_mul(quantity)
                        .ok_or(GameError::AmountOverflow)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let resources = self
            .resources
            .iter()
            .map(|g| {
                Ok(ResourceGrant {
                    resource: g.resource.clone(),
                    amount: g
                        .amount
                        .checked_mul(quantity)
                        .ok_or(GameError::AmountOverflow)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let items = self
            .items
            .iter()
            .map(|g| {
                Ok(ItemGrant {
                    item: g.item.clone(),
                    quantity: g
                        .quantity
                        .checked_mul(quantity)
                        .ok_or(GameError::AmountOverflow)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Reward {
            currencies,
            resources,
            items,
            skins: self.skins.clone(),
            xp: self
                .xp
                .checked_mul(quantity)
                .ok_or(GameError::AmountOverflow)?,
        })
    }
}

/// Everything a player can pay at once.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    #[serde(default)]
    pub currencies: Vec<CurrencyGrant>,
    #[serde(default)]
    pub resources: Vec<ResourceGrant>,
}

impl Cost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn currency(mut self, currency: impl Into<CurrencyId>, amount: u64) -> Self {
        self.currencies.push(CurrencyGrant::new(currency, amount));
        self
    }

    pub fn resource(mut self, resource: impl Into<ResourceId>, amount: u64) -> Self {
        self.resources.push(ResourceGrant::new(resource, amount));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.currencies.is_empty() && self.resources.is_empty()
    }

    pub fn scaled(&self, quantity: u64) -> Result<Cost> {
        let currencies = self
            .currencies
            .iter()
            .map(|g| {
                Ok(CurrencyGrant {
                    currency: g.currency.clone(),
                    amount: g
                        .amount
                        .checked_mul(quantity)
                        .ok_or(GameError::AmountOverflow)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let resources = self
            .resources
            .iter()
            .map(|g| {
                Ok(ResourceGrant {
                    resource: g.resource.clone(),
                    amount: g
                        .amount
                        .checked_mul(quantity)
                        .ok_or(GameError::AmountOverflow)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Cost {
            currencies,
            resources,
        })
    }
}
