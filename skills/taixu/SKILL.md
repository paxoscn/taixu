---
name: taixu
version: 1.0.0
description: Use when building or modifying a Rust game backend with the taixu library ("太虚", taixu, game backend, quests/tasks, inventory, wallet, shop, trading, rooms, chat, friends, leaderboards, achievements, announcements, NPC agents, snapshots). Covers the complete facade API, JSON wire contracts, atomicity, locks, persistence, configuration, errors, recipes and verification.
---

# Using taixu

`taixu` is a synchronous, in-memory, single-process game backend **library**.
The host owns networking, authentication, async runtimes and persistence.
Everything game-side goes through one `Game` facade, which holds a single
`parking_lot::RwLock<GameState>`.

## First moves

1. Read `Cargo.toml` and confirm how `taixu` is linked.
2. Use `taixu::prelude::*` for the facade and domain types.
3. For any non-trivial task, read the canonical reference page listed in
   [Reference map](#reference-map) before editing.
4. Follow the feature workflow at the end of this skill. Run
   `scripts/check.sh` before declaring work done.

```toml
[dependencies]
taixu = { path = "../taixu" } # or a released version
```

```rust
use taixu::prelude::*;

let game = Game::new();
```

`AccountId` is trusted input from the external auth layer. The library does
not validate tokens/sessions.

## Golden rules

- **One `Game` owns all state.** Do not keep parallel mutable copies; use
  `snapshot()` / `from_state()` or the facade.
- **Cross-module operations are atomic.** Purchases, trades, claims and NPC
  turns clone affected assets first and commit only after all validation.
- **Never do I/O while holding a lock.** The facade takes locks internally.
- **`read_state()` guards are not reentrant.** Do not call `wallet`,
  `task_views`, `purchase`, `record_event`, etc. while holding the guard.
  Prefer `game.with_state(|state| ...)`.
- **Time is injected.** Use `ManualClock` in tests; all timestamps are Unix ms.
- **Snapshots carry `version`.** `GameConfig`, the `Clock` and `NpcRuntime`
  agents are not part of `GameState`.
- **Do not bypass the facade** to mutate `GameState` unless you own the
  persistence/migration pipeline.
- **Behavior changes need tests**, especially JSON contract and atomicity
  tests.

## Minimal world

```rust
use std::collections::BTreeMap;
use taixu::prelude::*;

fn main() -> taixu::Result<()> {
    let game = Game::new();

    // Regions must exist before accounts are assigned.
    game.create_region("asia", "Asia", Some(5_000), Some("wss://asia.example".into()))?;

    // ensure_account is race-free and idempotent; useful for lazy first login.
    let alice = game.ensure_account("alice", "Alice", "asia")?;
    game.login(&alice.id)?;

    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
    game.register_resource(
        ResourceDef::new("energy", "Energy")
            .with_cap(100)
            .with_regen_per_hour(20),
    )?;
    game.add_item(&alice.id, &"potion".into(), 5)?;
    game.add_currency(&alice.id, &"gold".into(), 500)?;
    game.set_resource(&alice.id, &"energy".into(), 80)?;
    Ok(())
}
```

Currencies are dynamic string IDs; there is no built-in currency catalog.
Display names/icons are host configuration.

## Module map

| Area | Key types / methods |
| --- | --- |
| Accounts / regions | `Account`, `Region`, `create_region`, `ensure_account`, `login`, `set_account_region` |
| Rooms | `Room`, `create_room`, `join_room`, `leave_room`, `close_room`, `kick_from_room` |
| Friends | `FriendGraph` via `send_friend_request`, `accept_friend_request`, `block_account` |
| Chat | `ChatChannel`, `ChannelSummary`, `send_message`, `recent_messages`, `channels_for` |
| Items / inventory | `ItemDef`, `Inventory`, `register_item`, `add_item`, `remove_item` |
| Wallet / rewards | `Wallet`, `Reward`, `Cost`, `add_currency`, `spend_resource`, `grant_reward` |
| Shop | `Shop`, `ShopOffer`, `purchase`, `purchased_quantity` |
| Trading | `Trade`, `TradeSide`, `create_trade`, `set_trade_side`, `confirm_trade` |
| Cosmetics / avatar | `SkinDef`, `SkinCategory`, `Avatar`, `equip_skin`, `update_avatar` |
| Collections | `CollectionDef`, `Collectible`, `claim_collection_milestone` |
| Leaderboards | `Leaderboard`, `ScoreOrder`, `set_leaderboard_score`, `leaderboard_top` |
| Achievements | `AchievementDef`, `record_event`, `claim_achievement` |
| Tasks | `TaskDef`, `TaskObjective`, `TaskRepeat`, `accept_task`, `claim_task` |
| Announcements | `AnnouncementDraft`, `publish_announcement`, `active_announcements` |
| NPCs | `NpcDef`, `NpcAgent`, `NpcRuntime`, `apply_npc_turn` |
| Persistence | `GameState`, `GAME_STATE_VERSION`, `GameStore`, `MemoryStore` |

Generated API reference: `cargo doc --no-deps --open`.

## Recipes

### Accounts, presence and regions

```rust
let account = game.ensure_account("alice", "Alice", "asia")?;
game.heartbeat(&account.id)?;        // online = true, last_seen_ms = now
game.logout(&account.id)?;           // online = false

game.set_level(&account.id, 10)?;    // no auto-leveling from XP
game.add_xp(&account.id, 500)?;
game.set_account_region(&account.id, "eu")?; // leaves the current room
```

Region semantics:

- `Open`: normal.
- `Maintenance`: existing players keep rooms/chat/economy/tasks; new
  assignments and migrations in are blocked.
- `Closed`: additionally blocks player chat, purchases, trades, friend
  requests, task acceptance and NPC turns. Read-only queries, claims,
  abandoning, profile/cosmetic edits and cancellation remain available.

### Inventory, wallet and rewards

```rust
game.register_item(ItemDef::new("sword", "Sword").unique())?;
game.add_item(&account.id, &"sword".into(), 1)?;   // all-or-nothing
game.remove_item(&account.id, &"sword".into(), 1)?;

let inventory = game.inventory(&account.id)?;     // read lock
let wallet = game.wallet(&account.id)?;           // W*: lazy regen settle
let gold = game.currency_balance(&account.id, &"gold".into())?;

game.grant_reward(
    &account.id,
    &Reward::new()
        .currency("gold", 100)
        .resource("energy", 10)
        .item("potion", 2)
        .skin("gold_frame")
        .xp(50),
)?;
```

- Item adds/removes are atomic; `InventoryFull`/`InsufficientItems` leave the
  inventory untouched.
- Resource regeneration is lazy and persists a fractional remainder. Reading
  the same timestamp twice never grants twice.
- Skin rewards are unique: scaling an offer by 5 still grants at most one skin.
- `add_item` and `grant_skin` refresh task completion immediately.

### Shop purchase

```rust
let mut shop = Shop::new("general", "General Store");
shop.add_offer(
    ShopOffer::new(
        "potion_pack",
        "Potion pack",
        Cost::new().currency("gold", 100),
        Reward::new().item("potion", 4),
    )
    .with_per_account_limit(5)
    .with_stock(1_000),
)?;
game.register_shop(shop)?;

let receipt = game.purchase(&account.id, &"general".into(), &"potion_pack".into(), 2)?;
```

`purchase` validates offer window/region/stock/limit, debits, delivers, then
records the purchase under one lock. Any failure leaves zero side effects.

### Player trade

```rust
let bob = game.create_account("bob", "Bob", "asia")?;
let trade = game.create_trade(&account.id, &bob.id)?;

game.set_trade_side(
    &account.id,
    &trade.id,
    TradeSide::new().with_items(BTreeMap::from([("potion".into(), 2)])),
)?;
game.set_trade_side(
    &bob.id,
    &trade.id,
    TradeSide::new().with_currencies(BTreeMap::from([("gold".into(), 50)])),
)?;

game.confirm_trade(&account.id, &trade.id)?;
let completed = game.confirm_trade(&bob.id, &trade.id)?; // settles atomically
```

- Setting a side resets both confirmations.
- Settlement removes both sides' outgoing assets first, then inserts incoming
  assets. Missing assets/full inventory → trade stays `Open`, confirmations
  reset, error returned.
- `trades_for(account)` is newest-first.
- Hosts must periodically call `expire_trades()`; mutations also check expiry.
- Chat/trade/friend/task/NPC activity is blocked while either party is in a
  `Closed` region (creation checks both parties).

### Rooms, friends and chat

```rust
let room = game.create_room(&account.id, "Hangout", &"asia".into(), 8, BTreeMap::new())?;
game.join_room(&bob.id, &room.id)?;
game.send_message(&account.id, ChatChannel::Room(room.id.clone()), "hello")?;

game.send_friend_request(&account.id, &bob.id, Some("hi"))?;
game.accept_friend_request(&bob.id, &account.id)?;

let dm = ChatChannel::private(account.id.clone(), bob.id.clone());
game.send_message(&account.id, dm, "wanna trade?")?;

for summary in game.channels_for(&account.id)? {
    println!("{:?} last={:?}", summary.channel, summary.last_message_at_ms);
}
```

- One room per account. Owner leaving transfers ownership to the lowest
  `AccountId`; an empty room is deleted.
- Blocking removes friendship and pending requests; either direction blocks
  DMs and friend requests.
- `channels_for` returns only channels the account can currently read and that
  have retained history, newest activity first.

### Tasks (quests)

```rust
game.register_item(ItemDef::new("potion", "Potion"))?;
game.register_skin(SkinDef::new("frame", "Frame", SkinCategory::Frame))?;
game.register_achievement(AchievementDef::new(
    "first_blood", "First blood", "kills", 2, Reward::new(),
))?;

game.register_task(
    TaskDef::new("collector", "Collector")
        .with_objective(TaskObjective::metric("kills", 2))
        .with_objective(TaskObjective::item("potion", 1))
        .with_objective(TaskObjective::skin("frame"))
        .with_objective(TaskObjective::achievement("first_blood"))
        .with_reward(Reward::new().currency("gold", 100))
        .with_repeat(TaskRepeat::daily())
        .with_time_limit_ms(6 * 3_600_000),
)?;

game.accept_task(&account.id, &"collector".into())?;
game.add_item(&account.id, &"potion".into(), 1)?;
game.grant_skin(&account.id, &"frame".into())?;
let outcome = game.record_event(&account.id, "kills", 2)?;
// outcome.achievements + outcome.tasks

let view = game.task_view(&account.id, &"collector".into())?;
if view.status == TaskStatus::Completed {
    game.claim_task(&account.id, &"collector".into())?;
}
```

Task rules:

- Objectives: metric, item, skin, achievement. Metric progresses through
  `record_event`; the others are evaluated from live state.
- Lifecycle: `Available* → Active → Completed → Claimed`; also `Locked`,
  `Expired`.
- `TaskView` exposes `requires`, `regions`, `starts_at_ms`, `ends_at_ms`,
  `min_level` so clients can explain `Locked`.
- Limited tasks completed before the deadline stay claimable after expiry.
- Repeat cooldown starts at **claim**, not acceptance. After cooldown, the view
  presents a fresh cycle (zero metric progress/timestamps); if requirements are
  not met, status is `Locked`, not `Claimed`.
- `accept_task` resets counters. `abandon_task` removes progress; claimed tasks
  cannot be abandoned.
- One event advances achievements and every active task:
  `record_event(account, metric, amount)`.
- Use `available_tasks` / `active_tasks` / `completed_tasks` / `task_views`.

### Achievements, collections, leaderboards, announcements

```rust
game.record_event(&account.id, "kills", 1)?;
game.claim_achievement(&account.id, &"first_blood".into())?;

game.register_collection(
    CollectionDef::new("album", "Album")
        .with_entries([Collectible::Item("potion".into())])
        .with_milestones([CollectionMilestone::new(1, Reward::new().currency("gold", 5))]),
)?;
game.collection_progress(&account.id, &"album".into())?;
game.claim_collection_milestone(&account.id, &"album".into(), 0)?;

game.register_leaderboard(
    Leaderboard::new("weekly", "Weekly", ScoreOrder::HigherIsBetter)
        .with_max_entries(100),
)?;
game.set_leaderboard_score(&"weekly".into(), &account.id, 100)?;
game.leaderboard_top(&"weekly".into(), 10)?;

game.publish_announcement(
    AnnouncementDraft::new("Event", "Double XP!")
        .with_kind(AnnouncementKind::Event)
        .with_priority(10),
)?;
let unread = game.unread_announcements(&account.id)?;
```

- Achievements unlock by metric counters; rewards require explicit claim.
- Collections aggregate live item/skin/achievement state; milestones are
  claimed once.
- Leaderboard ranks are tie-aware; lower-is-better is supported; reset
  increments the season.
- Announcements are time-windowed, region-targeted, priority-sorted, and
  per-account read state is tracked.

### NPC agent

```rust
use std::sync::Arc;
use async_trait::async_trait;
use taixu::npc::NpcError;
use taixu::prelude::*;

struct GiftAgent;

#[async_trait]
impl NpcAgent for GiftAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("For you.".into()),
            actions: vec![NpcAction::GiveReward {
                account: ctx.speaker.clone(),
                reward: Reward::new().currency("gold", 5),
            }],
        })
    }
}

game.register_npc(NpcDef::new("guide", "Guide", "helpful").in_region("asia"))?;
let mut runtime = NpcRuntime::new();
runtime.register("guide", Arc::new(GiftAgent));

let turn = runtime
    .converse(&game, &"guide".into(), &account.id, "hello")
    .await?;
```

- `NpcRuntime` is not serializable; re-register agents on boot.
- `converse` builds context, awaits the agent, then calls
  `Game::apply_npc_turn`, which validates everything before committing.
- Actions: `Say`, `GiveReward`, `SetScore`. Targets are limited to the speaker
  and nearby players.
- A failing action cancels the whole turn: no reward, no score, no chat.
- `NpcContext::system_prompt()` includes persona, channel, nearby players,
  recent messages and the expected JSON shape.
- No action-count or reward-total cap exists; enforce limits in the host/agent
  wrapper.

### Persistence

```rust
use std::sync::Arc;
use taixu::prelude::*;

let state = game.snapshot();
let json = serde_json::to_string(&state)?;

let decoded: GameState = serde_json::from_str(&json)?;
let restored = Game::from_state(
    decoded,
    game.config().clone(),
    Arc::new(SystemClock),
);

// Or through a store:
let store = taixu::store::MemoryStore::new();
game.save_to(&store)?;
let loaded = Game::load_from(&store, game.config().clone())?;
```

Note: when restoring in tests, pass the same `ManualClock` you use for the
original game instead of `SystemClock`.

- `GameState.version` / `GAME_STATE_VERSION` is the schema version. Missing
  versions default to current; `0` is normalized by `from_state`.
- Migrate snapshots yourself before `from_state` when structures change.
- `GameConfig` and the clock are not persisted. Chat history capacity is fixed
  when the `ChatService` is constructed.
- `GameStore` is synchronous; wrap async DB clients or snapshot outside the
  request path.
- No built-in retention/account deletion. Hosts must define policies.

## JSON contracts (quick reference)

- `ChatChannel`: `"global"`, `"region:<id>"`, `"room:<id>"`,
  `"dm:<byte_len_of_a>:<a><b>"`. DM participants are sorted; length is UTF-8
  bytes. Example: `"dm:5:alicebob"`.
- `ChatAuthor`: `{"kind":"account","account":...}` / `{"kind":"npc","npc":...}`
  / `{"kind":"system"}`.
- `TaskObjective`: tag `type` = `metric|item|skin|achievement`.
- `TaskRepeat`: tag `kind` = `once|cooldown`; cooldown carries `duration_ms`.
- `TaskStatus`: snake_case string.
- `SkinCategory`: `character|weapon|avatar|frame|badge|background|emote|other:<name>`.
- `Collectible`: adjacent tags `{"kind":"item","id":...}`.
- `NpcAction`: tag `type` = `say|give_reward|set_score`.
- `NpcDecision`: `{"reply": string|null, "actions": [...]}`.
- `Reward`: `currencies/resources/items/skins/xp`; `Cost`: currencies/resources.
- `Option` fields serialize as `null`, not omitted.
- Errors are not serde types; map `GameError` to protocol error codes.

Full details and examples: `references/json-contracts.md`.
Wire shapes are guarded by `tests/json_contracts.rs`.

## Locks, atomicity and errors

Write-locking operations include `wallet`, `resource_balance`, `record_event`,
`task_views`, `task_view`, `purchase`, `set_account_region`, `add_item`,
`grant_reward` and most mutations. Resource reads and task views also take the
write lock because of lazy regeneration/completion refresh.

```rust
// Safe short read:
let count = game.with_state(|state| state.accounts.len());

// Deadlock risk:
let state = game.read_state();
// game.task_views(&account.id); // don't do this while `state` is alive
```

Atomic operations:

- `purchase`: charge + delivery + stock/limit ledger, all-or-nothing.
- `confirm_trade`: swap + commit; failure resets confirmations and keeps `Open`.
- `claim_task` / `claim_achievement` / `claim_collection_milestone`: reward
  first, then mark claimed.
- `grant_reward`, `apply_npc_turn`: cloned-asset commit.
- `add_item` / `remove_item`, currency/resource spend: all-or-nothing.

Common error families: `AccountNotFound`, `RegionNotOpen`, `RoomFull`,
`InsufficientFunds`/`Resources`/`Items`, `InventoryFull`, `TaskLocked` /
`TaskOnCooldown`, `TradeExpired`, `ChannelAccessDenied`, `Blocked`,
`LeaderboardNotFound`, `NpcDisabled`, `Persistence`.

Full matrix: `references/errors.md`.

## Reference map

Bundled with this skill (copied from the taixu repository `docs/`):

| Need | File |
| --- | --- |
| Method/API index, lock table | `references/api.md` |
| JSON shapes and examples | `references/json-contracts.md` |
| Task/trade/region/resource/NPC state machines | `references/state-machines.md` |
| Snapshot version/migration/GameStore | `references/persistence.md` |
| Host responsibilities and operations | `references/hosting.md` |
| NPC prompt/decision/action contract | `references/npc.md` |
| All `GameConfig` fields | `references/config.md` |
| Error variants and atomicity | `references/errors.md` |
| Common flows | `references/cookbook.md` |
| Behavior changes | `references/CHANGELOG.md` |
| Runnable examples | <https://github.com/paxoscn/taixu/tree/main/examples> |
| Acceptance tests | <https://github.com/paxoscn/taixu/tree/main/tests> |

Examples in the taixu repository: `basic`, `tasks`, `trade_rollback`,
`npc_actions`, `persistence`, `region_matrix`, `channels`.

## Feature workflow

When changing the taixu library itself:

1. Read the relevant module and bundled reference; identify the state machine
   and all side effects.
2. Implement inside `Game` under one write lock. For multi-step mutations,
   validate first and mutate clones before committing.
3. Keep `GameState` serde-compatible; bump `GAME_STATE_VERSION` for breaking
   snapshot changes and document migration.
4. Add a regression test (atomicity, boundary, concurrency, or wire shape).
5. Update JSON contract tests if a client-visible type changed.
6. Update the relevant repository `docs/*.md`, `CHANGELOG.md`, and README if
   user-facing, then run `scripts/sync-docs.sh` to refresh bundled references.
7. Run the full check:

```bash
skills/taixu/scripts/check.sh
```

The script checks that bundled references are in sync, then runs fmt, clippy,
all tests, README doctests, strict rustdoc and all examples.

When using taixu from a consumer application, run your own `cargo test`,
`cargo clippy` and any integration tests; the maintainer check script lives here
but is not required in the consumer project.

## Pitfalls checklist

- [ ] `ensure_account`, not read-then-create.
- [ ] Do not hold `read_state()` across a write-locking call.
- [ ] `wallet()` is a write-lock call (lazy regen).
- [ ] `TaskView` fresh cycle after cooldown; `Locked` when ineligible.
- [ ] `update_avatar` is authoritative for frame/badge and clears on `None`.
- [ ] `Closed` regions block new activity; `Maintenance` does not.
- [ ] Trade confirmations reset when a side changes or settlement fails.
- [ ] `record_event` advances both achievements and tasks.
- [ ] Item/skin/achievement task objectives are live state, not counters.
- [ ] Repeat cooldowns start at claim time.
- [ ] Reward scaling does not duplicate skins.
- [ ] NPC actions and chat commit atomically; targets are restricted.
- [ ] DM channel strings use byte-length encoding and sorted participants.
- [ ] `GameConfig` and `Clock` are not in snapshots.
- [ ] No account deletion/retention is built in.
- [ ] `Result` is not exported by the prelude; use `GameResult` or
  `std::result::Result`.
- [ ] Tests/README code blocks are part of `cargo test --doc`.
