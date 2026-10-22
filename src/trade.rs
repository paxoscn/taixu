//! Player-to-player trades.
//!
//! Flow:
//! 1. Initiator creates a trade with a partner.
//! 2. Both sides set their offered items / currencies.
//! 3. Both sides confirm. The second confirmation executes the trade
//!    atomically: if either side no longer owns the offered assets, or the
//!    incoming items do not fit, nothing is transferred.
//!
//! Assets are not escrowed while the trade is open — they are validated both
//! when a side is set and again at execution time.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, CurrencyId, ItemId, TradeId};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeSide {
    pub items: BTreeMap<ItemId, u64>,
    pub currencies: BTreeMap<CurrencyId, u64>,
}

impl TradeSide {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_items(mut self, items: BTreeMap<ItemId, u64>) -> Self {
        self.items = items;
        self
    }

    pub fn with_currencies(mut self, currencies: BTreeMap<CurrencyId, u64>) -> Self {
        self.currencies = currencies;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.currencies.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeState {
    Open,
    Completed,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trade {
    pub id: TradeId,
    pub initiator: AccountId,
    pub partner: AccountId,
    pub initiator_side: TradeSide,
    pub partner_side: TradeSide,
    pub initiator_confirmed: bool,
    pub partner_confirmed: bool,
    pub state: TradeState,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Trade {
    pub fn is_participant(&self, account: &AccountId) -> bool {
        self.initiator == *account || self.partner == *account
    }

    pub fn is_open(&self) -> bool {
        self.state == TradeState::Open
    }

    pub fn is_expired(&self, now_ms: i64) -> bool {
        now_ms >= self.expires_at_ms
    }

    pub fn other(&self, account: &AccountId) -> Option<&AccountId> {
        if self.initiator == *account {
            Some(&self.partner)
        } else if self.partner == *account {
            Some(&self.initiator)
        } else {
            None
        }
    }

    pub fn side(&self, account: &AccountId) -> Option<&TradeSide> {
        if self.initiator == *account {
            Some(&self.initiator_side)
        } else if self.partner == *account {
            Some(&self.partner_side)
        } else {
            None
        }
    }

    pub fn side_mut(&mut self, account: &AccountId) -> Option<&mut TradeSide> {
        if self.initiator == *account {
            Some(&mut self.initiator_side)
        } else if self.partner == *account {
            Some(&mut self.partner_side)
        } else {
            None
        }
    }

    pub fn confirmed(&self, account: &AccountId) -> bool {
        if self.initiator == *account {
            self.initiator_confirmed
        } else if self.partner == *account {
            self.partner_confirmed
        } else {
            false
        }
    }

    pub fn set_confirmed(&mut self, account: &AccountId, value: bool) {
        if self.initiator == *account {
            self.initiator_confirmed = value;
        } else if self.partner == *account {
            self.partner_confirmed = value;
        }
    }

    fn reset_confirmations(&mut self) {
        self.initiator_confirmed = false;
        self.partner_confirmed = false;
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TradeBook {
    trades: BTreeMap<TradeId, Trade>,
}

impl TradeBook {
    pub fn create(
        &mut self,
        initiator: AccountId,
        partner: AccountId,
        now_ms: i64,
        expires_at_ms: i64,
    ) -> Result<Trade> {
        if initiator == partner {
            return Err(GameError::CannotTradeWithSelf);
        }
        let trade = Trade {
            id: TradeId::new(),
            initiator,
            partner,
            initiator_side: TradeSide::default(),
            partner_side: TradeSide::default(),
            initiator_confirmed: false,
            partner_confirmed: false,
            state: TradeState::Open,
            created_at_ms: now_ms,
            expires_at_ms,
            updated_at_ms: now_ms,
        };
        self.trades.insert(trade.id.clone(), trade.clone());
        Ok(trade)
    }

    pub fn get(&self, id: &TradeId) -> Option<&Trade> {
        self.trades.get(id)
    }

    pub fn get_mut(&mut self, id: &TradeId) -> Option<&mut Trade> {
        self.trades.get_mut(id)
    }

    pub fn require(&self, id: &TradeId) -> Result<&Trade> {
        self.get(id)
            .ok_or_else(|| GameError::TradeNotFound(id.clone()))
    }

    pub fn for_account(&self, account: &AccountId) -> Vec<Trade> {
        self.trades
            .values()
            .filter(|trade| trade.is_participant(account))
            .cloned()
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Trade> {
        self.trades.values()
    }

    pub fn len(&self) -> usize {
        self.trades.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trades.is_empty()
    }

    /// Mark open trades whose deadline has passed as expired.
    pub fn expire(&mut self, now_ms: i64) -> Vec<TradeId> {
        let mut expired = Vec::new();
        for trade in self.trades.values_mut() {
            if trade.state == TradeState::Open && trade.is_expired(now_ms) {
                trade.state = TradeState::Expired;
                trade.updated_at_ms = now_ms;
                expired.push(trade.id.clone());
            }
        }
        expired
    }

    pub(crate) fn update_side(
        &mut self,
        account: &AccountId,
        id: &TradeId,
        side: TradeSide,
        now_ms: i64,
    ) -> Result<()> {
        let trade = self
            .trades
            .get_mut(id)
            .ok_or_else(|| GameError::TradeNotFound(id.clone()))?;
        if !trade.is_participant(account) {
            return Err(GameError::NotTradeParticipant(account.clone()));
        }
        let side_slot = trade
            .side_mut(account)
            .ok_or_else(|| GameError::NotTradeParticipant(account.clone()))?;
        *side_slot = side;
        trade.reset_confirmations();
        trade.updated_at_ms = now_ms;
        Ok(())
    }
}
