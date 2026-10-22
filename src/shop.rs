//! Shops, offers, stock, per-account purchase limits and time windows.
//!
//! Purchases are applied atomically by [`crate::Game::purchase`]: payment and
//! delivery either both succeed or nothing changes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, OfferId, RegionId, ShopId};
use crate::reward::{Cost, Reward};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShopOffer {
    pub id: OfferId,
    pub name: String,
    pub description: String,
    pub cost: Cost,
    pub reward: Reward,
    pub starts_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
    /// Restrict this offer to accounts in these regions. `None` means all.
    pub regions: Option<BTreeSet<RegionId>>,
    /// Maximum units one account may buy over the offer's whole lifetime.
    pub per_account_limit: Option<u64>,
    /// Remaining stock across all accounts. `None` means unlimited.
    pub stock: Option<u64>,
    /// Units already sold. Maintained by the backend.
    #[serde(default)]
    pub sold: u64,
    pub tags: BTreeSet<String>,
}

impl ShopOffer {
    pub fn new(
        id: impl Into<OfferId>,
        name: impl Into<String>,
        cost: Cost,
        reward: Reward,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            cost,
            reward,
            starts_at_ms: None,
            ends_at_ms: None,
            regions: None,
            per_account_limit: None,
            stock: None,
            sold: 0,
            tags: BTreeSet::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
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

    pub fn with_per_account_limit(mut self, limit: u64) -> Self {
        self.per_account_limit = Some(limit);
        self
    }

    pub fn with_stock(mut self, stock: u64) -> Self {
        self.stock = Some(stock);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.insert(tag.into());
        self
    }

    pub fn available_at(&self, now_ms: i64, region: &RegionId) -> bool {
        let window = self.starts_at_ms.map_or(true, |start| now_ms >= start)
            && self.ends_at_ms.map_or(true, |end| now_ms < end);
        let region_ok = self
            .regions
            .as_ref()
            .map_or(true, |regions| regions.contains(region));
        window && region_ok
    }

    pub fn remaining_stock(&self) -> Option<u64> {
        self.stock.map(|stock| stock.saturating_sub(self.sold))
    }
}

/// A themed group of offers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shop {
    pub id: ShopId,
    pub name: String,
    pub description: String,
    pub regions: Option<BTreeSet<RegionId>>,
    pub opens_at_ms: Option<i64>,
    pub closes_at_ms: Option<i64>,
    pub offers: BTreeMap<OfferId, ShopOffer>,
}

impl Shop {
    pub fn new(id: impl Into<ShopId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            regions: None,
            opens_at_ms: None,
            closes_at_ms: None,
            offers: BTreeMap::new(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_window(mut self, opens_at_ms: Option<i64>, closes_at_ms: Option<i64>) -> Self {
        self.opens_at_ms = opens_at_ms;
        self.closes_at_ms = closes_at_ms;
        self
    }

    pub fn with_regions(mut self, regions: impl IntoIterator<Item = RegionId>) -> Self {
        self.regions = Some(regions.into_iter().collect());
        self
    }

    pub fn is_open_at(&self, now_ms: i64, region: &RegionId) -> bool {
        self.opens_at_ms.map_or(true, |open| now_ms >= open)
            && self.closes_at_ms.map_or(true, |close| now_ms < close)
            && self
                .regions
                .as_ref()
                .map_or(true, |regions| regions.contains(region))
    }

    pub fn add_offer(&mut self, offer: ShopOffer) -> Result<()> {
        if self.offers.contains_key(&offer.id) {
            return Err(GameError::OfferAlreadyExists(offer.id));
        }
        self.offers.insert(offer.id.clone(), offer);
        Ok(())
    }

    pub fn remove_offer(&mut self, offer: &OfferId) -> Option<ShopOffer> {
        self.offers.remove(offer)
    }

    pub fn offer(&self, id: &OfferId) -> Option<&ShopOffer> {
        self.offers.get(id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PurchaseRecord {
    shop: ShopId,
    offer: OfferId,
    account: AccountId,
    quantity: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShopBook {
    shops: BTreeMap<ShopId, Shop>,
    purchases: Vec<PurchaseRecord>,
}

impl ShopBook {
    pub fn register(&mut self, shop: Shop) -> Result<()> {
        if self.shops.contains_key(&shop.id) {
            return Err(GameError::ShopAlreadyExists(shop.id));
        }
        self.shops.insert(shop.id.clone(), shop);
        Ok(())
    }

    pub fn get(&self, id: &ShopId) -> Option<&Shop> {
        self.shops.get(id)
    }

    pub fn get_mut(&mut self, id: &ShopId) -> Option<&mut Shop> {
        self.shops.get_mut(id)
    }

    pub fn require(&self, id: &ShopId) -> Result<&Shop> {
        self.get(id)
            .ok_or_else(|| GameError::ShopNotFound(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Shop> {
        self.shops.values()
    }

    pub fn len(&self) -> usize {
        self.shops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shops.is_empty()
    }

    /// Number of units an account has bought from an offer.
    pub fn purchased(&self, shop: &ShopId, offer: &OfferId, account: &AccountId) -> u64 {
        self.purchases
            .iter()
            .filter(|record| {
                record.shop == *shop && record.offer == *offer && record.account == *account
            })
            .fold(0u64, |total, record| total.saturating_add(record.quantity))
    }

    pub(crate) fn record_purchase(
        &mut self,
        shop: ShopId,
        offer: OfferId,
        account: AccountId,
        quantity: u64,
    ) {
        self.purchases.push(PurchaseRecord {
            shop,
            offer,
            account,
            quantity,
        });
    }
}

/// Result of a successful purchase.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PurchaseReceipt {
    pub shop: ShopId,
    pub offer: OfferId,
    pub quantity: u64,
    pub paid: Cost,
    pub received: Reward,
}
