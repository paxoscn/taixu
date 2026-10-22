//! Central, thread-safe backend service.
//!
//! [`Game`] owns all mutable state behind a single `RwLock` and exposes
//! high-level operations. Cross-module actions (shop purchase, trade
//! settlement, reward claims) are **atomic**: validation and all mutations
//! happen under one write lock, and account assets are cloned before a
//! multi-step mutation so that failures leave no partial state.
//!
//! The service is synchronous and runtime-agnostic; host it behind your own
//! HTTP/WebSocket layer. Persistence is provided by serializing
//! [`GameState`] snapshots (see [`crate::store`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard};
use serde::{Deserialize, Serialize};

use crate::account::{validate_display_name, Account};
use crate::achievement::{AchievementBook, AchievementDef, AchievementView};
use crate::announcement::{Announcement, AnnouncementBoard, AnnouncementDraft};
use crate::chat::{ChannelSummary, ChatAuthor, ChatChannel, ChatMessage, ChatService};
use crate::clock::{Clock, SystemClock};
use crate::collection::{
    Collectible, CollectionBook, CollectionDef, CollectionProgress, MilestoneProgress,
};
use crate::cosmetics::{Avatar, Cosmetics, OwnedSkins, SkinCatalog, SkinCategory, SkinDef};
use crate::error::{GameError, Result};
use crate::friend::{FriendGraph, FriendRequest};
use crate::ids::{
    AccountId, AchievementId, AnnouncementId, CollectionId, CurrencyId, ItemId, LeaderboardId,
    MessageId, NpcId, OfferId, RegionId, ResourceId, RoomId, ShopId, SkinId, TaskId, TradeId,
};
use crate::item::{Inventory, ItemCatalog, ItemDef};
use crate::leaderboard::{Leaderboard, ScoreEntry};
use crate::npc::{NearbyPlayer, NpcAction, NpcContext, NpcDef, NpcRegistry, NpcTurn};
use crate::region::{Region, RegionRegistry, RegionStatus};
use crate::reward::{Cost, CurrencyGrant, Reward};
use crate::room::{Room, RoomRegistry};
use crate::shop::{PurchaseReceipt, Shop, ShopBook, ShopOffer};
use crate::task::{
    TaskBook, TaskDef, TaskObjective, TaskObjectiveView, TaskProgress, TaskRepeat, TaskStatus,
    TaskView,
};
use crate::trade::{Trade, TradeBook, TradeSide, TradeState};
use crate::wallet::{ResourceCatalog, ResourceDef, Wallet, WalletSnapshot};

/// Tunable limits for a game instance.
#[derive(Clone, Debug)]
pub struct GameConfig {
    /// Inventory slots assigned to new accounts and missing inventories.
    pub default_inventory_capacity: usize,
    /// Upper bound accepted by `set_inventory_capacity`.
    pub max_inventory_capacity: usize,
    /// Maximum capacity accepted by `create_room`.
    pub max_room_capacity: usize,
    /// Maximum character count for account and room names.
    pub max_name_len: usize,
    /// Maximum character count for chat and NPC messages.
    pub max_chat_message_len: usize,
    /// Messages retained per channel; fixed when the `ChatService` is built.
    pub chat_history_per_channel: usize,
    /// New-trade lifetime in seconds (`expire_trades` applies it lazily).
    pub trade_expiry_secs: u64,
    /// Recent channel messages injected into an NPC context.
    pub npc_context_messages: usize,
    /// Maximum nearby players injected into an NPC context.
    pub npc_nearby_limit: usize,
    /// Maximum character count for a friend-request message.
    pub max_friend_request_len: usize,
    /// Maximum character count for an avatar title.
    pub max_avatar_title_len: usize,
    /// Maximum character count for an avatar image identifier/URL.
    pub max_avatar_image_len: usize,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            default_inventory_capacity: 50,
            max_inventory_capacity: 500,
            max_room_capacity: 200,
            max_name_len: 32,
            max_chat_message_len: 2_000,
            chat_history_per_channel: 100,
            trade_expiry_secs: 86_400,
            npc_context_messages: 20,
            npc_nearby_limit: 50,
            max_friend_request_len: 200,
            max_avatar_title_len: 48,
            max_avatar_image_len: 2_048,
        }
    }
}

/// Current schema version written into every [`GameState`] snapshot.
pub const GAME_STATE_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    GAME_STATE_VERSION
}

/// Full serializable game state.
///
/// This is the unit of persistence. It is deliberately a plain data tree so
/// hosts can persist it as JSON, MessagePack, a database snapshot, etc.
///
/// `version` identifies the schema. Snapshots written before the field existed
/// deserialize to the current version; when changing persisted structures,
/// bump [`GAME_STATE_VERSION`] and migrate snapshots before calling
/// [`Game::from_state`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameState {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    pub accounts: BTreeMap<AccountId, Account>,
    pub regions: RegionRegistry,
    pub rooms: RoomRegistry,
    pub friends: FriendGraph,
    pub chat: ChatService,
    pub items: ItemCatalog,
    pub resources: ResourceCatalog,
    pub wallets: BTreeMap<AccountId, Wallet>,
    pub inventories: BTreeMap<AccountId, Inventory>,
    pub shops: ShopBook,
    pub trades: TradeBook,
    pub skins: SkinCatalog,
    pub cosmetics: Cosmetics,
    pub collections: CollectionBook,
    pub leaderboards: BTreeMap<LeaderboardId, Leaderboard>,
    pub achievements: AchievementBook,
    pub tasks: TaskBook,
    pub announcements: AnnouncementBoard,
    pub npcs: NpcRegistry,
}

impl Default for GameState {
    fn default() -> Self {
        Self {
            version: GAME_STATE_VERSION,
            accounts: BTreeMap::new(),
            regions: RegionRegistry::default(),
            rooms: RoomRegistry::default(),
            friends: FriendGraph::default(),
            chat: ChatService::default(),
            items: ItemCatalog::default(),
            resources: ResourceCatalog::default(),
            wallets: BTreeMap::new(),
            inventories: BTreeMap::new(),
            shops: ShopBook::default(),
            trades: TradeBook::default(),
            skins: SkinCatalog::default(),
            cosmetics: Cosmetics::default(),
            collections: CollectionBook::default(),
            leaderboards: BTreeMap::new(),
            achievements: AchievementBook::default(),
            tasks: TaskBook::default(),
            announcements: AnnouncementBoard::default(),
            npcs: NpcRegistry::default(),
        }
    }
}

/// Result of feeding one gameplay event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricOutcome {
    /// Achievements completed by this event.
    pub achievements: Vec<AchievementId>,
    /// Tasks completed by this event.
    pub tasks: Vec<TaskId>,
}

/// Thread-safe facade over the whole backend.
pub struct Game {
    state: RwLock<GameState>,
    config: GameConfig,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Game {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read();
        f.debug_struct("Game")
            .field("accounts", &state.accounts.len())
            .field("regions", &state.regions.len())
            .field("rooms", &state.rooms.len())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        Self::with_config_and_clock(GameConfig::default(), Arc::new(SystemClock))
    }

    pub fn with_config(config: GameConfig) -> Self {
        Self::with_config_and_clock(config, Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self::with_config_and_clock(GameConfig::default(), clock)
    }

    pub fn with_config_and_clock(config: GameConfig, clock: Arc<dyn Clock>) -> Self {
        let state = GameState {
            chat: ChatService::new(config.chat_history_per_channel),
            ..GameState::default()
        };
        Self {
            state: RwLock::new(state),
            config,
            clock,
        }
    }

    /// Restore a game from a previous snapshot.
    ///
    /// A missing/zero schema version is normalized to
    /// [`GAME_STATE_VERSION`]. Hosts must migrate snapshots with a different
    /// version before calling this constructor.
    pub fn from_state(mut state: GameState, config: GameConfig, clock: Arc<dyn Clock>) -> Self {
        if state.version == 0 {
            state.version = GAME_STATE_VERSION;
        }
        Self {
            state: RwLock::new(state),
            config,
            clock,
        }
    }

    /// Schema version of the currently loaded state.
    pub fn state_version(&self) -> u32 {
        self.state.read().version
    }

    pub fn config(&self) -> &GameConfig {
        &self.config
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    /// Take a serializable snapshot. The lock is only held for the clone.
    pub fn snapshot(&self) -> GameState {
        self.state.read().clone()
    }

    /// Read-only access to raw state for integrations that need custom queries.
    ///
    /// **Deadlock warning:** parking_lot locks are not reentrant. Do not hold
    /// this guard while calling a `Game` method that takes a write lock
    /// (`wallet`, `task_views`, `purchase`, `record_event`, ...). Prefer
    /// [`Game::with_state`] so the guard cannot escape the query scope.
    pub fn read_state(&self) -> RwLockReadGuard<'_, GameState> {
        self.state.read()
    }

    /// Run a short read-only query under the read lock.
    ///
    /// The closure still must not call write-locking `Game` methods; it is a
    /// safer alternative to storing a [`Game::read_state`] guard.
    pub fn with_state<R>(&self, query: impl FnOnce(&GameState) -> R) -> R {
        query(&self.state.read())
    }

    // =====================================================================
    // Regions
    // =====================================================================

    pub fn create_region(
        &self,
        id: impl Into<RegionId>,
        name: impl Into<String>,
        capacity: Option<usize>,
        endpoint: Option<String>,
    ) -> Result<Region> {
        let id = id.into();
        let name = name.into().trim().to_owned();
        if name.is_empty() {
            return Err(GameError::InvalidInput(
                "region name must not be empty".into(),
            ));
        }
        if capacity == Some(0) {
            return Err(GameError::InvalidInput(
                "region capacity must be greater than zero".into(),
            ));
        }
        let region = Region::new(id, name, capacity, endpoint, self.now_ms());
        let mut state = self.state.write();
        state.regions.register(region.clone())?;
        Ok(region)
    }

    pub fn region(&self, id: &RegionId) -> Result<Region> {
        self.state
            .read()
            .regions
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::RegionNotFound(id.clone()))
    }

    pub fn regions(&self) -> Vec<Region> {
        self.state.read().regions.iter().cloned().collect()
    }

    pub fn region_player_count(&self, id: &RegionId) -> Result<usize> {
        Ok(self.state.read().regions.require(id)?.player_count())
    }

    pub fn set_region_status(&self, id: &RegionId, status: RegionStatus) -> Result<Region> {
        let mut state = self.state.write();
        let region = state
            .regions
            .get_mut(id)
            .ok_or_else(|| GameError::RegionNotFound(id.clone()))?;
        region.status = status;
        Ok(region.clone())
    }

    // =====================================================================
    // Accounts
    // =====================================================================

    /// Provision a game-side account. Authentication is external: the host
    /// passes a trusted, stable account id.
    pub fn create_account(
        &self,
        id: impl Into<AccountId>,
        display_name: &str,
        region: impl Into<RegionId>,
    ) -> Result<Account> {
        let id = id.into();
        let region = region.into();
        let display_name = validate_display_name(display_name, self.config.max_name_len)?;
        let now = self.now_ms();
        let mut state = self.state.write();
        create_account_locked(&mut state, id, display_name, region, now, &self.config)
    }

    /// Return the account if it exists, otherwise create it.
    ///
    /// The existence check and creation happen under one write lock, so
    /// concurrent first logins for the same id all resolve to one account
    /// instead of racing with `AccountAlreadyExists`.
    pub fn ensure_account(
        &self,
        id: impl Into<AccountId>,
        display_name: &str,
        region: impl Into<RegionId>,
    ) -> Result<Account> {
        let id = id.into();
        let region = region.into();
        let now = self.now_ms();
        let mut state = self.state.write();
        if let Some(account) = state.accounts.get(&id) {
            return Ok(account.clone());
        }
        let display_name = validate_display_name(display_name, self.config.max_name_len)?;
        create_account_locked(&mut state, id, display_name, region, now, &self.config)
    }

    pub fn account(&self, id: &AccountId) -> Result<Account> {
        self.state
            .read()
            .accounts
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::AccountNotFound(id.clone()))
    }

    pub fn account_count(&self) -> usize {
        self.state.read().accounts.len()
    }

    pub fn accounts_in_region(&self, region: &RegionId) -> Result<Vec<Account>> {
        let state = self.state.read();
        let region = state.regions.require(region)?;
        Ok(region
            .players
            .iter()
            .filter_map(|id| state.accounts.get(id).cloned())
            .collect())
    }

    pub fn login(&self, account: &AccountId) -> Result<Account> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.online = true;
        account.last_seen_ms = now;
        Ok(account.clone())
    }

    pub fn logout(&self, account: &AccountId) -> Result<Account> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.online = false;
        account.last_seen_ms = now;
        Ok(account.clone())
    }

    pub fn heartbeat(&self, account: &AccountId) -> Result<()> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.online = true;
        account.last_seen_ms = now;
        Ok(())
    }

    pub fn update_display_name(&self, account: &AccountId, name: &str) -> Result<Account> {
        let name = validate_display_name(name, self.config.max_name_len)?;
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.display_name = name;
        Ok(account.clone())
    }

    pub fn set_level(&self, account: &AccountId, level: u32) -> Result<Account> {
        if level == 0 {
            return Err(GameError::InvalidInput("level must be at least 1".into()));
        }
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.level = level;
        Ok(account.clone())
    }

    pub fn add_xp(&self, account: &AccountId, amount: u64) -> Result<u64> {
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.xp = account
            .xp
            .checked_add(amount)
            .ok_or(GameError::AmountOverflow)?;
        Ok(account.xp)
    }

    pub fn set_account_metadata(
        &self,
        account: &AccountId,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        account.metadata.insert(key.into(), value.into());
        Ok(())
    }

    pub fn remove_account_metadata(&self, account: &AccountId, key: &str) -> Result<bool> {
        let mut state = self.state.write();
        let account = state
            .accounts
            .get_mut(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        Ok(account.metadata.remove(key).is_some())
    }

    /// Move an account to another region. Leaves the current room (if any) and
    /// enforces the target region's status and capacity atomically.
    pub fn set_account_region(
        &self,
        account: &AccountId,
        target: impl Into<RegionId>,
    ) -> Result<Account> {
        let target = target.into();
        let mut state = self.state.write();
        let current_region = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?
            .region
            .clone();
        if current_region == target {
            return Ok(state.accounts[account].clone());
        }
        check_region_accepts(&state, &target, account)?;
        state.regions.add_player(&target, account)?;
        state.rooms.leave(account);
        state.regions.remove_player(&current_region, account);
        let moved = state.accounts.get_mut(account).expect("account checked");
        moved.region = target;
        Ok(moved.clone())
    }

    // =====================================================================
    // Inventory / wallet
    // =====================================================================

    pub fn register_item(&self, def: ItemDef) -> Result<()> {
        self.state.write().items.register(def)
    }

    pub fn item(&self, id: &ItemId) -> Result<ItemDef> {
        self.state
            .read()
            .items
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::UnknownItem(id.clone()))
    }

    pub fn items(&self) -> Vec<ItemDef> {
        self.state.read().items.iter().cloned().collect()
    }

    pub fn register_resource(&self, def: ResourceDef) -> Result<()> {
        self.state.write().resources.register(def)
    }

    pub fn resources(&self) -> Vec<ResourceDef> {
        self.state.read().resources.iter().cloned().collect()
    }

    pub fn inventory(&self, account: &AccountId) -> Result<Inventory> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state
            .inventories
            .get(account)
            .cloned()
            .unwrap_or_else(|| Inventory::new(self.config.default_inventory_capacity)))
    }

    pub fn item_count(&self, account: &AccountId, item: &ItemId) -> Result<u64> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state
            .inventories
            .get(account)
            .map(|inventory| inventory.count(item))
            .unwrap_or(0))
    }

    /// Grant items through the inventory (no payment, no reward bundle).
    pub fn add_item(&self, account: &AccountId, item: &ItemId, quantity: u64) -> Result<()> {
        if quantity == 0 {
            return Err(GameError::InvalidQuantity(quantity));
        }
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.items.require(item)?.clone();
        state
            .inventories
            .entry(account.clone())
            .or_insert_with(|| Inventory::new(self.config.default_inventory_capacity))
            .try_add(&def, quantity)?;
        // Capture item-based task completion at acquisition time so a task
        // completed inside its deadline is not lost if it expires before the
        // next query.
        refresh_account_tasks(&mut state, account, now);
        Ok(())
    }

    pub fn remove_item(&self, account: &AccountId, item: &ItemId, quantity: u64) -> Result<()> {
        if quantity == 0 {
            return Err(GameError::InvalidQuantity(quantity));
        }
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state
            .inventories
            .entry(account.clone())
            .or_insert_with(|| Inventory::new(self.config.default_inventory_capacity))
            .try_remove(item, quantity)?;
        Ok(())
    }

    pub fn set_inventory_capacity(
        &self,
        account: &AccountId,
        capacity: usize,
    ) -> Result<Inventory> {
        if capacity == 0 || capacity > self.config.max_inventory_capacity {
            return Err(GameError::InvalidInput(format!(
                "inventory capacity must be between 1 and {}",
                self.config.max_inventory_capacity
            )));
        }
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let inventory = state
            .inventories
            .entry(account.clone())
            .or_insert_with(|| Inventory::new(capacity));
        inventory.resize(capacity)?;
        Ok(inventory.clone())
    }

    pub fn wallet(&self, account: &AccountId) -> Result<WalletSnapshot> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let resources = state.resources.clone();
        let wallet = state.wallets.entry(account.clone()).or_default();
        Ok(wallet.snapshot(&resources, now))
    }

    pub fn currency_balance(&self, account: &AccountId, currency: &CurrencyId) -> Result<u64> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state
            .wallets
            .get(account)
            .map(|wallet| wallet.currency_balance(currency))
            .unwrap_or(0))
    }

    pub fn add_currency(
        &self,
        account: &AccountId,
        currency: &CurrencyId,
        amount: u64,
    ) -> Result<u64> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state
            .wallets
            .entry(account.clone())
            .or_default()
            .add_currency(currency, amount)
    }

    pub fn spend_currency(
        &self,
        account: &AccountId,
        currency: &CurrencyId,
        amount: u64,
    ) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state
            .wallets
            .entry(account.clone())
            .or_default()
            .spend_currency(currency, amount)
    }

    pub fn resource_balance(&self, account: &AccountId, resource: &ResourceId) -> Result<u64> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.resources.require(resource)?.clone();
        let wallet = state.wallets.entry(account.clone()).or_default();
        Ok(wallet.resource_balance(&def, now))
    }

    pub fn add_resource(
        &self,
        account: &AccountId,
        resource: &ResourceId,
        amount: u64,
    ) -> Result<u64> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.resources.require(resource)?.clone();
        state
            .wallets
            .entry(account.clone())
            .or_default()
            .add_resource(&def, amount, now)
    }

    pub fn spend_resource(
        &self,
        account: &AccountId,
        resource: &ResourceId,
        amount: u64,
    ) -> Result<()> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.resources.require(resource)?.clone();
        state
            .wallets
            .entry(account.clone())
            .or_default()
            .spend_resource(&def, amount, now)
    }

    pub fn set_resource(
        &self,
        account: &AccountId,
        resource: &ResourceId,
        amount: u64,
    ) -> Result<u64> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.resources.require(resource)?.clone();
        Ok(state
            .wallets
            .entry(account.clone())
            .or_default()
            .set_resource(&def, amount, now))
    }

    /// Grant an arbitrary reward bundle atomically.
    pub fn grant_reward(&self, account: &AccountId, reward: &Reward) -> Result<()> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let mut assets = load_assets(&state, &self.config, account)?;
        apply_reward(&state, &mut assets, reward, now)?;
        save_assets(&mut state, account, assets, now);
        Ok(())
    }

    // =====================================================================
    // Cosmetics / avatars
    // =====================================================================

    pub fn register_skin(&self, def: SkinDef) -> Result<()> {
        self.state.write().skins.register(def)
    }

    pub fn skins(&self) -> Vec<SkinDef> {
        self.state.read().skins.iter().cloned().collect()
    }

    /// Grant a skin. Returns `true` when it was newly acquired. Idempotent, so
    /// reward bundles may safely include already-owned skins.
    pub fn grant_skin(&self, account: &AccountId, skin: &SkinId) -> Result<bool> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state.skins.require(skin)?;
        let granted = state.cosmetics.grant(account, skin.clone());
        refresh_account_tasks(&mut state, account, now);
        Ok(granted)
    }

    pub fn owns_skin(&self, account: &AccountId, skin: &SkinId) -> Result<bool> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.cosmetics.owns(account, skin))
    }

    pub fn owned_skins(&self, account: &AccountId) -> Result<OwnedSkins> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.cosmetics.owned(account).cloned().unwrap_or_default())
    }

    pub fn equipped_skins(&self, account: &AccountId) -> Result<BTreeMap<SkinCategory, SkinId>> {
        Ok(self.owned_skins(account)?.equipped().clone())
    }

    /// Equip an owned skin, replacing whatever was equipped in its category.
    /// Equip an owned skin.
    ///
    /// Frame and badge skins are mirrored into [`Avatar`] so the avatar and
    /// equipped-skin state never diverge.
    pub fn equip_skin(&self, account: &AccountId, skin: &SkinId) -> Result<Option<SkinId>> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let category = state.skins.require(skin)?.category.clone();
        let previous = state
            .cosmetics
            .equip(account, skin.clone(), category.clone())?;
        let avatar = &mut state
            .accounts
            .get_mut(account)
            .expect("account checked")
            .avatar;
        match category {
            SkinCategory::Frame => avatar.frame = Some(skin.clone()),
            SkinCategory::Badge => avatar.badge = Some(skin.clone()),
            _ => {}
        }
        Ok(previous)
    }

    /// Unequip a skin category. Frame/badge selections are cleared from the
    /// avatar as well.
    pub fn unequip_skin(
        &self,
        account: &AccountId,
        category: &SkinCategory,
    ) -> Result<Option<SkinId>> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let removed = state.cosmetics.unequip(account, category);
        let avatar = &mut state
            .accounts
            .get_mut(account)
            .expect("account checked")
            .avatar;
        match category {
            SkinCategory::Frame => avatar.frame = None,
            SkinCategory::Badge => avatar.badge = None,
            _ => {}
        }
        Ok(removed)
    }

    /// Update avatar, validating that frame / badge skins are owned and of the
    /// correct category.
    ///
    /// The supplied avatar is authoritative: frame/badge values are mirrored
    /// into the account's equipped skins, and `None` unequips that category.
    pub fn update_avatar(&self, account: &AccountId, mut avatar: Avatar) -> Result<Avatar> {
        avatar.image = avatar
            .image
            .map(|image| image.trim().to_owned())
            .filter(|image| !image.is_empty());
        avatar.title = avatar
            .title
            .map(|title| title.trim().to_owned())
            .filter(|title| !title.is_empty());

        if let Some(image) = &avatar.image {
            if image.chars().count() > self.config.max_avatar_image_len {
                return Err(GameError::InvalidInput(format!(
                    "avatar image must be at most {} characters",
                    self.config.max_avatar_image_len
                )));
            }
        }
        if let Some(title) = &avatar.title {
            if title.chars().count() > self.config.max_avatar_title_len
                || title.chars().any(char::is_control)
            {
                return Err(GameError::InvalidInput(format!(
                    "avatar title must be at most {} printable characters",
                    self.config.max_avatar_title_len
                )));
            }
        }

        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        validate_skin_slot(&state, account, avatar.frame.as_ref(), &SkinCategory::Frame)?;
        validate_skin_slot(&state, account, avatar.badge.as_ref(), &SkinCategory::Badge)?;

        match &avatar.frame {
            Some(frame) => {
                state
                    .cosmetics
                    .equip(account, frame.clone(), SkinCategory::Frame)?;
            }
            None => {
                state.cosmetics.unequip(account, &SkinCategory::Frame);
            }
        }
        match &avatar.badge {
            Some(badge) => {
                state
                    .cosmetics
                    .equip(account, badge.clone(), SkinCategory::Badge)?;
            }
            None => {
                state.cosmetics.unequip(account, &SkinCategory::Badge);
            }
        }
        state
            .accounts
            .get_mut(account)
            .expect("account checked")
            .avatar = avatar.clone();
        Ok(avatar)
    }

    // =====================================================================
    // Friends
    // =====================================================================

    pub fn send_friend_request(
        &self,
        from: &AccountId,
        to: &AccountId,
        message: Option<&str>,
    ) -> Result<FriendRequest> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, from)?;
        ensure_account_exists(&state, to)?;
        check_region_open_for_activity(&state, from)?;
        let message = message
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .map(|message| {
                if message.chars().count() > self.config.max_friend_request_len {
                    Err(GameError::ContentTooLong {
                        max: self.config.max_friend_request_len,
                    })
                } else {
                    Ok(message.to_owned())
                }
            })
            .transpose()?;
        state.friends.send_request(from, to, message, now)
    }

    pub fn accept_friend_request(&self, receiver: &AccountId, sender: &AccountId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, receiver)?;
        check_region_open_for_activity(&state, receiver)?;
        state.friends.accept_request(receiver, sender)
    }

    pub fn decline_friend_request(&self, receiver: &AccountId, sender: &AccountId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, receiver)?;
        state.friends.decline_request(receiver, sender)
    }

    pub fn cancel_friend_request(&self, sender: &AccountId, receiver: &AccountId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, sender)?;
        state.friends.cancel_request(sender, receiver)
    }

    pub fn remove_friend(&self, a: &AccountId, b: &AccountId) -> Result<bool> {
        let mut state = self.state.write();
        ensure_account_exists(&state, a)?;
        Ok(state.friends.remove_friend(a, b))
    }

    pub fn block_account(&self, blocker: &AccountId, blocked: &AccountId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, blocker)?;
        ensure_account_exists(&state, blocked)?;
        state.friends.block(blocker, blocked)
    }

    pub fn unblock_account(&self, blocker: &AccountId, blocked: &AccountId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, blocker)?;
        state.friends.unblock(blocker, blocked)
    }

    pub fn friends(&self, account: &AccountId) -> Result<Vec<AccountId>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.friends.friends(account))
    }

    pub fn friend_requests(&self, account: &AccountId) -> Result<Vec<FriendRequest>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.friends.incoming_requests(account))
    }

    pub fn outgoing_friend_requests(&self, account: &AccountId) -> Result<Vec<FriendRequest>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.friends.outgoing_requests(account))
    }

    pub fn blocked_accounts(&self, account: &AccountId) -> Result<Vec<AccountId>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.friends.blocked_accounts(account))
    }

    pub fn are_friends(&self, a: &AccountId, b: &AccountId) -> bool {
        self.state.read().friends.are_friends(a, b)
    }

    // =====================================================================
    // Rooms
    // =====================================================================

    pub fn create_room(
        &self,
        owner: &AccountId,
        name: &str,
        region: &RegionId,
        capacity: usize,
        metadata: BTreeMap<String, String>,
    ) -> Result<Room> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > self.config.max_name_len {
            return Err(GameError::InvalidName(format!(
                "room name must be 1-{} characters",
                self.config.max_name_len
            )));
        }
        if capacity == 0 || capacity > self.config.max_room_capacity {
            return Err(GameError::InvalidRoomCapacity {
                requested: capacity,
                max: self.config.max_room_capacity,
            });
        }
        let now = self.now_ms();
        let mut state = self.state.write();
        let account = state
            .accounts
            .get(owner)
            .ok_or_else(|| GameError::AccountNotFound(owner.clone()))?;
        if account.region != *region {
            return Err(GameError::InvalidInput(
                "owner must belong to the room's region".into(),
            ));
        }
        // Maintenance keeps existing players active; only new entry is blocked.
        check_region_activity(&state, region)?;
        state.rooms.create(
            owner.clone(),
            name.to_owned(),
            region.clone(),
            capacity,
            metadata,
            now,
        )
    }

    pub fn room(&self, id: &RoomId) -> Result<Room> {
        self.state
            .read()
            .rooms
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::RoomNotFound(id.clone()))
    }

    pub fn rooms(&self, region: Option<&RegionId>) -> Vec<Room> {
        let state = self.state.read();
        let mut rooms: Vec<Room> = state
            .rooms
            .iter()
            .filter(|room| region.map_or(true, |region| room.region == *region))
            .cloned()
            .collect();
        rooms.sort_by(|a, b| {
            a.created_at_ms
                .cmp(&b.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        rooms
    }

    pub fn join_room(&self, account: &AccountId, room: &RoomId) -> Result<Room> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let account_info = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        let room_info = state
            .rooms
            .get(room)
            .cloned()
            .ok_or_else(|| GameError::RoomNotFound(room.clone()))?;
        if account_info.region != room_info.region {
            return Err(GameError::InvalidInput(
                "account and room belong to different regions".into(),
            ));
        }
        check_region_activity(&state, &room_info.region)?;
        state.rooms.join(account, room, now)?;
        Ok(state.rooms.require(room)?.clone())
    }

    pub fn leave_room(&self, account: &AccountId) -> Result<Option<RoomId>> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        Ok(state.rooms.leave(account))
    }

    pub fn close_room(&self, owner: &AccountId, room: &RoomId) -> Result<Room> {
        let mut state = self.state.write();
        ensure_account_exists(&state, owner)?;
        state.rooms.close(owner, room)
    }

    pub fn kick_from_room(
        &self,
        owner: &AccountId,
        room: &RoomId,
        target: &AccountId,
    ) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, owner)?;
        state.rooms.kick(owner, room, target)
    }

    pub fn room_id_of(&self, account: &AccountId) -> Result<Option<RoomId>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.rooms.room_of(account).cloned())
    }

    pub fn room_members(&self, room: &RoomId) -> Result<Vec<AccountId>> {
        let state = self.state.read();
        Ok(state.rooms.require(room)?.players.iter().cloned().collect())
    }

    pub fn update_room_metadata(
        &self,
        owner: &AccountId,
        room: &RoomId,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<()> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, owner)?;
        state
            .rooms
            .set_metadata(owner, room, key.into(), value.into(), now)
    }

    pub fn remove_room_metadata(
        &self,
        owner: &AccountId,
        room: &RoomId,
        key: &str,
    ) -> Result<bool> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, owner)?;
        state.rooms.remove_metadata(owner, room, key, now)
    }

    // =====================================================================
    // Chat
    // =====================================================================

    pub fn send_message(
        &self,
        account: &AccountId,
        channel: ChatChannel,
        body: &str,
    ) -> Result<ChatMessage> {
        self.send_message_with_metadata(account, channel, body, BTreeMap::new())
    }

    pub fn send_message_with_metadata(
        &self,
        account: &AccountId,
        channel: ChatChannel,
        body: &str,
        metadata: BTreeMap<String, String>,
    ) -> Result<ChatMessage> {
        let channel = normalize_channel(channel);
        let now = self.now_ms();
        let mut state = self.state.write();
        check_region_open_for_activity(&state, account)?;
        check_channel_access(&state, account, &channel)?;
        state.chat.post(
            channel,
            ChatAuthor::account(account.clone()),
            body,
            metadata,
            now,
            self.config.max_chat_message_len,
        )
    }

    /// Operator/system message; no account permission check.
    pub fn send_system_message(&self, channel: ChatChannel, body: &str) -> Result<ChatMessage> {
        let channel = normalize_channel(channel);
        let now = self.now_ms();
        let mut state = self.state.write();
        validate_channel_targets(&state, &channel)?;
        state.chat.post(
            channel,
            ChatAuthor::System,
            body,
            BTreeMap::new(),
            now,
            self.config.max_chat_message_len,
        )
    }

    /// Post a message authored by an NPC in that NPC's own channel.
    pub fn send_npc_message(
        &self,
        npc_id: &NpcId,
        channel: ChatChannel,
        body: &str,
    ) -> Result<ChatMessage> {
        let channel = normalize_channel(channel);
        let now = self.now_ms();
        let mut state = self.state.write();
        let npc = state.npcs.require(npc_id)?.clone();
        if !npc.enabled {
            return Err(GameError::NpcDisabled(npc_id.clone()));
        }
        if npc.channel() != channel {
            return Err(GameError::ChannelAccessDenied(channel));
        }
        if let Some(room) = &npc.room {
            state.rooms.require(room)?;
        } else if let Some(region) = &npc.region {
            state.regions.require(region)?;
        }
        state.chat.post(
            channel,
            ChatAuthor::npc(npc_id.clone()),
            body,
            BTreeMap::new(),
            now,
            self.config.max_chat_message_len,
        )
    }

    pub fn recent_messages(
        &self,
        account: &AccountId,
        channel: ChatChannel,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let channel = normalize_channel(channel);
        let state = self.state.read();
        check_channel_access(&state, account, &channel)?;
        Ok(state.chat.recent(&channel, limit))
    }

    /// Operator read access to any channel (used by NPCs, moderation tools).
    pub fn channel_history(&self, channel: &ChatChannel, limit: usize) -> Vec<ChatMessage> {
        self.state.read().chat.recent(channel, limit)
    }

    /// Channels with retained history that the account may currently read,
    /// newest activity first.
    ///
    /// Global, the account's region, rooms the account belongs to, and
    /// unblocked private conversations are included. Channels are created
    /// lazily by the first message.
    pub fn channels_for(&self, account: &AccountId) -> Result<Vec<ChannelSummary>> {
        let state = self.state.read();
        let account_info = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        let mut summaries: Vec<ChannelSummary> = state
            .chat
            .summaries()
            .into_iter()
            .filter(|summary| match &summary.channel {
                ChatChannel::Global => true,
                ChatChannel::Region(region) => *region == account_info.region,
                ChatChannel::Room(room) => state
                    .rooms
                    .get(room)
                    .is_some_and(|room| room.contains(account)),
                ChatChannel::Private { a, b } => {
                    if account != a && account != b {
                        return false;
                    }
                    let other = if account == a { b } else { a };
                    state.accounts.contains_key(other) && !state.friends.blocked_either(a, b)
                }
            })
            .collect();
        summaries.sort_by(|a, b| {
            b.last_message_at_ms
                .cmp(&a.last_message_at_ms)
                .then_with(|| a.channel.cmp(&b.channel))
        });
        Ok(summaries)
    }

    pub fn delete_message(&self, channel: &ChatChannel, message: &MessageId) -> Result<()> {
        let mut state = self.state.write();
        if state.chat.delete(channel, message) {
            Ok(())
        } else {
            Err(GameError::MessageNotFound(message.clone()))
        }
    }

    pub fn clear_channel(&self, channel: &ChatChannel) -> bool {
        self.state.write().chat.clear(channel)
    }

    // =====================================================================
    // Shops
    // =====================================================================

    pub fn register_shop(&self, shop: Shop) -> Result<()> {
        self.state.write().shops.register(shop)
    }

    pub fn shops(&self) -> Vec<Shop> {
        self.state.read().shops.iter().cloned().collect()
    }

    pub fn shop(&self, id: &ShopId) -> Result<Shop> {
        self.state
            .read()
            .shops
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::ShopNotFound(id.clone()))
    }

    pub fn add_offer(&self, shop: &ShopId, offer: ShopOffer) -> Result<()> {
        let mut state = self.state.write();
        state
            .shops
            .get_mut(shop)
            .ok_or_else(|| GameError::ShopNotFound(shop.clone()))?
            .add_offer(offer)
    }

    pub fn remove_offer(&self, shop: &ShopId, offer: &OfferId) -> Result<Option<ShopOffer>> {
        let mut state = self.state.write();
        Ok(state
            .shops
            .get_mut(shop)
            .ok_or_else(|| GameError::ShopNotFound(shop.clone()))?
            .remove_offer(offer))
    }

    /// Purchase `quantity` units of an offer atomically.
    pub fn purchase(
        &self,
        account: &AccountId,
        shop_id: &ShopId,
        offer_id: &OfferId,
        quantity: u64,
    ) -> Result<PurchaseReceipt> {
        if quantity == 0 {
            return Err(GameError::InvalidQuantity(quantity));
        }
        let now = self.now_ms();
        let mut state = self.state.write();
        let account_info = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?
            .clone();
        check_region_open_for_activity(&state, account)?;
        let offer = {
            let shop = state
                .shops
                .get(shop_id)
                .ok_or_else(|| GameError::ShopNotFound(shop_id.clone()))?;
            if !shop.is_open_at(now, &account_info.region) {
                return Err(GameError::OfferUnavailable);
            }
            shop.offer(offer_id)
                .cloned()
                .ok_or_else(|| GameError::OfferNotFound(offer_id.clone()))?
        };
        if !offer.available_at(now, &account_info.region) {
            return Err(GameError::OfferUnavailable);
        }
        if let Some(remaining) = offer.remaining_stock() {
            if quantity > remaining {
                return Err(GameError::OutOfStock);
            }
        }
        let already = state.shops.purchased(shop_id, offer_id, account);
        if let Some(limit) = offer.per_account_limit {
            let total = already
                .checked_add(quantity)
                .ok_or(GameError::AmountOverflow)?;
            if total > limit {
                return Err(GameError::PurchaseLimitReached);
            }
        }

        let cost = offer.cost.scaled(quantity)?;
        let reward = offer.reward.scaled(quantity)?;

        let mut assets = load_assets(&state, &self.config, account)?;
        apply_cost(&state, &mut assets, &cost, now)?;
        apply_reward(&state, &mut assets, &reward, now)?;

        // Commit point: nothing above touched shared state.
        let shop = state.shops.get_mut(shop_id).expect("shop checked");
        let stored_offer = shop.offers.get_mut(offer_id).expect("offer checked");
        stored_offer.sold = stored_offer
            .sold
            .checked_add(quantity)
            .ok_or(GameError::AmountOverflow)?;
        state
            .shops
            .record_purchase(shop_id.clone(), offer_id.clone(), account.clone(), quantity);
        save_assets(&mut state, account, assets, now);

        Ok(PurchaseReceipt {
            shop: shop_id.clone(),
            offer: offer_id.clone(),
            quantity,
            paid: cost,
            received: reward,
        })
    }

    pub fn purchased_quantity(&self, account: &AccountId, shop: &ShopId, offer: &OfferId) -> u64 {
        self.state.read().shops.purchased(shop, offer, account)
    }

    // =====================================================================
    // Trades
    // =====================================================================

    pub fn create_trade(&self, initiator: &AccountId, partner: &AccountId) -> Result<Trade> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, initiator)?;
        ensure_account_exists(&state, partner)?;
        check_region_open_for_activity(&state, initiator)?;
        check_region_open_for_activity(&state, partner)?;
        let expiry_ms = self
            .config
            .trade_expiry_secs
            .saturating_mul(1_000)
            .min(i64::MAX as u64) as i64;
        let expires_at_ms = now.saturating_add(expiry_ms);
        state
            .trades
            .create(initiator.clone(), partner.clone(), now, expires_at_ms)
    }

    pub fn trade(&self, id: &TradeId) -> Result<Trade> {
        self.state
            .read()
            .trades
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::TradeNotFound(id.clone()))
    }

    pub fn trades_for(&self, account: &AccountId) -> Result<Vec<Trade>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state.trades.for_account(account))
    }

    /// Replace one side of an open trade. Both confirmations are reset.
    ///
    /// Offered assets are checked against the account's current holdings; they
    /// remain usable elsewhere until the trade executes.
    pub fn set_trade_side(
        &self,
        account: &AccountId,
        trade_id: &TradeId,
        side: TradeSide,
    ) -> Result<Trade> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        check_region_open_for_activity(&state, account)?;
        let snapshot = state
            .trades
            .get(trade_id)
            .cloned()
            .ok_or_else(|| GameError::TradeNotFound(trade_id.clone()))?;
        ensure_trade_open(&mut state, &snapshot, now)?;
        if !snapshot.is_participant(account) {
            return Err(GameError::NotTradeParticipant(account.clone()));
        }

        for (item, quantity) in &side.items {
            if *quantity == 0 {
                return Err(GameError::InvalidQuantity(*quantity));
            }
            state.items.require(item)?;
            let available = state
                .inventories
                .get(account)
                .map(|inventory| inventory.count(item))
                .unwrap_or(0);
            if available < *quantity {
                return Err(GameError::InsufficientItems {
                    item: item.clone(),
                    needed: *quantity,
                    available,
                });
            }
        }
        for (currency, amount) in &side.currencies {
            if *amount == 0 {
                return Err(GameError::InvalidQuantity(*amount));
            }
            let available = state
                .wallets
                .get(account)
                .map(|wallet| wallet.currency_balance(currency))
                .unwrap_or(0);
            if available < *amount {
                return Err(GameError::InsufficientFunds {
                    currency: currency.clone(),
                    needed: *amount,
                    available,
                });
            }
        }

        state.trades.update_side(account, trade_id, side, now)?;
        Ok(state.trades.require(trade_id)?.clone())
    }

    /// Confirm a trade. The second confirmation executes it atomically.
    pub fn confirm_trade(&self, account: &AccountId, trade_id: &TradeId) -> Result<Trade> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        check_region_open_for_activity(&state, account)?;
        let snapshot = state
            .trades
            .get(trade_id)
            .cloned()
            .ok_or_else(|| GameError::TradeNotFound(trade_id.clone()))?;
        ensure_trade_open(&mut state, &snapshot, now)?;
        if !snapshot.is_participant(account) {
            return Err(GameError::NotTradeParticipant(account.clone()));
        }

        {
            let trade = state.trades.get_mut(trade_id).expect("trade checked");
            trade.set_confirmed(account, true);
            trade.updated_at_ms = now;
        }
        let ready = {
            let trade = state.trades.get(trade_id).expect("trade checked");
            trade.initiator_confirmed && trade.partner_confirmed
        };
        if !ready {
            return Ok(state.trades.require(trade_id)?.clone());
        }

        let trade = state.trades.require(trade_id)?.clone();
        match execute_trade(&mut state, &self.config, &trade, now) {
            Ok(()) => {
                let trade = state.trades.get_mut(trade_id).expect("trade checked");
                trade.state = TradeState::Completed;
                trade.updated_at_ms = now;
                Ok(trade.clone())
            }
            Err(error) => {
                let trade = state.trades.get_mut(trade_id).expect("trade checked");
                trade.initiator_confirmed = false;
                trade.partner_confirmed = false;
                trade.updated_at_ms = now;
                Err(error)
            }
        }
    }

    pub fn cancel_trade(&self, account: &AccountId, trade_id: &TradeId) -> Result<Trade> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let snapshot = state
            .trades
            .get(trade_id)
            .cloned()
            .ok_or_else(|| GameError::TradeNotFound(trade_id.clone()))?;
        ensure_trade_open(&mut state, &snapshot, now)?;
        if !snapshot.is_participant(account) {
            return Err(GameError::NotTradeParticipant(account.clone()));
        }
        let trade = state.trades.get_mut(trade_id).expect("trade checked");
        trade.state = TradeState::Cancelled;
        trade.updated_at_ms = now;
        Ok(trade.clone())
    }

    /// Mark overdue trades as expired.
    pub fn expire_trades(&self) -> Vec<TradeId> {
        let now = self.now_ms();
        self.state.write().trades.expire(now)
    }

    // =====================================================================
    // Collections
    // =====================================================================

    pub fn register_collection(&self, def: CollectionDef) -> Result<()> {
        self.state.write().collections.register(def)
    }

    pub fn collection(&self, id: &CollectionId) -> Result<CollectionDef> {
        self.state
            .read()
            .collections
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::CollectionNotFound(id.clone()))
    }

    pub fn collections(&self) -> Vec<CollectionDef> {
        self.state.read().collections.iter().cloned().collect()
    }

    pub fn collection_progress(
        &self,
        account: &AccountId,
        collection: &CollectionId,
    ) -> Result<CollectionProgress> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        let def = state.collections.require(collection)?;
        Ok(collection_progress_in_state(&state, account, def))
    }

    /// Claim one milestone. Progress is computed from live inventory / skins /
    /// achievements, then the reward is granted atomically.
    pub fn claim_collection_milestone(
        &self,
        account: &AccountId,
        collection: &CollectionId,
        milestone: usize,
    ) -> Result<Reward> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.collections.require(collection)?.clone();
        let progress = collection_progress_in_state(&state, account, &def);
        let milestone_def = def.milestone(milestone)?.clone();
        if progress.collected < milestone_def.required {
            return Err(GameError::MilestoneLocked);
        }
        if state.collections.is_claimed(account, collection, milestone) {
            return Err(GameError::AlreadyClaimed);
        }

        let mut assets = load_assets(&state, &self.config, account)?;
        apply_reward(&state, &mut assets, &milestone_def.reward, now)?;
        state
            .collections
            .mark_claimed(account, collection, milestone);
        save_assets(&mut state, account, assets, now);
        Ok(milestone_def.reward)
    }

    // =====================================================================
    // Achievements
    // =====================================================================

    pub fn register_achievement(&self, def: AchievementDef) -> Result<()> {
        self.state.write().achievements.register(def)
    }

    pub fn achievement(&self, id: &AchievementId) -> Result<AchievementDef> {
        self.state
            .read()
            .achievements
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::AchievementNotFound(id.clone()))
    }

    pub fn achievements(&self) -> Vec<AchievementDef> {
        self.state.read().achievements.iter().cloned().collect()
    }

    /// Feed one named gameplay event to **both** achievements and active tasks.
    ///
    /// Returns what was completed. Item / skin / achievement objectives are
    /// re-evaluated as part of this call, so a task may complete here even if
    /// its objectives are not metric-based.
    pub fn record_event(
        &self,
        account: &AccountId,
        metric: &str,
        amount: u64,
    ) -> Result<MetricOutcome> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let achievements = state.achievements.record_metric(account, metric, amount);
        let tasks = record_task_metric_in_state(&mut state, account, metric, amount, now);
        Ok(MetricOutcome {
            achievements,
            tasks,
        })
    }

    /// Feed a named counter and return the achievements newly completed.
    ///
    /// Task progress is advanced as well; use [`Game::record_event`] when the
    /// caller also needs the ids of tasks completed by the event.
    pub fn record_metric(
        &self,
        account: &AccountId,
        metric: &str,
        amount: u64,
    ) -> Result<Vec<AchievementId>> {
        Ok(self.record_event(account, metric, amount)?.achievements)
    }

    pub fn achievement_view(
        &self,
        account: &AccountId,
        achievement: &AchievementId,
    ) -> Result<AchievementView> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        let def = state.achievements.require(achievement)?;
        Ok(achievement_view_in_state(&state, account, def))
    }

    pub fn achievements_for(&self, account: &AccountId) -> Result<Vec<AchievementView>> {
        let state = self.state.read();
        ensure_account_exists(&state, account)?;
        Ok(state
            .achievements
            .iter()
            .filter(|def| !def.hidden || state.achievements.is_completed(account, &def.id))
            .map(|def| achievement_view_in_state(&state, account, def))
            .collect())
    }

    pub fn claim_achievement(
        &self,
        account: &AccountId,
        achievement: &AchievementId,
    ) -> Result<Reward> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.achievements.require(achievement)?.clone();
        if !state.achievements.is_completed(account, achievement) {
            return Err(GameError::AchievementNotCompleted);
        }
        if state.achievements.is_claimed(account, achievement) {
            return Err(GameError::AchievementAlreadyClaimed);
        }

        let mut assets = load_assets(&state, &self.config, account)?;
        apply_reward(&state, &mut assets, &def.reward, now)?;
        state.achievements.mark_claimed(account, achievement);
        save_assets(&mut state, account, assets, now);
        Ok(def.reward)
    }

    // =====================================================================
    // Tasks
    // =====================================================================

    pub fn register_task(&self, def: TaskDef) -> Result<()> {
        self.state.write().tasks.register(def)
    }

    pub fn task_definition(&self, id: &TaskId) -> Result<TaskDef> {
        self.state
            .read()
            .tasks
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::TaskNotFound(id.clone()))
    }

    pub fn task_definitions(&self) -> Vec<TaskDef> {
        self.state.read().tasks.iter().cloned().collect()
    }

    /// Current state of every visible task for an account.
    ///
    /// Hidden tasks appear only once the account has progress on them.
    pub fn task_views(&self, account: &AccountId) -> Result<Vec<TaskView>> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        refresh_account_tasks(&mut state, account, now);
        Ok(task_views_in_state(&state, account, now))
    }

    pub fn task_view(&self, account: &AccountId, id: &TaskId) -> Result<TaskView> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.tasks.require(id)?.clone();
        refresh_account_tasks(&mut state, account, now);
        Ok(task_view_in_state(&state, account, &def, now))
    }

    /// Tasks whose requirements are met and that are not accepted yet.
    pub fn available_tasks(&self, account: &AccountId) -> Result<Vec<TaskView>> {
        Ok(self
            .task_views(account)?
            .into_iter()
            .filter(|view| view.status == TaskStatus::Available)
            .collect())
    }

    pub fn active_tasks(&self, account: &AccountId) -> Result<Vec<TaskView>> {
        Ok(self
            .task_views(account)?
            .into_iter()
            .filter(|view| view.status == TaskStatus::Active)
            .collect())
    }

    /// Completed tasks whose reward has not been claimed yet.
    pub fn completed_tasks(&self, account: &AccountId) -> Result<Vec<TaskView>> {
        Ok(self
            .task_views(account)?
            .into_iter()
            .filter(|view| view.status == TaskStatus::Completed)
            .collect())
    }

    /// Accept a task. Accepting an expired or repeatable-ready task resets its
    /// counters and starts a new cycle.
    pub fn accept_task(&self, account: &AccountId, id: &TaskId) -> Result<TaskView> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        check_region_open_for_activity(&state, account)?;
        let def = state.tasks.require(id)?.clone();
        if !task_unlocked_for(&state, account, &def, now) {
            return Err(GameError::TaskLocked(id.clone()));
        }

        if let Some(existing) = state.tasks.progress(account, id).cloned() {
            if let Some(claimed_at) = existing.claimed_at_ms {
                match def.repeat {
                    TaskRepeat::Once => return Err(GameError::TaskAlreadyClaimed(id.clone())),
                    TaskRepeat::Cooldown { duration_ms } => {
                        let available_at_ms = claimed_at.saturating_add(duration_ms);
                        if now < available_at_ms {
                            return Err(GameError::TaskOnCooldown {
                                task: id.clone(),
                                available_at_ms,
                            });
                        }
                    }
                }
            } else if existing.completed_at_ms.is_some() {
                return Err(GameError::TaskAlreadyCompleted(id.clone()));
            } else if !existing.is_expired(now) {
                return Err(GameError::TaskAlreadyAccepted(id.clone()));
            }
        }

        let expires_at_ms = def.time_limit_ms.map(|limit| now.saturating_add(limit));
        state
            .tasks
            .set_progress(account, id.clone(), TaskProgress::new(now, expires_at_ms));
        refresh_account_tasks(&mut state, account, now);
        Ok(task_view_in_state(&state, account, &def, now))
    }

    /// Give up an accepted (possibly completed) task. Claimed tasks cannot be
    /// abandoned. The task becomes available again if requirements still hold.
    pub fn abandon_task(&self, account: &AccountId, id: &TaskId) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state.tasks.require(id)?;
        let progress = state
            .tasks
            .progress(account, id)
            .cloned()
            .ok_or_else(|| GameError::TaskNotAccepted(id.clone()))?;
        if progress.claimed_at_ms.is_some() {
            return Err(GameError::TaskAlreadyClaimed(id.clone()));
        }
        state.tasks.remove_progress(account, id);
        Ok(())
    }

    /// Claim a completed task's reward atomically.
    pub fn claim_task(&self, account: &AccountId, id: &TaskId) -> Result<Reward> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let def = state.tasks.require(id)?.clone();
        // Items / skins / achievements may have satisfied objectives since the
        // task was last inspected.
        refresh_account_tasks(&mut state, account, now);
        let progress = state
            .tasks
            .progress(account, id)
            .cloned()
            .ok_or_else(|| GameError::TaskNotAccepted(id.clone()))?;
        if progress.claimed_at_ms.is_some() {
            return Err(GameError::TaskAlreadyClaimed(id.clone()));
        }
        if progress.completed_at_ms.is_none() {
            return Err(GameError::TaskNotCompleted(id.clone()));
        }

        let mut assets = load_assets(&state, &self.config, account)?;
        apply_reward(&state, &mut assets, &def.reward, now)?;
        if let Some(progress) = state.tasks.progress_mut(account, id) {
            progress.claimed_at_ms = Some(now);
        }
        save_assets(&mut state, account, assets, now);
        Ok(def.reward)
    }

    // =====================================================================
    // Leaderboards
    // =====================================================================

    pub fn register_leaderboard(&self, board: Leaderboard) -> Result<()> {
        let mut state = self.state.write();
        if state.leaderboards.contains_key(&board.id) {
            return Err(GameError::LeaderboardAlreadyExists(board.id));
        }
        state.leaderboards.insert(board.id.clone(), board);
        Ok(())
    }

    pub fn leaderboard(&self, id: &LeaderboardId) -> Result<Leaderboard> {
        self.state
            .read()
            .leaderboards
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::LeaderboardNotFound(id.clone()))
    }

    pub fn leaderboards(&self) -> Vec<Leaderboard> {
        self.state.read().leaderboards.values().cloned().collect()
    }

    pub fn set_leaderboard_score(
        &self,
        board: &LeaderboardId,
        account: &AccountId,
        score: i64,
    ) -> Result<()> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let board = state
            .leaderboards
            .get_mut(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?;
        board.set_score(account.clone(), score, now);
        Ok(())
    }

    pub fn add_leaderboard_score(
        &self,
        board: &LeaderboardId,
        account: &AccountId,
        delta: i64,
    ) -> Result<i64> {
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        let board = state
            .leaderboards
            .get_mut(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?;
        board.add_score(account, delta, now)
    }

    pub fn leaderboard_score(
        &self,
        board: &LeaderboardId,
        account: &AccountId,
    ) -> Result<Option<i64>> {
        Ok(self
            .state
            .read()
            .leaderboards
            .get(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?
            .score_of(account))
    }

    pub fn leaderboard_rank(
        &self,
        board: &LeaderboardId,
        account: &AccountId,
    ) -> Result<Option<usize>> {
        Ok(self
            .state
            .read()
            .leaderboards
            .get(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?
            .rank_of(account))
    }

    pub fn leaderboard_top(&self, board: &LeaderboardId, limit: usize) -> Result<Vec<ScoreEntry>> {
        Ok(self
            .state
            .read()
            .leaderboards
            .get(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?
            .top(limit))
    }

    pub fn remove_leaderboard_score(
        &self,
        board: &LeaderboardId,
        account: &AccountId,
    ) -> Result<bool> {
        let mut state = self.state.write();
        let board = state
            .leaderboards
            .get_mut(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?;
        Ok(board.remove(account))
    }

    pub fn reset_leaderboard_season(&self, board: &LeaderboardId) -> Result<u64> {
        let now = self.now_ms();
        let mut state = self.state.write();
        let board = state
            .leaderboards
            .get_mut(board)
            .ok_or_else(|| GameError::LeaderboardNotFound(board.clone()))?;
        Ok(board.reset_season(now))
    }

    // =====================================================================
    // Announcements
    // =====================================================================

    pub fn publish_announcement(&self, draft: AnnouncementDraft) -> Result<Announcement> {
        let now = self.now_ms();
        let mut state = self.state.write();
        if let Some(regions) = &draft.regions {
            for region in regions {
                if !state.regions.contain(region) {
                    return Err(GameError::RegionNotFound(region.clone()));
                }
            }
        }
        state.announcements.publish(draft, now)
    }

    pub fn announcement(&self, id: &AnnouncementId) -> Result<Announcement> {
        self.state
            .read()
            .announcements
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::AnnouncementNotFound(id.clone()))
    }

    pub fn announcements(&self) -> Vec<Announcement> {
        self.state.read().announcements.iter().cloned().collect()
    }

    pub fn active_announcements(&self, account: &AccountId) -> Result<Vec<Announcement>> {
        let now = self.now_ms();
        let state = self.state.read();
        let account = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        Ok(state.announcements.active(&account.region, now))
    }

    pub fn active_announcements_for_region(&self, region: &RegionId) -> Result<Vec<Announcement>> {
        let now = self.now_ms();
        let state = self.state.read();
        state.regions.require(region)?;
        Ok(state.announcements.active(region, now))
    }

    pub fn unread_announcements(&self, account: &AccountId) -> Result<Vec<Announcement>> {
        let now = self.now_ms();
        let state = self.state.read();
        let account_info = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
        Ok(state
            .announcements
            .unread(account, &account_info.region, now))
    }

    pub fn mark_announcement_read(
        &self,
        account: &AccountId,
        announcement: &AnnouncementId,
    ) -> Result<()> {
        let mut state = self.state.write();
        ensure_account_exists(&state, account)?;
        state.announcements.mark_read(account, announcement)
    }

    pub fn is_announcement_read(&self, account: &AccountId, announcement: &AnnouncementId) -> bool {
        self.state
            .read()
            .announcements
            .is_read(account, announcement)
    }

    pub fn set_announcement_enabled(
        &self,
        announcement: &AnnouncementId,
        enabled: bool,
    ) -> Result<()> {
        self.state
            .write()
            .announcements
            .set_enabled(announcement, enabled)
    }

    pub fn remove_announcement(&self, announcement: &AnnouncementId) -> Option<Announcement> {
        self.state.write().announcements.remove(announcement)
    }

    // =====================================================================
    // NPCs
    // =====================================================================

    pub fn register_npc(&self, npc: NpcDef) -> Result<()> {
        let mut state = self.state.write();
        if let Some(room) = &npc.room {
            let room = state
                .rooms
                .get(room)
                .ok_or_else(|| GameError::RoomNotFound(room.clone()))?;
            if let Some(region) = &npc.region {
                if *region != room.region {
                    return Err(GameError::InvalidInput(
                        "npc region does not match its room's region".into(),
                    ));
                }
            }
        } else if let Some(region) = &npc.region {
            state.regions.require(region)?;
        }
        state.npcs.register(npc)
    }

    pub fn npc(&self, id: &NpcId) -> Result<NpcDef> {
        self.state
            .read()
            .npcs
            .get(id)
            .cloned()
            .ok_or_else(|| GameError::NpcNotFound(id.clone()))
    }

    pub fn npcs(&self) -> Vec<NpcDef> {
        self.state.read().npcs.iter().cloned().collect()
    }

    pub fn set_npc_enabled(&self, id: &NpcId, enabled: bool) -> Result<()> {
        self.state.write().npcs.set_enabled(id, enabled)
    }

    pub fn remove_npc(&self, id: &NpcId) -> Option<NpcDef> {
        self.state.write().npcs.remove(id)
    }

    /// Build the context for an NPC turn, enforcing channel access.
    pub fn npc_context(
        &self,
        npc_id: &NpcId,
        account: &AccountId,
        message: &str,
    ) -> Result<NpcContext> {
        let now = self.now_ms();
        let message = message.trim();
        if message.is_empty() {
            return Err(GameError::EmptyContent);
        }
        if message.chars().count() > self.config.max_chat_message_len {
            return Err(GameError::ContentTooLong {
                max: self.config.max_chat_message_len,
            });
        }

        let state = self.state.read();
        let account_info = state
            .accounts
            .get(account)
            .ok_or_else(|| GameError::AccountNotFound(account.clone()))?
            .clone();
        let npc = state.npcs.require(npc_id)?.clone();
        if !npc.enabled {
            return Err(GameError::NpcDisabled(npc_id.clone()));
        }
        let channel = npc.channel();

        let mut nearby_players = Vec::new();
        match &channel {
            ChatChannel::Room(room_id) => {
                let room = state
                    .rooms
                    .get(room_id)
                    .ok_or_else(|| GameError::RoomNotFound(room_id.clone()))?;
                if !room.contains(account) {
                    return Err(GameError::ChannelAccessDenied(channel));
                }
                nearby_players = room
                    .players
                    .iter()
                    .take(self.config.npc_nearby_limit)
                    .filter_map(|id| state.accounts.get(id))
                    .map(|account| NearbyPlayer {
                        account: account.id.clone(),
                        display_name: account.display_name.clone(),
                        level: account.level,
                    })
                    .collect();
            }
            ChatChannel::Region(region_id) => {
                if account_info.region != *region_id {
                    return Err(GameError::ChannelAccessDenied(channel));
                }
                state.regions.require(region_id)?;
                nearby_players = state
                    .regions
                    .require(region_id)?
                    .players
                    .iter()
                    .take(self.config.npc_nearby_limit)
                    .filter_map(|id| state.accounts.get(id))
                    .map(|account| NearbyPlayer {
                        account: account.id.clone(),
                        display_name: account.display_name.clone(),
                        level: account.level,
                    })
                    .collect();
            }
            ChatChannel::Global => {}
            ChatChannel::Private { .. } => {
                return Err(GameError::ChannelAccessDenied(channel));
            }
        }

        let recent_messages = state
            .chat
            .recent(&channel, self.config.npc_context_messages);
        Ok(NpcContext {
            npc,
            channel,
            speaker: account.clone(),
            speaker_name: account_info.display_name,
            speaker_level: account_info.level,
            message: message.to_owned(),
            recent_messages,
            nearby_players,
            now_ms: now,
        })
    }

    /// Apply a complete NPC turn atomically.
    ///
    /// Rewards, leaderboard updates, the player's message and all NPC messages
    /// are committed under one write lock. Every action and message is
    /// validated first, so a failing action leaves no partial side effects.
    ///
    /// [`crate::npc::NpcRuntime::converse`] uses this method after the agent
    /// responds; hosts may call it directly when they generate actions
    /// themselves.
    pub fn apply_npc_turn(
        &self,
        npc_id: &NpcId,
        speaker: &AccountId,
        channel: ChatChannel,
        player_message: &str,
        actions: &[NpcAction],
        npc_messages: &[String],
    ) -> Result<NpcTurn> {
        let channel = normalize_channel(channel);
        let now = self.now_ms();
        let mut state = self.state.write();
        ensure_account_exists(&state, speaker)?;
        check_region_open_for_activity(&state, speaker)?;
        let npc = state.npcs.require(npc_id)?.clone();
        if !npc.enabled {
            return Err(GameError::NpcDisabled(npc_id.clone()));
        }
        if npc.channel() != channel {
            return Err(GameError::ChannelAccessDenied(channel));
        }
        if let Some(room) = &npc.room {
            state.rooms.require(room)?;
        }
        if let Some(region) = &npc.region {
            state.regions.require(region)?;
        }
        check_channel_access(&state, speaker, &channel)?;

        let max_len = self.config.max_chat_message_len;
        validate_chat_body(player_message, max_len)?;
        for message in npc_messages {
            validate_chat_body(message, max_len)?;
        }

        let allowed_accounts = npc_allowed_accounts(&state, &npc, speaker);
        let mut applied_actions = 0usize;
        for action in actions {
            match action {
                NpcAction::Say { text } => validate_chat_body(text, max_len)?,
                NpcAction::GiveReward { account, .. } => {
                    if !allowed_accounts.contains(account) {
                        return Err(GameError::InvalidInput(format!(
                            "npc reward target {account} is not in the npc context"
                        )));
                    }
                    ensure_account_exists(&state, account)?;
                    applied_actions += 1;
                }
                NpcAction::SetScore {
                    leaderboard,
                    account,
                    ..
                } => {
                    if !allowed_accounts.contains(account) {
                        return Err(GameError::InvalidInput(format!(
                            "npc score target {account} is not in the npc context"
                        )));
                    }
                    ensure_account_exists(&state, account)?;
                    if !state.leaderboards.contains_key(leaderboard) {
                        return Err(GameError::LeaderboardNotFound(leaderboard.clone()));
                    }
                    applied_actions += 1;
                }
            }
        }

        // All fallible action work happens on cloned assets first. Nothing
        // below this point can fail after a shared mutation.
        let mut assets_by_account: BTreeMap<AccountId, AccountAssets> = BTreeMap::new();
        for action in actions {
            if let NpcAction::GiveReward { account, reward } = action {
                if !assets_by_account.contains_key(account) {
                    assets_by_account
                        .insert(account.clone(), load_assets(&state, &self.config, account)?);
                }
                apply_reward(
                    &state,
                    assets_by_account
                        .get_mut(account)
                        .expect("assets inserted above"),
                    reward,
                    now,
                )?;
            }
        }

        let player_chat = state.chat.post(
            channel.clone(),
            ChatAuthor::account(speaker.clone()),
            player_message,
            BTreeMap::new(),
            now,
            max_len,
        )?;
        let mut npc_chat_messages = Vec::with_capacity(npc_messages.len());
        for message in npc_messages {
            npc_chat_messages.push(state.chat.post(
                channel.clone(),
                ChatAuthor::npc(npc_id.clone()),
                message,
                BTreeMap::new(),
                now,
                max_len,
            )?);
        }

        for (account, assets) in assets_by_account {
            save_assets(&mut state, &account, assets, now);
        }
        for action in actions {
            if let NpcAction::SetScore {
                leaderboard,
                account,
                score,
            } = action
            {
                if let Some(board) = state.leaderboards.get_mut(leaderboard) {
                    board.set_score(account.clone(), *score, now);
                }
            }
        }

        Ok(NpcTurn {
            channel,
            player_message: player_chat,
            npc_messages: npc_chat_messages,
            applied_actions,
        })
    }
}

// =========================================================================
// Internal state helpers
// =========================================================================

#[derive(Clone)]
struct AccountAssets {
    wallet: Wallet,
    inventory: Inventory,
    owned_skins: OwnedSkins,
    xp: u64,
}

fn create_account_locked(
    state: &mut GameState,
    id: AccountId,
    display_name: String,
    region: RegionId,
    now_ms: i64,
    config: &GameConfig,
) -> Result<Account> {
    if state.accounts.contains_key(&id) {
        return Err(GameError::AccountAlreadyExists(id));
    }
    check_region_accepts(state, &region, &id)?;
    let account = Account::new(id.clone(), display_name, region.clone(), now_ms);
    state.regions.add_player(&region, &id)?;
    state.accounts.insert(id.clone(), account.clone());
    state.wallets.insert(id.clone(), Wallet::new());
    state.inventories.insert(
        id.clone(),
        Inventory::new(config.default_inventory_capacity),
    );
    Ok(account)
}

fn ensure_account_exists(state: &GameState, account: &AccountId) -> Result<()> {
    if state.accounts.contains_key(account) {
        Ok(())
    } else {
        Err(GameError::AccountNotFound(account.clone()))
    }
}

/// Reject new player activity for accounts whose region is `Closed`.
///
/// `Maintenance` keeps existing players active. Read-only queries, reward
/// claims and profile edits are deliberately not gated by this check.
fn check_region_open_for_activity(state: &GameState, account: &AccountId) -> Result<()> {
    let account_info = state
        .accounts
        .get(account)
        .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
    let region = state.regions.require(&account_info.region)?;
    if region.status == RegionStatus::Closed {
        return Err(GameError::RegionNotOpen(account_info.region.clone()));
    }
    Ok(())
}

/// Activities performed by accounts already assigned to a region.
///
/// `Maintenance` keeps existing players playing but blocks new entry
/// (`check_region_accepts`); `Closed` blocks everything.
fn check_region_activity(state: &GameState, region_id: &RegionId) -> Result<()> {
    let region = state.regions.require(region_id)?;
    if region.status == RegionStatus::Closed {
        return Err(GameError::RegionNotOpen(region_id.clone()));
    }
    Ok(())
}

fn check_region_accepts(
    state: &GameState,
    region_id: &RegionId,
    account: &AccountId,
) -> Result<()> {
    let region = state.regions.require(region_id)?;
    if region.status != RegionStatus::Open {
        return Err(GameError::RegionNotOpen(region_id.clone()));
    }
    if !region.players.contains(account) && region.is_full() {
        return Err(GameError::RegionFull(region_id.clone()));
    }
    Ok(())
}

fn normalize_channel(channel: ChatChannel) -> ChatChannel {
    match channel {
        ChatChannel::Private { a, b } => ChatChannel::private(a, b),
        other => other,
    }
}

/// Validate that a channel points at existing entities. Used by operator
/// messages, which have no account identity to derive access from.
fn validate_channel_targets(state: &GameState, channel: &ChatChannel) -> Result<()> {
    match channel {
        ChatChannel::Global => Ok(()),
        ChatChannel::Region(region) => state.regions.require(region).map(|_| ()),
        ChatChannel::Room(room) => state.rooms.require(room).map(|_| ()),
        ChatChannel::Private { a, b } => {
            if a == b {
                return Err(GameError::InvalidInput(
                    "cannot open a private chat with yourself".into(),
                ));
            }
            ensure_account_exists(state, a)?;
            ensure_account_exists(state, b)
        }
    }
}

fn validate_chat_body(body: &str, max_len: usize) -> Result<()> {
    let body = body.trim();
    if body.is_empty() {
        return Err(GameError::EmptyContent);
    }
    if body.chars().count() > max_len {
        return Err(GameError::ContentTooLong { max: max_len });
    }
    Ok(())
}

/// Accounts an NPC may act upon: the speaker plus players in its room/region.
fn npc_allowed_accounts(
    state: &GameState,
    npc: &NpcDef,
    speaker: &AccountId,
) -> BTreeSet<AccountId> {
    let mut allowed = BTreeSet::new();
    allowed.insert(speaker.clone());
    match npc.channel() {
        ChatChannel::Room(room_id) => {
            if let Some(room) = state.rooms.get(&room_id) {
                allowed.extend(room.players.iter().cloned());
            }
        }
        ChatChannel::Region(region_id) => {
            if let Some(region) = state.regions.get(&region_id) {
                allowed.extend(region.players.iter().cloned());
            }
        }
        ChatChannel::Global | ChatChannel::Private { .. } => {}
    }
    allowed
}

fn check_channel_access(
    state: &GameState,
    account: &AccountId,
    channel: &ChatChannel,
) -> Result<()> {
    let account_info = state
        .accounts
        .get(account)
        .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
    match channel {
        ChatChannel::Global => Ok(()),
        ChatChannel::Region(region) => {
            if account_info.region == *region {
                Ok(())
            } else {
                Err(GameError::ChannelAccessDenied(channel.clone()))
            }
        }
        ChatChannel::Room(room) => {
            let room = state
                .rooms
                .get(room)
                .ok_or_else(|| GameError::RoomNotFound(room.clone()))?;
            if room.contains(account) {
                Ok(())
            } else {
                Err(GameError::ChannelAccessDenied(channel.clone()))
            }
        }
        ChatChannel::Private { a, b } => {
            if a == b {
                return Err(GameError::InvalidInput(
                    "cannot open a private chat with yourself".into(),
                ));
            }
            if account != a && account != b {
                return Err(GameError::ChannelAccessDenied(channel.clone()));
            }
            let other = if account == a { b } else { a };
            if !state.accounts.contains_key(other) {
                return Err(GameError::AccountNotFound(other.clone()));
            }
            if state.friends.blocked_either(a, b) {
                return Err(GameError::Blocked);
            }
            Ok(())
        }
    }
}

fn validate_skin_slot(
    state: &GameState,
    account: &AccountId,
    skin: Option<&SkinId>,
    expected: &SkinCategory,
) -> Result<()> {
    let Some(skin) = skin else {
        return Ok(());
    };
    let def = state.skins.require(skin)?;
    if !state.cosmetics.owns(account, skin) {
        return Err(GameError::SkinNotOwned(skin.clone()));
    }
    if &def.category != expected {
        return Err(GameError::SkinCategoryMismatch {
            skin: skin.clone(),
            expected: expected.clone(),
            actual: def.category.clone(),
        });
    }
    Ok(())
}

fn load_assets(
    state: &GameState,
    config: &GameConfig,
    account: &AccountId,
) -> Result<AccountAssets> {
    let account_info = state
        .accounts
        .get(account)
        .ok_or_else(|| GameError::AccountNotFound(account.clone()))?;
    Ok(AccountAssets {
        wallet: state.wallets.get(account).cloned().unwrap_or_default(),
        inventory: state
            .inventories
            .get(account)
            .cloned()
            .unwrap_or_else(|| Inventory::new(config.default_inventory_capacity)),
        owned_skins: state.cosmetics.owned(account).cloned().unwrap_or_default(),
        xp: account_info.xp,
    })
}

fn save_assets(state: &mut GameState, account: &AccountId, assets: AccountAssets, now_ms: i64) {
    state.wallets.insert(account.clone(), assets.wallet);
    state.inventories.insert(account.clone(), assets.inventory);
    *state.cosmetics.owned_mut(account) = assets.owned_skins;
    if let Some(account_info) = state.accounts.get_mut(account) {
        account_info.xp = assets.xp;
    }
    // Any item/skin/achievement objective that became satisfied as part of
    // this commit is stamped now, while it is still observable.
    refresh_account_tasks(state, account, now_ms);
}

fn apply_reward(
    state: &GameState,
    assets: &mut AccountAssets,
    reward: &Reward,
    now_ms: i64,
) -> Result<()> {
    for grant in &reward.currencies {
        assets.wallet.add_currency(&grant.currency, grant.amount)?;
    }
    for grant in &reward.resources {
        let def = state.resources.require(&grant.resource)?;
        assets.wallet.add_resource(def, grant.amount, now_ms)?;
    }
    for grant in &reward.items {
        if grant.quantity == 0 {
            continue;
        }
        let def = state.items.require(&grant.item)?;
        assets.inventory.try_add(def, grant.quantity)?;
    }
    for skin in &reward.skins {
        state.skins.require(skin)?;
        assets.owned_skins.grant(skin.clone());
    }
    assets.xp = assets
        .xp
        .checked_add(reward.xp)
        .ok_or(GameError::AmountOverflow)?;
    Ok(())
}

fn apply_cost(
    state: &GameState,
    assets: &mut AccountAssets,
    cost: &Cost,
    now_ms: i64,
) -> Result<()> {
    assets.wallet.try_spend_currencies(&cost.currencies)?;
    assets
        .wallet
        .try_spend_resources(&state.resources, &cost.resources, now_ms)?;
    Ok(())
}

fn ensure_trade_open(state: &mut GameState, trade: &Trade, now_ms: i64) -> Result<()> {
    if trade.state != TradeState::Open {
        return Err(GameError::TradeNotOpen);
    }
    if trade.is_expired(now_ms) {
        if let Some(stored) = state.trades.get_mut(&trade.id) {
            stored.state = TradeState::Expired;
            stored.updated_at_ms = now_ms;
        }
        return Err(GameError::TradeExpired);
    }
    Ok(())
}

fn execute_trade(
    state: &mut GameState,
    config: &GameConfig,
    trade: &Trade,
    now_ms: i64,
) -> Result<()> {
    if trade.initiator_side.is_empty() && trade.partner_side.is_empty() {
        return Err(GameError::EmptyTrade);
    }

    let mut initiator = load_assets(state, config, &trade.initiator)?;
    let mut partner = load_assets(state, config, &trade.partner)?;

    // Remove outgoing assets first so free space can be used for incoming ones.
    initiator
        .inventory
        .try_remove_many(&trade.initiator_side.items)?;
    partner
        .inventory
        .try_remove_many(&trade.partner_side.items)?;
    initiator
        .wallet
        .try_spend_currencies(&currency_grants(&trade.initiator_side.currencies))?;
    partner
        .wallet
        .try_spend_currencies(&currency_grants(&trade.partner_side.currencies))?;

    for (item, quantity) in &trade.partner_side.items {
        let def = state.items.require(item)?;
        initiator.inventory.try_add(def, *quantity)?;
    }
    for (item, quantity) in &trade.initiator_side.items {
        let def = state.items.require(item)?;
        partner.inventory.try_add(def, *quantity)?;
    }
    initiator
        .wallet
        .add_currencies(&currency_grants(&trade.partner_side.currencies))?;
    partner
        .wallet
        .add_currencies(&currency_grants(&trade.initiator_side.currencies))?;

    save_assets(state, &trade.initiator, initiator, now_ms);
    save_assets(state, &trade.partner, partner, now_ms);
    Ok(())
}

fn currency_grants(currencies: &BTreeMap<CurrencyId, u64>) -> Vec<CurrencyGrant> {
    currencies
        .iter()
        .map(|(currency, amount)| CurrencyGrant::new(currency.clone(), *amount))
        .collect()
}

fn collection_progress_in_state(
    state: &GameState,
    account: &AccountId,
    def: &CollectionDef,
) -> CollectionProgress {
    let collected = def
        .entries
        .iter()
        .filter(|entry| match entry {
            Collectible::Item(item) => state
                .inventories
                .get(account)
                .is_some_and(|inventory| inventory.count(item) > 0),
            Collectible::Skin(skin) => state.cosmetics.owns(account, skin),
            Collectible::Achievement(achievement) => {
                state.achievements.is_completed(account, achievement)
            }
        })
        .count();
    let milestones = def
        .milestones
        .iter()
        .enumerate()
        .map(|(index, milestone)| MilestoneProgress {
            index,
            required: milestone.required,
            unlocked: collected >= milestone.required,
            claimed: state.collections.is_claimed(account, &def.id, index),
        })
        .collect();
    CollectionProgress {
        collection: def.id.clone(),
        collected,
        total: def.entries.len(),
        milestones,
    }
}

fn achievement_view_in_state(
    state: &GameState,
    account: &AccountId,
    def: &AchievementDef,
) -> AchievementView {
    AchievementView {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        metric: def.metric.clone(),
        progress: state.achievements.progress_of(account, def),
        target: def.target,
        completed: state.achievements.is_completed(account, &def.id),
        claimed: state.achievements.is_claimed(account, &def.id),
        hidden: def.hidden,
    }
}

// =========================================================================
// Task helpers
// =========================================================================

fn task_views_in_state(state: &GameState, account: &AccountId, now_ms: i64) -> Vec<TaskView> {
    let mut views: Vec<TaskView> = state
        .tasks
        .iter()
        .filter(|def| !def.hidden || state.tasks.progress(account, &def.id).is_some())
        .map(|def| task_view_in_state(state, account, def, now_ms))
        .collect();
    views.sort_by(|a, b| a.category.cmp(&b.category).then_with(|| a.id.cmp(&b.id)));
    views
}

fn task_view_in_state(
    state: &GameState,
    account: &AccountId,
    def: &TaskDef,
    now_ms: i64,
) -> TaskView {
    let progress = state.tasks.progress(account, &def.id);
    // Once the cooldown is over, the task presents a new cycle: counter
    // progress and all cycle timestamps from the claimed run must not leak into
    // the view. Item/skin/achievement objectives remain live by design.
    let fresh_cycle = progress.is_some_and(|progress| task_cycle_fresh(def, progress, now_ms));
    let cycle = if fresh_cycle { None } else { progress };

    let empty_counters = BTreeMap::new();
    let counters = cycle
        .map(|progress| &progress.counters)
        .unwrap_or(&empty_counters);
    let objectives = def
        .objectives
        .iter()
        .map(|objective| {
            let (progress, target, complete) =
                task_objective_progress(state, account, counters, objective);
            TaskObjectiveView {
                objective: objective.clone(),
                progress,
                target,
                complete,
            }
        })
        .collect();

    TaskView {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        category: def.category.clone(),
        status: task_status_in_state(state, account, def, now_ms),
        objectives,
        reward: def.reward.clone(),
        accepted_at_ms: cycle.map(|progress| progress.accepted_at_ms),
        expires_at_ms: cycle.and_then(|progress| progress.expires_at_ms),
        completed_at_ms: cycle.and_then(|progress| progress.completed_at_ms),
        claimed_at_ms: cycle.and_then(|progress| progress.claimed_at_ms),
        next_available_at_ms: cycle.and_then(|progress| progress.next_available_at(def.repeat)),
        repeat: def.repeat,
        min_level: def.min_level,
        requires: def.requires.clone(),
        regions: def.regions.clone(),
        starts_at_ms: def.starts_at_ms,
        ends_at_ms: def.ends_at_ms,
        hidden: def.hidden,
    }
}

fn task_cycle_fresh(def: &TaskDef, progress: &TaskProgress, now_ms: i64) -> bool {
    match (progress.claimed_at_ms, def.repeat.cooldown_ms()) {
        (Some(claimed_at), Some(duration)) => now_ms >= claimed_at.saturating_add(duration),
        _ => false,
    }
}

fn task_status_in_state(
    state: &GameState,
    account: &AccountId,
    def: &TaskDef,
    now_ms: i64,
) -> TaskStatus {
    let Some(progress) = state.tasks.progress(account, &def.id) else {
        return if task_unlocked_for(state, account, def, now_ms) {
            TaskStatus::Available
        } else {
            TaskStatus::Locked
        };
    };

    if let Some(claimed_at) = progress.claimed_at_ms {
        if let TaskRepeat::Cooldown { duration_ms } = def.repeat {
            let available_at = claimed_at.saturating_add(duration_ms);
            if now_ms >= available_at {
                return if task_unlocked_for(state, account, def, now_ms) {
                    TaskStatus::Available
                } else {
                    TaskStatus::Locked
                };
            }
        }
        return TaskStatus::Claimed;
    }
    if progress.completed_at_ms.is_some() {
        return TaskStatus::Completed;
    }
    if progress.is_expired(now_ms) {
        return TaskStatus::Expired;
    }
    TaskStatus::Active
}

fn task_unlocked_for(state: &GameState, account: &AccountId, def: &TaskDef, now_ms: i64) -> bool {
    let Some(account_info) = state.accounts.get(account) else {
        return false;
    };
    if def
        .min_level
        .is_some_and(|min_level| account_info.level < min_level)
    {
        return false;
    }
    if let Some(regions) = &def.regions {
        if !regions.contains(&account_info.region) {
            return false;
        }
    }
    if def.starts_at_ms.is_some_and(|start| now_ms < start) {
        return false;
    }
    if def.ends_at_ms.is_some_and(|end| now_ms >= end) {
        return false;
    }
    def.requires
        .iter()
        .all(|prerequisite| state.tasks.is_completed(account, prerequisite))
}

fn task_objective_progress(
    state: &GameState,
    account: &AccountId,
    counters: &BTreeMap<String, u64>,
    objective: &TaskObjective,
) -> (u64, u64, bool) {
    let target = objective.target();
    let raw = match objective {
        TaskObjective::Metric { metric, .. } => counters.get(metric).copied().unwrap_or(0),
        TaskObjective::Item { item, .. } => state
            .inventories
            .get(account)
            .map(|inventory| inventory.count(item))
            .unwrap_or(0),
        TaskObjective::Skin { skin } => u64::from(state.cosmetics.owns(account, skin)),
        TaskObjective::Achievement { achievement } => {
            u64::from(state.achievements.is_completed(account, achievement))
        }
    };
    let progress = raw.min(target);
    (progress, target, progress >= target)
}

fn task_objectives_complete(
    state: &GameState,
    account: &AccountId,
    def: &TaskDef,
    counters: &BTreeMap<String, u64>,
) -> bool {
    def.objectives
        .iter()
        .all(|objective| task_objective_progress(state, account, counters, objective).2)
}

/// Mark active tasks whose objectives are now satisfied as completed. Returns
/// the tasks newly marked by this call.
fn refresh_account_tasks(state: &mut GameState, account: &AccountId, now_ms: i64) -> Vec<TaskId> {
    let mut newly_completed = Vec::new();
    for (id, progress) in state.tasks.account_progress(account) {
        if progress.completed_at_ms.is_some()
            || progress.claimed_at_ms.is_some()
            || progress.is_expired(now_ms)
        {
            continue;
        }
        let complete = state
            .tasks
            .get(&id)
            .is_some_and(|def| task_objectives_complete(state, account, def, &progress.counters));
        if complete {
            newly_completed.push(id);
        }
    }
    for id in &newly_completed {
        if let Some(progress) = state.tasks.progress_mut(account, id) {
            progress.completed_at_ms = Some(now_ms);
        }
    }
    newly_completed
}

/// Advance metric objectives on all active tasks, then refresh completion from
/// live item / skin / achievement state.
fn record_task_metric_in_state(
    state: &mut GameState,
    account: &AccountId,
    metric: &str,
    amount: u64,
    now_ms: i64,
) -> Vec<TaskId> {
    if amount == 0 {
        return Vec::new();
    }
    for (id, mut progress) in state.tasks.account_progress(account) {
        if progress.completed_at_ms.is_some()
            || progress.claimed_at_ms.is_some()
            || progress.is_expired(now_ms)
        {
            continue;
        }
        let relevant = state.tasks.get(&id).is_some_and(|def| {
            def.objectives
                .iter()
                .any(|objective| objective.metric_key() == Some(metric))
        });
        if !relevant {
            continue;
        }
        let counter = progress.counters.entry(metric.to_owned()).or_insert(0);
        *counter = counter.saturating_add(amount);
        state.tasks.set_progress(account, id, progress);
    }
    refresh_account_tasks(state, account, now_ms)
}
