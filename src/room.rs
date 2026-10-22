//! Game rooms / lobbies.
//!
//! An account can be in at most one room at a time. Rooms live inside a
//! region, and only accounts assigned to that region may join.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, RegionId, RoomId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Room {
    /// Server-generated room id.
    pub id: RoomId,
    /// Display name.
    pub name: String,
    /// Region the room belongs to.
    pub region: RegionId,
    /// Current owner; transferred when the owner leaves.
    pub owner: AccountId,
    /// Maximum players.
    pub capacity: usize,
    /// Current members.
    pub players: BTreeSet<AccountId>,
    /// Free-form room settings (topic, mode, host-defined flags).
    pub metadata: BTreeMap<String, String>,
    /// Creation time (Unix ms).
    pub created_at_ms: i64,
    /// Last membership/metadata change (Unix ms).
    pub updated_at_ms: i64,
}

impl Room {
    pub fn is_full(&self) -> bool {
        self.players.len() >= self.capacity
    }

    pub fn contains(&self, account: &AccountId) -> bool {
        self.players.contains(account)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RoomRegistry {
    rooms: BTreeMap<RoomId, Room>,
    membership: BTreeMap<AccountId, RoomId>,
}

impl RoomRegistry {
    pub fn create(
        &mut self,
        owner: AccountId,
        name: String,
        region: RegionId,
        capacity: usize,
        metadata: BTreeMap<String, String>,
        now_ms: i64,
    ) -> Result<Room> {
        if capacity == 0 {
            return Err(GameError::InvalidRoomCapacity {
                requested: capacity,
                max: usize::MAX,
            });
        }
        if self.membership.contains_key(&owner) {
            return Err(GameError::AlreadyInRoom {
                account: owner.clone(),
                room: self.membership[&owner].clone(),
            });
        }
        let id = RoomId::new();
        let mut players = BTreeSet::new();
        players.insert(owner.clone());
        let room = Room {
            id: id.clone(),
            name,
            region,
            owner: owner.clone(),
            capacity,
            players,
            metadata,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.rooms.insert(id.clone(), room.clone());
        self.membership.insert(owner, id);
        Ok(room)
    }

    pub fn get(&self, id: &RoomId) -> Option<&Room> {
        self.rooms.get(id)
    }

    pub fn require(&self, id: &RoomId) -> Result<&Room> {
        self.rooms
            .get(id)
            .ok_or_else(|| GameError::RoomNotFound(id.clone()))
    }

    pub fn room_of(&self, account: &AccountId) -> Option<&RoomId> {
        self.membership.get(account)
    }

    pub fn join(&mut self, account: &AccountId, room_id: &RoomId, now_ms: i64) -> Result<()> {
        if let Some(current) = self.membership.get(account) {
            if current == room_id {
                return Ok(());
            }
            return Err(GameError::AlreadyInRoom {
                account: account.clone(),
                room: current.clone(),
            });
        }
        let room = self
            .rooms
            .get_mut(room_id)
            .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
        if room.is_full() {
            return Err(GameError::RoomFull(room_id.clone()));
        }
        room.players.insert(account.clone());
        room.updated_at_ms = now_ms;
        self.membership.insert(account.clone(), room_id.clone());
        Ok(())
    }

    /// Remove an account from its room. If the owner leaves, ownership is
    /// transferred to the lowest account id, or the room is deleted when empty.
    pub fn leave(&mut self, account: &AccountId) -> Option<RoomId> {
        let room_id = self.membership.remove(account)?;
        let mut remove_room = false;
        if let Some(room) = self.rooms.get_mut(&room_id) {
            room.players.remove(account);
            if room.players.is_empty() {
                remove_room = true;
            } else if room.owner == *account {
                if let Some(next) = room.players.iter().next().cloned() {
                    room.owner = next;
                }
            }
        }
        if remove_room {
            self.rooms.remove(&room_id);
        }
        Some(room_id)
    }

    /// Owner-only room deletion. All members are detached.
    pub fn close(&mut self, owner: &AccountId, room_id: &RoomId) -> Result<Room> {
        let room = self
            .rooms
            .get(room_id)
            .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
        if room.owner != *owner {
            return Err(GameError::NotRoomOwner);
        }
        for player in room.players.iter() {
            self.membership.remove(player);
        }
        let room = self.rooms.remove(room_id).expect("room checked above");
        Ok(room)
    }

    /// Owner-only kick.
    pub fn kick(&mut self, owner: &AccountId, room_id: &RoomId, target: &AccountId) -> Result<()> {
        let room = self
            .rooms
            .get_mut(room_id)
            .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
        if room.owner != *owner {
            return Err(GameError::NotRoomOwner);
        }
        if target == owner {
            return Err(GameError::InvalidInput(
                "the owner cannot kick themselves".into(),
            ));
        }
        if !room.players.remove(target) {
            return Err(GameError::NotRoomMember(target.clone(), room_id.clone()));
        }
        self.membership.remove(target);
        Ok(())
    }

    pub fn set_metadata(
        &mut self,
        owner: &AccountId,
        room_id: &RoomId,
        key: String,
        value: String,
        now_ms: i64,
    ) -> Result<()> {
        let room = self
            .rooms
            .get_mut(room_id)
            .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
        if room.owner != *owner {
            return Err(GameError::NotRoomOwner);
        }
        room.metadata.insert(key, value);
        room.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn remove_metadata(
        &mut self,
        owner: &AccountId,
        room_id: &RoomId,
        key: &str,
        now_ms: i64,
    ) -> Result<bool> {
        let room = self
            .rooms
            .get_mut(room_id)
            .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
        if room.owner != *owner {
            return Err(GameError::NotRoomOwner);
        }
        room.updated_at_ms = now_ms;
        Ok(room.metadata.remove(key).is_some())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values()
    }

    pub fn len(&self) -> usize {
        self.rooms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}
