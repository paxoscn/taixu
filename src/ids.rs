//! Strongly typed string identifiers used across the backend.
//!
//! Every identifier is a transparent newtype around `String`, so it serializes
//! as a plain string (also when used as a JSON map key) and remains easy to
//! transport through an external auth / gateway layer.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        // A random ID is deliberately not exposed as `Default`: a default
        // identifier would silently create a new entity instead of an empty one.
        #[allow(clippy::new_without_default)]
        impl $name {
            /// Generate a fresh globally unique identifier.
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, uuid::Uuid::new_v4().simple()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

define_id!(
    /// Identifier of an account. Accounts are provisioned by the host game and
    /// authenticated outside of this library.
    AccountId,
    "acc"
);
define_id!(/// Identifier of a region / server node.
    RegionId,
    "reg");
define_id!(/// Identifier of a room instance.
    RoomId,
    "room");
define_id!(/// Identifier of an item type.
    ItemId,
    "item");
define_id!(/// Identifier of a currency (gold, gems, ...).
    CurrencyId,
    "cur");
define_id!(/// Identifier of a replenishable resource (energy, stamina, ...).
    ResourceId,
    "res");
define_id!(/// Identifier of a shop.
    ShopId,
    "shop");
define_id!(/// Identifier of an offer inside a shop.
    OfferId,
    "offer");
define_id!(/// Identifier of a player-to-player trade.
    TradeId,
    "trade");
define_id!(/// Identifier of a cosmetic skin.
    SkinId,
    "skin");
define_id!(/// Identifier of a collection / album.
    CollectionId,
    "coll");
define_id!(/// Identifier of a leaderboard.
    LeaderboardId,
    "lb");
define_id!(/// Identifier of an achievement.
    AchievementId,
    "ach");
define_id!(/// Identifier of an announcement.
    AnnouncementId,
    "ann");
define_id!(/// Identifier of an NPC.
    NpcId,
    "npc");
define_id!(/// Identifier of a chat message.
    MessageId,
    "msg");
