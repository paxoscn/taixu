//! Item definitions and slot-based inventories.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::ItemId;
use crate::reward::ItemGrant;

/// Visual / economic tier of an item or cosmetic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rarity {
    #[default]
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Mythic,
}

/// Static definition of an item type. Register definitions in
/// [`ItemCatalog`] before granting items.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemDef {
    pub id: ItemId,
    pub name: String,
    pub description: String,
    pub rarity: Rarity,
    /// Stackable items may occupy a slot with more than one unit.
    pub stackable: bool,
    /// Units per slot for stackable items. Forced to `1` for unique items.
    pub max_stack: u64,
    /// Free-form tags used by the host game (`consumable`, `quest`, ...).
    pub tags: BTreeSet<String>,
}

impl ItemDef {
    pub fn new(id: impl Into<ItemId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            rarity: Rarity::Common,
            stackable: true,
            max_stack: 999,
            tags: BTreeSet::new(),
        }
    }

    /// Make the item unique (one unit per slot).
    pub fn unique(mut self) -> Self {
        self.stackable = false;
        self.max_stack = 1;
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_rarity(mut self, rarity: Rarity) -> Self {
        self.rarity = rarity;
        self
    }

    pub fn with_stack_limit(mut self, max_stack: u64) -> Self {
        self.stackable = max_stack > 1;
        self.max_stack = max_stack.max(1);
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.insert(tag.into());
        self
    }

    pub fn per_slot(&self) -> u64 {
        if self.stackable {
            self.max_stack.max(1)
        } else {
            1
        }
    }
}

/// Catalog of item definitions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemCatalog {
    defs: BTreeMap<ItemId, ItemDef>,
}

impl ItemCatalog {
    pub fn register(&mut self, mut def: ItemDef) -> Result<()> {
        if def.name.trim().is_empty() {
            return Err(GameError::InvalidInput(
                "item name must not be empty".into(),
            ));
        }
        if def.stackable && def.max_stack == 0 {
            def.max_stack = 1;
        }
        if !def.stackable {
            def.max_stack = 1;
        }
        if self.defs.contains_key(&def.id) {
            return Err(GameError::ItemAlreadyExists(def.id));
        }
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    pub fn upsert(&mut self, def: ItemDef) {
        self.defs.insert(def.id.clone(), def);
    }

    pub fn get(&self, id: &ItemId) -> Option<&ItemDef> {
        self.defs.get(id)
    }

    pub fn require(&self, id: &ItemId) -> Result<&ItemDef> {
        self.defs
            .get(id)
            .ok_or_else(|| GameError::UnknownItem(id.clone()))
    }

    pub fn iter(&self) -> impl Iterator<Item = &ItemDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// One inventory slot content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    item: ItemId,
    quantity: u64,
}

impl ItemStack {
    pub fn new(item: impl Into<ItemId>, quantity: u64) -> Self {
        Self {
            item: item.into(),
            quantity,
        }
    }

    pub fn item(&self) -> &ItemId {
        &self.item
    }

    pub fn quantity(&self) -> u64 {
        self.quantity
    }
}

/// Fixed-capacity, slot-based inventory.
///
/// Mutations are all-or-nothing: a failed add/remove never partially applies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Inventory {
    capacity: usize,
    slots: Vec<Option<ItemStack>>,
}

impl Inventory {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            slots: vec![None; capacity],
        }
    }

    pub fn with_contents(capacity: usize, stacks: impl IntoIterator<Item = ItemStack>) -> Self {
        let mut inventory = Self::new(capacity);
        for stack in stacks {
            if let Some(slot) = inventory.slots.iter_mut().find(|s| s.is_none()) {
                *slot = Some(stack);
            }
        }
        inventory
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn used_slots(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    pub fn free_slots(&self) -> usize {
        self.capacity - self.used_slots()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    pub fn is_full(&self) -> bool {
        self.used_slots() >= self.capacity
    }

    /// Read-only view of all slots, in slot order.
    pub fn slots(&self) -> &[Option<ItemStack>] {
        &self.slots
    }

    /// Total quantity of an item across all slots.
    pub fn count(&self, item: &ItemId) -> u64 {
        self.slots
            .iter()
            .flatten()
            .filter(|stack| stack.item == *item)
            .fold(0u64, |total, stack| total.saturating_add(stack.quantity))
    }

    pub fn has(&self, item: &ItemId, quantity: u64) -> bool {
        self.count(item) >= quantity
    }

    /// Add items, splitting across stacks as needed.
    ///
    /// Returns [`GameError::InventoryFull`] without modifying the inventory if
    /// the items cannot fit.
    pub fn try_add(&mut self, def: &ItemDef, quantity: u64) -> Result<()> {
        if quantity == 0 {
            return Err(GameError::InvalidQuantity(quantity));
        }

        let per_stack = def.per_slot();
        let existing_room: u64 = self
            .slots
            .iter()
            .flatten()
            .filter(|stack| stack.item == def.id)
            .map(|stack| per_stack.saturating_sub(stack.quantity))
            .fold(0u64, |a, b| a.saturating_add(b));
        let free_slots = self.free_slots() as u64;
        let total_room = existing_room.saturating_add(free_slots.saturating_mul(per_stack));
        if quantity > total_room {
            return Err(GameError::InventoryFull);
        }

        let mut remaining = quantity;
        for slot in self.slots.iter_mut() {
            if remaining == 0 {
                break;
            }
            if let Some(stack) = slot {
                if stack.item == def.id && stack.quantity < per_stack {
                    let room = per_stack - stack.quantity;
                    let added = room.min(remaining);
                    stack.quantity += added;
                    remaining -= added;
                }
            }
        }
        for slot in self.slots.iter_mut() {
            if remaining == 0 {
                break;
            }
            if slot.is_none() {
                let added = per_stack.min(remaining);
                *slot = Some(ItemStack::new(def.id.clone(), added));
                remaining -= added;
            }
        }
        debug_assert_eq!(remaining, 0);
        Ok(())
    }

    /// Add several grants atomically.
    pub fn try_add_many(&mut self, catalog: &ItemCatalog, grants: &[ItemGrant]) -> Result<()> {
        let mut trial = self.clone();
        for grant in grants {
            if grant.quantity == 0 {
                continue;
            }
            let def = catalog.require(&grant.item)?;
            trial.try_add(def, grant.quantity)?;
        }
        *self = trial;
        Ok(())
    }

    /// Remove items atomically.
    pub fn try_remove(&mut self, item: &ItemId, quantity: u64) -> Result<()> {
        if quantity == 0 {
            return Err(GameError::InvalidQuantity(quantity));
        }
        let available = self.count(item);
        if available < quantity {
            return Err(GameError::InsufficientItems {
                item: item.clone(),
                needed: quantity,
                available,
            });
        }

        let mut remaining = quantity;
        for slot in self.slots.iter_mut() {
            if remaining == 0 {
                break;
            }
            if let Some(stack) = slot {
                if stack.item == *item {
                    let taken = stack.quantity.min(remaining);
                    stack.quantity -= taken;
                    remaining -= taken;
                    if stack.quantity == 0 {
                        *slot = None;
                    }
                }
            }
        }
        debug_assert_eq!(remaining, 0);
        Ok(())
    }

    /// Remove several item quantities atomically.
    pub fn try_remove_many(&mut self, items: &BTreeMap<ItemId, u64>) -> Result<()> {
        let mut trial = self.clone();
        for (item, quantity) in items {
            trial.try_remove(item, *quantity)?;
        }
        *self = trial;
        Ok(())
    }

    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
    }

    /// Grow or shrink the inventory. Shrinking is rejected when any removed
    /// slot is occupied, so items are never silently destroyed.
    pub fn resize(&mut self, capacity: usize) -> Result<()> {
        if capacity == 0 {
            return Err(GameError::InvalidInput(
                "inventory capacity must be at least 1".into(),
            ));
        }
        if capacity < self.capacity {
            if self.slots[capacity..].iter().any(Option::is_some) {
                return Err(GameError::InvalidInput(
                    "cannot shrink inventory while the removed slots are occupied".into(),
                ));
            }
            self.slots.truncate(capacity);
        } else {
            self.slots.resize(capacity, None);
        }
        self.capacity = capacity;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> ItemCatalog {
        let mut catalog = ItemCatalog::default();
        catalog
            .register(ItemDef::new("potion", "Potion").with_stack_limit(10))
            .unwrap();
        catalog
            .register(ItemDef::new("sword", "Sword").unique())
            .unwrap();
        catalog
    }

    #[test]
    fn stacks_across_slots_and_removes_atomically() {
        let catalog = catalog();
        let potion = catalog.get(&"potion".into()).unwrap();
        let mut inventory = Inventory::new(2);
        inventory.try_add(potion, 15).unwrap();
        assert_eq!(inventory.count(&"potion".into()), 15);
        assert_eq!(inventory.used_slots(), 2);
        // The second stack has five free units.
        inventory.try_add(potion, 5).unwrap();
        assert_eq!(inventory.count(&"potion".into()), 20);
        assert!(inventory.try_add(potion, 1).is_err());
        assert_eq!(inventory.count(&"potion".into()), 20);
        inventory.try_remove(&"potion".into(), 12).unwrap();
        assert_eq!(inventory.count(&"potion".into()), 8);
        assert_eq!(inventory.used_slots(), 1);
    }

    #[test]
    fn unique_items_take_one_slot_each() {
        let catalog = catalog();
        let sword = catalog.get(&"sword".into()).unwrap();
        let mut inventory = Inventory::new(2);
        inventory.try_add(sword, 2).unwrap();
        assert_eq!(inventory.used_slots(), 2);
        assert!(inventory.try_add(sword, 1).is_err());
    }

    #[test]
    fn batch_add_is_atomic() {
        let catalog = catalog();
        let mut inventory = Inventory::new(1);
        let result = inventory.try_add_many(
            &catalog,
            &[ItemGrant::new("potion", 10), ItemGrant::new("sword", 1)],
        );
        assert!(result.is_err());
        assert!(inventory.is_empty());
    }
}
