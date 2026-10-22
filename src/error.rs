//! Error type returned by every fallible backend operation.

use crate::chat::ChatChannel;
use crate::cosmetics::SkinCategory;
use crate::ids::{
    AccountId, AchievementId, AnnouncementId, CollectionId, CurrencyId, ItemId, LeaderboardId,
    MessageId, NpcId, OfferId, RegionId, ResourceId, RoomId, ShopId, SkinId, TaskId, TradeId,
};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, GameError>;

#[derive(Debug, Error)]
pub enum GameError {
    // ---- accounts & regions -------------------------------------------------
    #[error("account not found: {0}")]
    AccountNotFound(AccountId),
    #[error("account already exists: {0}")]
    AccountAlreadyExists(AccountId),
    #[error("invalid display name: {0}")]
    InvalidName(String),
    #[error("region not found: {0}")]
    RegionNotFound(RegionId),
    #[error("region already exists: {0}")]
    RegionAlreadyExists(RegionId),
    #[error("region is not open: {0}")]
    RegionNotOpen(RegionId),
    #[error("region is full: {0}")]
    RegionFull(RegionId),

    // ---- rooms --------------------------------------------------------------
    #[error("room not found: {0}")]
    RoomNotFound(RoomId),
    #[error("room is full: {0}")]
    RoomFull(RoomId),
    #[error("account {account} is already in room {room}")]
    AlreadyInRoom { account: AccountId, room: RoomId },
    #[error("account {0} is not in a room")]
    NotInRoom(AccountId),
    #[error("account {0} is not a member of room {1}")]
    NotRoomMember(AccountId, RoomId),
    #[error("only the room owner may perform this action")]
    NotRoomOwner,
    #[error("room capacity must be between 1 and {max} (requested {requested})")]
    InvalidRoomCapacity { requested: usize, max: usize },

    // ---- generic ------------------------------------------------------------
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid quantity: {0}")]
    InvalidQuantity(u64),
    #[error("amount overflow")]
    AmountOverflow,
    #[error("entity already exists: {0}")]
    AlreadyExists(String),

    // ---- wallet -------------------------------------------------------------
    #[error("unknown resource: {0}")]
    UnknownResource(ResourceId),
    #[error("resource already exists: {0}")]
    ResourceAlreadyExists(ResourceId),
    #[error("insufficient funds: {currency} needs {needed}, available {available}")]
    InsufficientFunds {
        currency: CurrencyId,
        needed: u64,
        available: u64,
    },
    #[error("insufficient resource: {resource} needs {needed}, available {available}")]
    InsufficientResources {
        resource: ResourceId,
        needed: u64,
        available: u64,
    },

    // ---- items --------------------------------------------------------------
    #[error("unknown item: {0}")]
    UnknownItem(ItemId),
    #[error("item already exists: {0}")]
    ItemAlreadyExists(ItemId),
    #[error("insufficient items: {item} needs {needed}, available {available}")]
    InsufficientItems {
        item: ItemId,
        needed: u64,
        available: u64,
    },
    #[error("inventory is full")]
    InventoryFull,

    // ---- friends ------------------------------------------------------------
    #[error("cannot add yourself as a friend")]
    CannotFriendSelf,
    #[error("already friends with {0}")]
    AlreadyFriends(AccountId),
    #[error("a friend request is already pending")]
    FriendRequestPending,
    #[error("friend request not found")]
    FriendRequestNotFound,
    #[error("interaction blocked")]
    Blocked,
    #[error("account is already blocked")]
    AlreadyBlocked,
    #[error("account is not blocked")]
    NotBlocked,

    // ---- trades -------------------------------------------------------------
    #[error("trade not found: {0}")]
    TradeNotFound(TradeId),
    #[error("trade is not open")]
    TradeNotOpen,
    #[error("account {0} is not a participant of this trade")]
    NotTradeParticipant(AccountId),
    #[error("cannot trade with yourself")]
    CannotTradeWithSelf,
    #[error("both sides of a trade are empty")]
    EmptyTrade,
    #[error("trade has expired")]
    TradeExpired,
    #[error("offered assets are no longer available")]
    TradeAssetsUnavailable,

    // ---- shop ---------------------------------------------------------------
    #[error("shop not found: {0}")]
    ShopNotFound(ShopId),
    #[error("shop already exists: {0}")]
    ShopAlreadyExists(ShopId),
    #[error("offer not found: {0}")]
    OfferNotFound(OfferId),
    #[error("offer already exists: {0}")]
    OfferAlreadyExists(OfferId),
    #[error("offer is not currently available")]
    OfferUnavailable,
    #[error("offer is out of stock")]
    OutOfStock,
    #[error("per-account purchase limit reached")]
    PurchaseLimitReached,

    // ---- cosmetics ----------------------------------------------------------
    #[error("skin not found: {0}")]
    SkinNotFound(SkinId),
    #[error("skin already exists: {0}")]
    SkinAlreadyExists(SkinId),
    #[error("skin not owned: {0}")]
    SkinNotOwned(SkinId),
    #[error("skin {skin} is {actual:?}, expected {expected:?}")]
    SkinCategoryMismatch {
        skin: SkinId,
        expected: SkinCategory,
        actual: SkinCategory,
    },

    // ---- collections --------------------------------------------------------
    #[error("collection not found: {0}")]
    CollectionNotFound(CollectionId),
    #[error("collection already exists: {0}")]
    CollectionAlreadyExists(CollectionId),
    #[error("milestone index out of range: {0}")]
    MilestoneNotFound(usize),
    #[error("milestone has not been unlocked yet")]
    MilestoneLocked,
    #[error("milestone already claimed")]
    AlreadyClaimed,

    // ---- leaderboards -------------------------------------------------------
    #[error("leaderboard not found: {0}")]
    LeaderboardNotFound(LeaderboardId),
    #[error("leaderboard already exists: {0}")]
    LeaderboardAlreadyExists(LeaderboardId),

    // ---- achievements -------------------------------------------------------
    #[error("achievement not found: {0}")]
    AchievementNotFound(AchievementId),
    #[error("achievement already exists: {0}")]
    AchievementAlreadyExists(AchievementId),
    #[error("achievement is not completed yet")]
    AchievementNotCompleted,
    #[error("achievement reward already claimed")]
    AchievementAlreadyClaimed,

    // ---- tasks --------------------------------------------------------------
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
    #[error("task already exists: {0}")]
    TaskAlreadyExists(TaskId),
    #[error("task requirements are not met: {0}")]
    TaskLocked(TaskId),
    #[error("task is already accepted: {0}")]
    TaskAlreadyAccepted(TaskId),
    #[error("task is already completed and awaiting reward: {0}")]
    TaskAlreadyCompleted(TaskId),
    #[error("task has not been accepted: {0}")]
    TaskNotAccepted(TaskId),
    #[error("task objectives are not completed: {0}")]
    TaskNotCompleted(TaskId),
    #[error("task has already been claimed: {0}")]
    TaskAlreadyClaimed(TaskId),
    #[error("task is on cooldown until {available_at_ms}: {task}")]
    TaskOnCooldown { task: TaskId, available_at_ms: i64 },

    // ---- announcements ------------------------------------------------------
    #[error("announcement not found: {0}")]
    AnnouncementNotFound(AnnouncementId),
    #[error("invalid announcement time window")]
    InvalidAnnouncementWindow,

    // ---- chat ---------------------------------------------------------------
    #[error("chat channel is not accessible: {0:?}")]
    ChannelAccessDenied(ChatChannel),
    #[error("message not found: {0}")]
    MessageNotFound(MessageId),
    #[error("content must not be empty")]
    EmptyContent,
    #[error("content is too long (max {max} characters)")]
    ContentTooLong { max: usize },

    // ---- NPC ----------------------------------------------------------------
    #[error("npc not found: {0}")]
    NpcNotFound(NpcId),
    #[error("npc already exists: {0}")]
    NpcAlreadyExists(NpcId),
    #[error("npc is disabled: {0}")]
    NpcDisabled(NpcId),

    // ---- persistence --------------------------------------------------------
    #[error("persistence error: {0}")]
    Persistence(String),
}
