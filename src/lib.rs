#![forbid(unsafe_code)]

//! # Tai Xu
//!
//! General-purpose, embeddable game backend building blocks for Rust.
//!
//! Tai Xu is a **library**, not a server binary. Authentication is provided by
//! the host game; every operation takes a trusted [`AccountId`]. The library
//! owns the game-side domain state and enforces invariants atomically behind a
//! single lock. Host it behind your own HTTP / WebSocket / RPC layer and
//! persist [`GameState`] snapshots with any `serde` format.
//!
//! ## Features
//!
//! | Area | Module |
//! | --- | --- |
//! | Accounts (auth external) | [`account`] |
//! | Regions / server nodes | [`region`] |
//! | Rooms / lobbies | [`room`] |
//! | Friends, requests, blocking | [`friend`] |
//! | Global / region / room / DM chat | [`chat`] |
//! | Items & slot inventories | [`item`] |
//! | Currencies & regenerating resources | [`wallet`] |
//! | Shops, stock & purchase limits | [`shop`] |
//! | Player-to-player trading | [`trade`] |
//! | Avatars & cosmetic skins | [`cosmetics`] |
//! | Collection albums | [`collection`] |
//! | Leaderboards & seasons | [`leaderboard`] |
//! | Achievements | [`achievement`] |
//! | Tasks / quests | [`task`] |
//! | Announcements | [`announcement`] |
//! | NPC agents (sync or LLM-backed) | [`npc`] |
//! | Snapshot persistence | [`store`] |
//!
//! ## Quick start
//!
//! ```
//! use taixu::prelude::*;
//!
//! let game = Game::new();
//! game.create_region("asia", "Asia", Some(1_000), Some("wss://asia.example".into())).unwrap();
//! let account = game.create_account("player-1", "Nova", "asia").unwrap();
//!
//! // Register an item and grant it through the inventory.
//! game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20)).unwrap();
//! game.add_item(&account.id, &"potion".into(), 5).unwrap();
//! assert_eq!(game.item_count(&account.id, &"potion".into()).unwrap(), 5);
//!
//! // Create a room and chat in it.
//! let room = game.create_room(&account.id, "Hangout", &"asia".into(), 8, Default::default()).unwrap();
//! game.send_message(&account.id, ChatChannel::Room(room.id), "hello").unwrap();
//!
//! // Accept a task, advance it through gameplay events, then claim the reward.
//! game.register_task(
//!     TaskDef::new("first_steps", "First Steps")
//!         .with_objective(TaskObjective::metric("kills", 3))
//!         .with_reward(Reward::new().currency("gold", 50)),
//! ).unwrap();
//! game.accept_task(&account.id, &"first_steps".into()).unwrap();
//! let outcome = game.record_event(&account.id, "kills", 3).unwrap();
//! assert_eq!(outcome.tasks.len(), 1);
//! game.claim_task(&account.id, &"first_steps".into()).unwrap();
//! ```
//!
//! ## Design notes
//!
//! * **No authentication / authorization layer.** The caller is trusted.
//! * **One lock for cross-module atomicity.** Purchases, trades and claims
//!   either fully succeed or leave state untouched.
//! * **Serializable state.** [`Game::snapshot`] returns a plain [`GameState`]
//!   tree carrying [`GAME_STATE_VERSION`]; restore it with [`Game::from_state`].
//! * **Guard-free reads.** Prefer [`Game::with_state`] over holding a
//!   [`Game::read_state`] guard while calling write-locking methods.
//! * **Pluggable clock.** Use [`clock::ManualClock`] for deterministic tests.
//! * **NPC agents are async and external.** Register them on [`npc::NpcRuntime`];
//!   they are not part of the serialized state.

/// Compiles every Rust code block in the README as a doctest.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

pub mod account;
pub mod achievement;
pub mod announcement;
pub mod chat;
pub mod clock;
pub mod collection;
pub mod cosmetics;
pub mod error;
pub mod friend;
pub mod game;
pub mod ids;
pub mod item;
pub mod leaderboard;
pub mod npc;
pub mod region;
pub mod reward;
pub mod room;
pub mod shop;
pub mod store;
pub mod task;
pub mod trade;
pub mod wallet;

pub use error::{GameError, GameResult, Result};
pub use game::{Game, GameConfig, GameState, MetricOutcome, GAME_STATE_VERSION};
pub use ids::{
    AccountId, AchievementId, AnnouncementId, CollectionId, CurrencyId, ItemId, LeaderboardId,
    MessageId, NpcId, OfferId, RegionId, ResourceId, RoomId, ShopId, SkinId, TaskId, TradeId,
};
pub use reward::{Cost, CurrencyGrant, ItemGrant, ResourceGrant, Reward};

/// Re-exports for convenient `use taixu::prelude::*;`.
pub mod prelude {
    pub use crate::account::Account;
    pub use crate::achievement::{AchievementDef, AchievementView};
    pub use crate::announcement::{Announcement, AnnouncementDraft, AnnouncementKind};
    pub use crate::chat::{ChannelSummary, ChatAuthor, ChatChannel, ChatMessage};
    pub use crate::clock::{Clock, ManualClock, SystemClock};
    pub use crate::collection::{Collectible, CollectionDef, CollectionMilestone};
    pub use crate::cosmetics::{Avatar, SkinCategory, SkinDef};
    // `Result` is intentionally not exported: a glob import must not shadow
    // `std::result::Result` inside trait implementations.
    pub use crate::error::{GameError, GameResult};
    pub use crate::friend::FriendRequest;
    pub use crate::game::{Game, GameConfig, GameState, MetricOutcome, GAME_STATE_VERSION};
    pub use crate::ids::*;
    pub use crate::item::{Inventory, ItemDef, Rarity};
    pub use crate::leaderboard::{Leaderboard, ScoreEntry, ScoreOrder};
    pub use crate::npc::{
        NpcAction, NpcAgent, NpcContext, NpcDecision, NpcDef, NpcRuntime, RuleBasedNpc,
    };
    pub use crate::region::{Region, RegionStatus};
    pub use crate::reward::{Cost, CurrencyGrant, ItemGrant, ResourceGrant, Reward};
    pub use crate::room::Room;
    pub use crate::shop::{PurchaseReceipt, Shop, ShopOffer};
    pub use crate::store::{GameStore, MemoryStore};
    pub use crate::task::{
        TaskDef, TaskObjective, TaskObjectiveView, TaskRepeat, TaskStatus, TaskView,
    };
    pub use crate::trade::{Trade, TradeSide, TradeState};
    pub use crate::wallet::{ResourceDef, WalletSnapshot};
}
