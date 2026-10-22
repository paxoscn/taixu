//! Friend requests, friendships and blocking.
//!
//! Friendships are symmetric. Requests are directional until accepted.
//! Blocking is directional (A can block B without B knowing), and either
//! direction blocks friend requests and private messages.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::AccountId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FriendRequest {
    pub from: AccountId,
    pub to: AccountId,
    pub message: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FriendGraph {
    /// account -> friends
    friends: BTreeMap<AccountId, BTreeSet<AccountId>>,
    /// receiver -> sender -> request
    incoming: BTreeMap<AccountId, BTreeMap<AccountId, FriendRequest>>,
    /// blocker -> blocked accounts
    blocks: BTreeMap<AccountId, BTreeSet<AccountId>>,
}

impl FriendGraph {
    pub fn send_request(
        &mut self,
        from: &AccountId,
        to: &AccountId,
        message: Option<String>,
        now_ms: i64,
    ) -> Result<FriendRequest> {
        if from == to {
            return Err(GameError::CannotFriendSelf);
        }
        if self.blocked_either(from, to) {
            return Err(GameError::Blocked);
        }
        if self.are_friends(from, to) {
            return Err(GameError::AlreadyFriends(to.clone()));
        }
        if self.has_pending(from, to) {
            return Err(GameError::FriendRequestPending);
        }
        let request = FriendRequest {
            from: from.clone(),
            to: to.clone(),
            message,
            created_at_ms: now_ms,
        };
        self.incoming
            .entry(to.clone())
            .or_default()
            .insert(from.clone(), request.clone());
        Ok(request)
    }

    pub fn accept_request(&mut self, receiver: &AccountId, sender: &AccountId) -> Result<()> {
        let removed = self
            .incoming
            .get_mut(receiver)
            .and_then(|requests| requests.remove(sender));
        if removed.is_none() {
            return Err(GameError::FriendRequestNotFound);
        }
        self.friends
            .entry(receiver.clone())
            .or_default()
            .insert(sender.clone());
        self.friends
            .entry(sender.clone())
            .or_default()
            .insert(receiver.clone());
        Ok(())
    }

    pub fn decline_request(&mut self, receiver: &AccountId, sender: &AccountId) -> Result<()> {
        self.remove_request(receiver, sender)
    }

    pub fn cancel_request(&mut self, sender: &AccountId, receiver: &AccountId) -> Result<()> {
        self.remove_request(receiver, sender)
    }

    fn remove_request(&mut self, receiver: &AccountId, sender: &AccountId) -> Result<()> {
        let removed = self
            .incoming
            .get_mut(receiver)
            .and_then(|requests| requests.remove(sender));
        if removed.is_none() {
            return Err(GameError::FriendRequestNotFound);
        }
        Ok(())
    }

    pub fn remove_friend(&mut self, a: &AccountId, b: &AccountId) -> bool {
        let removed_a = self
            .friends
            .get_mut(a)
            .is_some_and(|friends| friends.remove(b));
        let removed_b = self
            .friends
            .get_mut(b)
            .is_some_and(|friends| friends.remove(a));
        removed_a || removed_b
    }

    pub fn block(&mut self, blocker: &AccountId, blocked: &AccountId) -> Result<()> {
        if blocker == blocked {
            return Err(GameError::InvalidInput("cannot block yourself".into()));
        }
        let set = self.blocks.entry(blocker.clone()).or_default();
        if !set.insert(blocked.clone()) {
            return Err(GameError::AlreadyBlocked);
        }
        self.remove_friend(blocker, blocked);
        if let Some(requests) = self.incoming.get_mut(blocker) {
            requests.remove(blocked);
        }
        if let Some(requests) = self.incoming.get_mut(blocked) {
            requests.remove(blocker);
        }
        Ok(())
    }

    pub fn unblock(&mut self, blocker: &AccountId, blocked: &AccountId) -> Result<()> {
        let removed = self
            .blocks
            .get_mut(blocker)
            .is_some_and(|set| set.remove(blocked));
        if removed {
            Ok(())
        } else {
            Err(GameError::NotBlocked)
        }
    }

    pub fn are_friends(&self, a: &AccountId, b: &AccountId) -> bool {
        self.friends
            .get(a)
            .is_some_and(|friends| friends.contains(b))
    }

    pub fn is_blocked(&self, blocker: &AccountId, blocked: &AccountId) -> bool {
        self.blocks
            .get(blocker)
            .is_some_and(|set| set.contains(blocked))
    }

    pub fn blocked_either(&self, a: &AccountId, b: &AccountId) -> bool {
        self.is_blocked(a, b) || self.is_blocked(b, a)
    }

    pub fn has_pending(&self, a: &AccountId, b: &AccountId) -> bool {
        self.incoming
            .get(b)
            .is_some_and(|requests| requests.contains_key(a))
            || self
                .incoming
                .get(a)
                .is_some_and(|requests| requests.contains_key(b))
    }

    pub fn friends(&self, account: &AccountId) -> Vec<AccountId> {
        self.friends
            .get(account)
            .map(|friends| friends.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn friend_count(&self, account: &AccountId) -> usize {
        self.friends.get(account).map_or(0, BTreeSet::len)
    }

    pub fn incoming_requests(&self, account: &AccountId) -> Vec<FriendRequest> {
        let mut requests: Vec<_> = self
            .incoming
            .get(account)
            .map(|requests| requests.values().cloned().collect())
            .unwrap_or_default();
        requests.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.from.cmp(&b.from))
        });
        requests
    }

    pub fn outgoing_requests(&self, account: &AccountId) -> Vec<FriendRequest> {
        let mut requests: Vec<_> = self
            .incoming
            .values()
            .flat_map(|requests| requests.values())
            .filter(|request| request.from == *account)
            .cloned()
            .collect();
        requests.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.to.cmp(&b.to))
        });
        requests
    }

    pub fn blocked_accounts(&self, account: &AccountId) -> Vec<AccountId> {
        self.blocks
            .get(account)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Remove every relationship involving an account (used by host cleanup).
    pub fn forget(&mut self, account: &AccountId) {
        self.friends.remove(account);
        for friends in self.friends.values_mut() {
            friends.remove(account);
        }
        self.incoming.remove(account);
        for requests in self.incoming.values_mut() {
            requests.remove(account);
        }
        self.blocks.remove(account);
        for set in self.blocks.values_mut() {
            set.remove(account);
        }
    }
}
