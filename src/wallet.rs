//! Currencies and replenishable resources.
//!
//! * **Currencies** (gold, gems, event tokens, ...) are unbounded counters.
//! * **Resources** (energy, stamina, crafting charges, ...) can have a cap and
//!   regenerate passively at a configurable hourly rate.
//!
//! Regeneration is lazy: it is evaluated whenever a resource is read or
//! mutated, using the injected [`crate::clock::Clock`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{CurrencyId, ResourceId};
use crate::reward::{CurrencyGrant, ResourceGrant};

pub type Amount = u64;

const MS_PER_HOUR: u128 = 3_600_000;

/// Definition of a replenishable resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceDef {
    pub id: ResourceId,
    pub name: String,
    /// Maximum amount that can be held. `None` means unbounded.
    pub cap: Option<Amount>,
    /// Passive regeneration rate. `0` disables regeneration.
    pub regen_per_hour: Amount,
}

impl ResourceDef {
    pub fn new(id: impl Into<ResourceId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            cap: None,
            regen_per_hour: 0,
        }
    }

    pub fn with_cap(mut self, cap: Amount) -> Self {
        self.cap = Some(cap);
        self
    }

    pub fn with_regen_per_hour(mut self, rate: Amount) -> Self {
        self.regen_per_hour = rate;
        self
    }
}

/// Catalog of known resources. Unknown resources cannot be granted or spent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceCatalog {
    defs: BTreeMap<ResourceId, ResourceDef>,
}

impl ResourceCatalog {
    pub fn register(&mut self, def: ResourceDef) -> Result<()> {
        if self.defs.contains_key(&def.id) {
            return Err(GameError::ResourceAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn upsert(&mut self, def: ResourceDef) {
        self.defs.insert(def.id.clone(), def);
    }

    pub fn get(&self, id: &ResourceId) -> Option<&ResourceDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &ResourceId) -> Result<&ResourceDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::UnknownResource(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ResourceDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ResourcePool {
    amount: Amount,
    updated_at_ms: i64,
    /// Fractional regeneration progress, in units of
    /// `elapsed_ms * regen_per_hour` modulo one hour. Persisting the
    /// remainder prevents both drift and double-counting when the balance is
    /// read repeatedly at the same timestamp.
    #[serde(default)]
    regen_remainder: u64,
}

/// Per-account balance of currencies and resources.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Wallet {
    currencies: BTreeMap<CurrencyId, Amount>,
    resources: BTreeMap<ResourceId, ResourcePool>,
}

/// Read-only snapshot with regeneration already applied.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSnapshot {
    pub currencies: BTreeMap<CurrencyId, Amount>,
    pub resources: BTreeMap<ResourceId, Amount>,
}

impl Wallet {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- currencies --------------------------------------------------------

    pub fn currency_balance(&self, currency: &CurrencyId) -> Amount {
        self.currencies.get(currency).copied().unwrap_or(0)
    }

    pub fn currencies(&self) -> &BTreeMap<CurrencyId, Amount> {
        &self.currencies
    }

    pub fn add_currency(&mut self, currency: &CurrencyId, amount: Amount) -> Result<Amount> {
        if amount == 0 {
            return Ok(self.currency_balance(currency));
        }
        let slot = self.currencies.entry(currency.clone()).or_insert(0);
        *slot = slot.checked_add(amount).ok_or(GameError::AmountOverflow)?;
        Ok(*slot)
    }

    pub fn add_currencies(&mut self, grants: &[CurrencyGrant]) -> Result<()> {
        for grant in grants {
            self.add_currency(&grant.currency, grant.amount)?;
        }
        Ok(())
    }

    pub fn spend_currency(&mut self, currency: &CurrencyId, amount: Amount) -> Result<()> {
        let available = self.currency_balance(currency);
        if amount > available {
            return Err(GameError::InsufficientFunds {
                currency: currency.clone(),
                needed: amount,
                available,
            });
        }
        if amount > 0 {
            self.currencies.insert(currency.clone(), available - amount);
        }
        Ok(())
    }

    /// Atomically check and spend a set of currency costs.
    pub fn try_spend_currencies(&mut self, costs: &[CurrencyGrant]) -> Result<()> {
        let totals = aggregate_currencies(costs)?;
        for (currency, needed) in &totals {
            let available = self.currency_balance(currency);
            if *needed > available {
                return Err(GameError::InsufficientFunds {
                    currency: currency.clone(),
                    needed: *needed,
                    available,
                });
            }
        }
        for (currency, needed) in totals {
            let available = self.currency_balance(&currency);
            self.currencies.insert(currency, available - needed);
        }
        Ok(())
    }

    pub fn has_currencies(&self, costs: &[CurrencyGrant]) -> bool {
        aggregate_currencies(costs).is_ok_and(|totals| {
            totals
                .iter()
                .all(|(currency, needed)| self.currency_balance(currency) >= *needed)
        })
    }

    // ---- resources ---------------------------------------------------------

    /// Current amount with lazy regeneration applied.
    pub fn resource_balance(&mut self, def: &ResourceDef, now_ms: i64) -> Amount {
        self.apply_regen(def, now_ms)
    }

    /// Add to a resource, clamping at its cap. Returns the new balance.
    pub fn add_resource(
        &mut self,
        def: &ResourceDef,
        amount: Amount,
        now_ms: i64,
    ) -> Result<Amount> {
        let current = self.apply_regen(def, now_ms);
        let next = match def.cap {
            Some(cap) => current.saturating_add(amount).min(cap),
            None => current.saturating_add(amount),
        };
        let pool = self.resources.get_mut(&def.id).expect("pool inserted");
        pool.amount = next;
        Ok(next)
    }

    pub fn add_resources(
        &mut self,
        catalog: &ResourceCatalog,
        grants: &[ResourceGrant],
        now_ms: i64,
    ) -> Result<()> {
        for grant in grants {
            let def = catalog.require(&grant.resource)?;
            self.add_resource(def, grant.amount, now_ms)?;
        }
        Ok(())
    }

    pub fn spend_resource(&mut self, def: &ResourceDef, amount: Amount, now_ms: i64) -> Result<()> {
        let available = self.apply_regen(def, now_ms);
        if amount > available {
            return Err(GameError::InsufficientResources {
                resource: def.id.clone(),
                needed: amount,
                available,
            });
        }
        let pool = self.resources.get_mut(&def.id).expect("pool inserted");
        pool.amount = available - amount;
        Ok(())
    }

    /// Atomically check and spend a set of resource costs.
    pub fn try_spend_resources(
        &mut self,
        catalog: &ResourceCatalog,
        costs: &[ResourceGrant],
        now_ms: i64,
    ) -> Result<()> {
        let totals = aggregate_resources(costs)?;
        let mut defs = Vec::with_capacity(totals.len());
        for (resource_id, needed) in &totals {
            let def = catalog.require(resource_id)?.clone();
            let available = self.apply_regen(&def, now_ms);
            if *needed > available {
                return Err(GameError::InsufficientResources {
                    resource: resource_id.clone(),
                    needed: *needed,
                    available,
                });
            }
            defs.push((def, *needed));
        }
        for (def, needed) in defs {
            let pool = self.resources.get_mut(&def.id).expect("pool inserted");
            pool.amount -= needed;
        }
        Ok(())
    }

    /// Set a resource to an explicit value (clamped at the cap).
    pub fn set_resource(&mut self, def: &ResourceDef, amount: Amount, now_ms: i64) -> Amount {
        let amount = match def.cap {
            Some(cap) => amount.min(cap),
            None => amount,
        };
        self.resources.insert(
            def.id.clone(),
            ResourcePool {
                amount,
                updated_at_ms: now_ms,
                regen_remainder: 0,
            },
        );
        amount
    }

    /// Fill a resource to its cap (or to `u64::MAX` for uncapped resources).
    pub fn refill_resource(&mut self, def: &ResourceDef, now_ms: i64) -> Amount {
        self.set_resource(def, def.cap.unwrap_or(Amount::MAX), now_ms)
    }

    pub fn resource_amount(&self, resource: &ResourceId) -> Amount {
        self.resources
            .get(resource)
            .map(|pool| pool.amount)
            .unwrap_or(0)
    }

    /// Snapshot of the whole wallet with regeneration applied.
    pub fn snapshot(&mut self, catalog: &ResourceCatalog, now_ms: i64) -> WalletSnapshot {
        for def in catalog.iter() {
            self.apply_regen(def, now_ms);
        }
        WalletSnapshot {
            currencies: self.currencies.clone(),
            resources: self
                .resources
                .iter()
                .map(|(id, pool)| (id.clone(), pool.amount))
                .collect(),
        }
    }

    fn apply_regen(&mut self, def: &ResourceDef, now_ms: i64) -> Amount {
        let pool = self
            .resources
            .entry(def.id.clone())
            .or_insert(ResourcePool {
                amount: 0,
                updated_at_ms: now_ms,
                regen_remainder: 0,
            });

        // Reading at the same (or an earlier) timestamp must be a no-op.
        if now_ms <= pool.updated_at_ms {
            return pool.amount;
        }

        if def.regen_per_hour == 0 {
            if let Some(cap) = def.cap {
                pool.amount = pool.amount.min(cap);
            }
            pool.updated_at_ms = now_ms;
            pool.regen_remainder = 0;
            return pool.amount;
        }

        if let Some(cap) = def.cap {
            if pool.amount >= cap {
                pool.amount = cap;
                pool.updated_at_ms = now_ms;
                pool.regen_remainder = 0;
                return pool.amount;
            }
        }

        let elapsed_ms = (now_ms - pool.updated_at_ms) as u128;
        let total = elapsed_ms
            .saturating_mul(def.regen_per_hour as u128)
            .saturating_add(pool.regen_remainder as u128);
        let gained = total / MS_PER_HOUR;
        let remainder = (total % MS_PER_HOUR) as u64;

        // Consume the whole elapsed interval and carry the fractional part.
        // This guarantees that a repeated read at `now_ms` cannot grant the
        // same interval twice, even with very high regeneration rates.
        pool.updated_at_ms = now_ms;

        if gained == 0 {
            pool.regen_remainder = remainder;
            return pool.amount;
        }

        let gained = gained.min(Amount::MAX as u128) as Amount;
        let unbounded = pool.amount.saturating_add(gained);
        let reached_cap = def.cap.is_some_and(|cap| unbounded >= cap);
        pool.amount = match def.cap {
            Some(cap) => unbounded.min(cap),
            None => unbounded,
        };
        if reached_cap {
            pool.regen_remainder = 0;
        } else {
            pool.regen_remainder = remainder;
        }
        pool.amount
    }
}

fn aggregate_currencies(grants: &[CurrencyGrant]) -> Result<BTreeMap<CurrencyId, Amount>> {
    let mut totals: BTreeMap<CurrencyId, Amount> = BTreeMap::new();
    for grant in grants {
        if grant.amount == 0 {
            continue;
        }
        let slot = totals.entry(grant.currency.clone()).or_insert(0);
        *slot = slot
            .checked_add(grant.amount)
            .ok_or(GameError::AmountOverflow)?;
    }
    Ok(totals)
}

fn aggregate_resources(grants: &[ResourceGrant]) -> Result<BTreeMap<ResourceId, Amount>> {
    let mut totals: BTreeMap<ResourceId, Amount> = BTreeMap::new();
    for grant in grants {
        if grant.amount == 0 {
            continue;
        }
        let slot = totals.entry(grant.resource.clone()).or_insert(0);
        *slot = slot
            .checked_add(grant.amount)
            .ok_or(GameError::AmountOverflow)?;
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy() -> ResourceDef {
        ResourceDef::new("energy", "Energy")
            .with_cap(100)
            .with_regen_per_hour(60)
    }

    #[test]
    fn regeneration_is_lazy_and_fractional() {
        let mut wallet = Wallet::new();
        let def = ResourceDef::new("slow", "Slow")
            .with_cap(100)
            .with_regen_per_hour(1);
        assert_eq!(wallet.add_resource(&def, 0, 1_000).unwrap(), 0);
        // 30 minutes at 1/hour: still 0, but the half unit is remembered.
        assert_eq!(wallet.resource_balance(&def, 1_000 + 30 * 60_000), 0);
        // Another 30 minutes completes one unit.
        assert_eq!(wallet.resource_balance(&def, 1_000 + 60 * 60_000), 1);
        // 2 hours later: 2 more units.
        assert_eq!(wallet.resource_balance(&def, 1_000 + 3 * 3_600_000), 3);
    }

    #[test]
    fn repeated_reads_do_not_duplicate_regeneration() {
        let mut wallet = Wallet::new();
        let def = ResourceDef::new("plasma", "Plasma")
            .with_cap(1_000_000)
            .with_regen_per_hour(10_000_000);
        wallet.set_resource(&def, 0, 0);

        // Two units are produced in the first millisecond. Reading the same
        // timestamp repeatedly must not produce them again.
        assert_eq!(wallet.resource_balance(&def, 1), 2);
        assert_eq!(wallet.resource_balance(&def, 1), 2);
        assert_eq!(wallet.resource_balance(&def, 1), 2);
        // Advancing one millisecond grants only that millisecond's value.
        assert_eq!(wallet.resource_balance(&def, 2), 5);
    }

    #[test]
    fn fractional_regeneration_accumulates_across_reads() {
        let mut wallet = Wallet::new();
        let def = ResourceDef::new("trickle", "Trickle")
            .with_cap(1_000)
            .with_regen_per_hour(10);
        wallet.set_resource(&def, 0, 0);
        // 1000 reads, one second apart: 1000s * 10/hour = 2.77 units.
        for second in 1..=1_000 {
            wallet.resource_balance(&def, second * 1_000);
        }
        assert_eq!(wallet.resource_balance(&def, 1_000_000), 2);
    }

    #[test]
    fn regeneration_respects_cap() {
        let mut wallet = Wallet::new();
        let def = energy();
        wallet.set_resource(&def, 99, 0);
        assert_eq!(wallet.resource_balance(&def, 10 * 3_600_000), 100);
    }

    #[test]
    fn spending_is_atomic() {
        let mut wallet = Wallet::new();
        wallet.add_currency(&"gold".into(), 100).unwrap();
        let err = wallet
            .try_spend_currencies(&[
                CurrencyGrant::new("gold", 60),
                CurrencyGrant::new("gold", 60),
            ])
            .unwrap_err();
        assert!(matches!(err, GameError::InsufficientFunds { .. }));
        assert_eq!(wallet.currency_balance(&"gold".into()), 100);
    }

    #[test]
    fn resource_spending_uses_definitions() {
        let mut catalog = ResourceCatalog::default();
        assert!(catalog.register(energy()).is_ok());
        let def = catalog.get(&"energy".into()).unwrap();
        let mut wallet = Wallet::new();
        wallet.set_resource(def, 10, 0);
        assert!(wallet.spend_resource(def, 11, 0).is_err());
        wallet.spend_resource(def, 4, 0).unwrap();
        assert_eq!(wallet.resource_amount(&"energy".into()), 6);
    }
}
