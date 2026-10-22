use std::collections::BTreeMap;
use std::sync::Arc;

use futures::executor::block_on;
use taixu::npc::NpcError;
use taixu::prelude::*;

fn game_with_clock() -> (Game, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("eu", "Europe", Some(10), Some("wss://eu".into()))
        .unwrap();
    game.create_region("us", "United States", None, None)
        .unwrap();
    (game, clock)
}

fn account(game: &Game, id: &str) -> Account {
    game.create_account(id, id, "eu").unwrap()
}

#[test]
fn accounts_regions_and_rooms() {
    let (game, _) = game_with_clock();
    let a = account(&game, "a");
    let b = account(&game, "b");
    let c = account(&game, "c");
    assert_eq!(game.region_player_count(&"eu".into()).unwrap(), 3);
    assert_eq!(game.account_count(), 3);

    game.create_region("tiny", "Tiny", Some(1), None).unwrap();
    game.create_account("x", "Xavier", "tiny").unwrap();
    assert!(matches!(
        game.create_account("y", "Yara", "tiny"),
        Err(GameError::RegionFull(_))
    ));

    let room = game
        .create_room(&a.id, "Lobby", &"eu".into(), 2, BTreeMap::new())
        .unwrap();
    game.join_room(&b.id, &room.id).unwrap();
    assert!(matches!(
        game.join_room(&c.id, &room.id),
        Err(GameError::RoomFull(_))
    ));

    // Owner transfer on leave.
    let room_id = game.leave_room(&a.id).unwrap().unwrap();
    let room_after = game.room(&room_id).unwrap();
    assert_eq!(room_after.owner, b.id);
    assert!(room_after.contains(&b.id));

    game.close_room(&b.id, &room_id).unwrap();
    assert!(game.room(&room_id).is_err());

    // Moving regions leaves (and, when sole member, deletes) the room.
    let solo = game
        .create_room(&c.id, "Solo", &"eu".into(), 4, BTreeMap::new())
        .unwrap();
    let moved = game.set_account_region(&c.id, "us").unwrap();
    assert_eq!(moved.region, "us".into());
    assert!(game.room_id_of(&c.id).unwrap().is_none());
    assert!(game.room(&solo.id).is_err());

    game.set_region_status(&"us".into(), RegionStatus::Maintenance)
        .unwrap();
    assert!(game
        .set_account_region(&a.id, "us")
        .unwrap_err()
        .to_string()
        .contains("not open"));
}

#[test]
fn shop_purchases_are_atomic_and_limited() {
    let clock = Arc::new(ManualClock::new(500_000));
    let config = GameConfig {
        default_inventory_capacity: 2,
        ..GameConfig::default()
    };
    let game = Game::with_config_and_clock(config, clock);
    game.create_region("eu", "Europe", None, None).unwrap();

    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(5))
        .unwrap();
    game.register_skin(SkinDef::new("emote", "Emote", SkinCategory::Emote))
        .unwrap();

    let mut shop = Shop::new("store", "Store");
    shop.add_offer(
        ShopOffer::new(
            "bundle",
            "Potion bundle",
            Cost::new().currency("gold", 100),
            Reward::new().item("potion", 4).skin("emote").xp(5),
        )
        .with_per_account_limit(2),
    )
    .unwrap();
    game.register_shop(shop).unwrap();

    let a = account(&game, "a");
    game.add_currency(&a.id, &"gold".into(), 150).unwrap();

    let error = game
        .purchase(&a.id, &"store".into(), &"bundle".into(), 2)
        .unwrap_err();
    assert!(matches!(error, GameError::InsufficientFunds { .. }));
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 150);
    assert_eq!(game.item_count(&a.id, &"potion".into()).unwrap(), 0);

    let receipt = game
        .purchase(&a.id, &"store".into(), &"bundle".into(), 1)
        .unwrap();
    assert_eq!(receipt.quantity, 1);
    assert_eq!(receipt.received.items[0].quantity, 4);
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 50);
    assert_eq!(game.item_count(&a.id, &"potion".into()).unwrap(), 4);
    assert!(game.owns_skin(&a.id, &"emote".into()).unwrap());
    assert_eq!(game.account(&a.id).unwrap().xp, 5);
    assert_eq!(
        game.purchased_quantity(&a.id, &"store".into(), &"bundle".into()),
        1
    );

    game.add_currency(&a.id, &"gold".into(), 100).unwrap();
    game.purchase(&a.id, &"store".into(), &"bundle".into(), 1)
        .unwrap();
    assert!(matches!(
        game.purchase(&a.id, &"store".into(), &"bundle".into(), 1),
        Err(GameError::PurchaseLimitReached)
    ));

    // Full inventory must reject the purchase without charging the player.
    let b = account(&game, "b");
    game.set_inventory_capacity(&b.id, 1).unwrap();
    game.add_currency(&b.id, &"gold".into(), 100).unwrap();
    game.add_offer(
        &"store".into(),
        ShopOffer::new(
            "overflow",
            "Overflow",
            Cost::new().currency("gold", 100),
            Reward::new().item("potion", 6), // needs two slots (max stack 5)
        ),
    )
    .unwrap();
    assert!(matches!(
        game.purchase(&b.id, &"store".into(), &"overflow".into(), 1),
        Err(GameError::InventoryFull)
    ));
    assert_eq!(game.currency_balance(&b.id, &"gold".into()).unwrap(), 100);
    assert_eq!(
        game.purchased_quantity(&b.id, &"store".into(), &"overflow".into()),
        0
    );

    // Out-of-stock offers fail before charging.
    game.add_offer(
        &"store".into(),
        ShopOffer::new(
            "rare",
            "Rare",
            Cost::new().currency("gold", 10),
            Reward::new().item("potion", 1),
        )
        .with_stock(1),
    )
    .unwrap();
    assert!(matches!(
        game.purchase(&b.id, &"store".into(), &"rare".into(), 2),
        Err(GameError::OutOfStock)
    ));
}

#[test]
fn trades_swap_assets_atomically() {
    let (game, clock) = game_with_clock();
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(10))
        .unwrap();
    game.register_item(ItemDef::new("sword", "Sword").unique())
        .unwrap();
    let a = account(&game, "a");
    let b = account(&game, "b");
    game.add_item(&a.id, &"potion".into(), 10).unwrap();
    game.add_currency(&a.id, &"gold".into(), 100).unwrap();
    game.add_item(&b.id, &"sword".into(), 1).unwrap();
    game.add_currency(&b.id, &"gold".into(), 20).unwrap();

    let trade = game.create_trade(&a.id, &b.id).unwrap();
    game.set_trade_side(
        &a.id,
        &trade.id,
        TradeSide::new()
            .with_items(BTreeMap::from([("potion".into(), 4)]))
            .with_currencies(BTreeMap::from([("gold".into(), 30)])),
    )
    .unwrap();
    game.set_trade_side(
        &b.id,
        &trade.id,
        TradeSide::new()
            .with_items(BTreeMap::from([("sword".into(), 1)]))
            .with_currencies(BTreeMap::from([("gold".into(), 10)])),
    )
    .unwrap();

    let pending = game.confirm_trade(&a.id, &trade.id).unwrap();
    assert_eq!(pending.state, TradeState::Open);
    let completed = game.confirm_trade(&b.id, &trade.id).unwrap();
    assert_eq!(completed.state, TradeState::Completed);

    assert_eq!(game.item_count(&a.id, &"potion".into()).unwrap(), 6);
    assert_eq!(game.item_count(&a.id, &"sword".into()).unwrap(), 1);
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 80);
    assert_eq!(game.item_count(&b.id, &"potion".into()).unwrap(), 4);
    assert_eq!(game.item_count(&b.id, &"sword".into()).unwrap(), 0);
    assert_eq!(game.currency_balance(&b.id, &"gold".into()).unwrap(), 40);

    // Assets disappearing between commitment and execution abort the trade.
    game.add_item(&b.id, &"sword".into(), 1).unwrap();
    let trade2 = game.create_trade(&a.id, &b.id).unwrap();
    game.set_trade_side(
        &b.id,
        &trade2.id,
        TradeSide::new().with_items(BTreeMap::from([("sword".into(), 1)])),
    )
    .unwrap();
    // b's sword disappears between setting the side and settlement.
    game.remove_item(&b.id, &"sword".into(), 1).unwrap();
    assert!(matches!(
        game.set_trade_side(
            &b.id,
            &trade2.id,
            TradeSide::new().with_items(BTreeMap::from([("sword".into(), 1)])),
        ),
        Err(GameError::InsufficientItems { .. })
    ));

    game.confirm_trade(&a.id, &trade2.id).unwrap();
    let err = game.confirm_trade(&b.id, &trade2.id).unwrap_err();
    assert!(matches!(err, GameError::InsufficientItems { .. }));
    let after = game.trade(&trade2.id).unwrap();
    assert_eq!(after.state, TradeState::Open);
    assert!(!after.initiator_confirmed && !after.partner_confirmed);
    assert_eq!(game.item_count(&a.id, &"potion".into()).unwrap(), 6);

    // Expiry is lazy but enforced on the next mutation.
    let trade3 = game.create_trade(&a.id, &b.id).unwrap();
    clock.advance(86_400 * 1_000 + 1);
    assert!(matches!(
        game.confirm_trade(&a.id, &trade3.id),
        Err(GameError::TradeExpired)
    ));
    assert_eq!(game.trade(&trade3.id).unwrap().state, TradeState::Expired);
}

#[test]
fn friends_chat_and_rooms_enforce_access() {
    let (game, _) = game_with_clock();
    let a = account(&game, "a");
    let b = account(&game, "b");
    let c = account(&game, "c");

    game.send_friend_request(&a.id, &b.id, Some("hi")).unwrap();
    assert_eq!(game.friend_requests(&b.id).unwrap().len(), 1);
    game.accept_friend_request(&b.id, &a.id).unwrap();
    assert!(game.are_friends(&a.id, &b.id));
    assert_eq!(game.friends(&a.id).unwrap(), vec![b.id.clone()]);

    let dm = ChatChannel::private(b.id.clone(), a.id.clone());
    game.send_message(&a.id, dm.clone(), "hello b").unwrap();
    let messages = game.recent_messages(&b.id, dm.clone(), 10).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hello b");

    game.block_account(&b.id, &a.id).unwrap();
    assert!(!game.are_friends(&a.id, &b.id));
    assert!(matches!(
        game.send_message(&a.id, dm, "blocked?"),
        Err(GameError::Blocked)
    ));

    let room = game
        .create_room(&a.id, "Room", &"eu".into(), 4, BTreeMap::new())
        .unwrap();
    game.join_room(&b.id, &room.id).unwrap();
    game.send_message(&a.id, ChatChannel::Room(room.id.clone()), "in room")
        .unwrap();
    assert_eq!(
        game.recent_messages(&b.id, ChatChannel::Room(room.id.clone()), 10)
            .unwrap()
            .len(),
        1
    );
    assert!(matches!(
        game.send_message(&c.id, ChatChannel::Room(room.id.clone()), "nope"),
        Err(GameError::ChannelAccessDenied(_))
    ));
    assert!(matches!(
        game.recent_messages(&c.id, ChatChannel::Room(room.id), 10),
        Err(GameError::ChannelAccessDenied(_))
    ));

    let region_chat = ChatChannel::Region("eu".into());
    game.send_message(&a.id, region_chat.clone(), "eu chat")
        .unwrap();
    game.set_account_region(&c.id, "us").unwrap();
    assert!(matches!(
        game.send_message(&c.id, region_chat, "wrong region"),
        Err(GameError::ChannelAccessDenied(_))
    ));

    game.send_message(&a.id, ChatChannel::Global, "  global  ")
        .unwrap();
    let global = game.channel_history(&ChatChannel::Global, 10);
    assert_eq!(global.last().unwrap().body, "global");
}

#[test]
fn chat_history_is_bounded() {
    let clock = Arc::new(ManualClock::new(0));
    let config = GameConfig {
        chat_history_per_channel: 2,
        ..GameConfig::default()
    };
    let game = Game::with_config_and_clock(config, clock);
    game.create_region("eu", "Europe", None, None).unwrap();
    let a = account(&game, "a");
    for index in 0..3 {
        game.send_message(&a.id, ChatChannel::Global, &format!("m{index}"))
            .unwrap();
    }
    let messages = game
        .recent_messages(&a.id, ChatChannel::Global, 10)
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].body, "m1");
    assert_eq!(messages[1].body, "m2");
}

#[test]
fn skins_avatars_and_collections() {
    let (game, _) = game_with_clock();
    game.register_item(ItemDef::new("relic", "Relic").unique())
        .unwrap();
    game.register_skin(SkinDef::new("frame", "Gold frame", SkinCategory::Frame))
        .unwrap();
    game.register_skin(SkinDef::new("badge", "Star badge", SkinCategory::Badge))
        .unwrap();
    let a = account(&game, "a");

    game.grant_skin(&a.id, &"frame".into()).unwrap();
    let previous = game.equip_skin(&a.id, &"frame".into()).unwrap();
    assert!(previous.is_none());
    assert_eq!(
        game.equipped_skins(&a.id)
            .unwrap()
            .get(&SkinCategory::Frame)
            .unwrap(),
        &SkinId::from("frame")
    );

    let mut avatar = Avatar {
        image: Some("  https://cdn/avatar.png  ".into()),
        frame: Some("frame".into()),
        badge: Some("badge".into()),
        title: Some("  Explorer  ".into()),
    };
    assert!(matches!(
        game.update_avatar(&a.id, avatar.clone()),
        Err(GameError::SkinNotOwned(_))
    ));
    game.grant_skin(&a.id, &"badge".into()).unwrap();
    let saved = game.update_avatar(&a.id, avatar.clone()).unwrap();
    assert_eq!(saved.image.as_deref(), Some("https://cdn/avatar.png"));
    assert_eq!(saved.title.as_deref(), Some("Explorer"));

    avatar.frame = Some("badge".into()); // wrong category
    assert!(matches!(
        game.update_avatar(&a.id, avatar),
        Err(GameError::SkinCategoryMismatch { .. })
    ));

    game.register_achievement(AchievementDef::new(
        "first_step",
        "First step",
        "steps",
        1,
        Reward::new(),
    ))
    .unwrap();
    game.register_collection(
        CollectionDef::new("album", "Starter album")
            .with_entries([
                Collectible::Item("relic".into()),
                Collectible::Skin("badge".into()),
                Collectible::Achievement("first_step".into()),
            ])
            .with_milestones([
                CollectionMilestone::new(2, Reward::new().currency("gold", 50)),
                CollectionMilestone::new(3, Reward::new().currency("gold", 100)),
            ]),
    )
    .unwrap();

    game.add_item(&a.id, &"relic".into(), 1).unwrap();
    let progress = game.collection_progress(&a.id, &"album".into()).unwrap();
    assert_eq!((progress.collected, progress.total), (2, 3));
    assert!(progress.milestones[0].unlocked);
    assert!(!progress.milestones[1].unlocked);
    assert!(matches!(
        game.claim_collection_milestone(&a.id, &"album".into(), 1),
        Err(GameError::MilestoneLocked)
    ));
    game.claim_collection_milestone(&a.id, &"album".into(), 0)
        .unwrap();
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 50);
    assert!(matches!(
        game.claim_collection_milestone(&a.id, &"album".into(), 0),
        Err(GameError::AlreadyClaimed)
    ));

    game.record_metric(&a.id, "steps", 1).unwrap();
    let progress = game.collection_progress(&a.id, &"album".into()).unwrap();
    assert_eq!(progress.collected, 3);
    game.claim_collection_milestone(&a.id, &"album".into(), 1)
        .unwrap();
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 150);
}

#[test]
fn leaderboards_achievements_and_announcements() {
    let (game, clock) = game_with_clock();
    let a = account(&game, "a");
    let b = account(&game, "b");
    let c = account(&game, "c");
    let d = account(&game, "d");

    game.register_leaderboard(
        Leaderboard::new("kills", "Weekly kills", ScoreOrder::HigherIsBetter).with_max_entries(3),
    )
    .unwrap();
    for (account, score) in [(&a, 100), (&b, 300), (&c, 300), (&d, 50)] {
        game.set_leaderboard_score(&"kills".into(), &account.id, score)
            .unwrap();
    }
    assert_eq!(
        game.leaderboard_rank(&"kills".into(), &a.id).unwrap(),
        Some(3)
    );
    let top = game.leaderboard_top(&"kills".into(), 2).unwrap();
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].score, 300);
    assert_eq!(top[0].rank, 1);
    assert_eq!(top[1].rank, 1);
    assert_eq!(game.reset_leaderboard_season(&"kills".into()).unwrap(), 2);
    assert!(game
        .leaderboard_top(&"kills".into(), 10)
        .unwrap()
        .is_empty());

    game.register_achievement(AchievementDef::new(
        "veteran",
        "Veteran",
        "wins",
        2,
        Reward::new().currency("gold", 10),
    ))
    .unwrap();
    game.register_achievement(
        AchievementDef::new(
            "secret",
            "Secret",
            "wins",
            5,
            Reward::new().currency("gold", 1),
        )
        .hidden(),
    )
    .unwrap();
    assert!(game.record_metric(&a.id, "wins", 1).unwrap().is_empty());
    let completed = game.record_metric(&a.id, "wins", 1).unwrap();
    assert_eq!(completed, vec![AchievementId::from("veteran")]);
    assert!(!game
        .achievements_for(&a.id)
        .unwrap()
        .iter()
        .any(|view| view.id == "secret".into()));
    let view = game.achievement_view(&a.id, &"veteran".into()).unwrap();
    assert!(view.completed && !view.claimed);
    game.claim_achievement(&a.id, &"veteran".into()).unwrap();
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 10);
    assert!(matches!(
        game.claim_achievement(&a.id, &"veteran".into()),
        Err(GameError::AchievementAlreadyClaimed)
    ));

    let announcement = game
        .publish_announcement(
            AnnouncementDraft::new("Maintenance", "Servers restart at midnight")
                .with_kind(AnnouncementKind::Maintenance)
                .with_priority(10)
                .with_window(None, Some(clock.now_ms() + 1_000)),
        )
        .unwrap();
    assert_eq!(game.active_announcements(&a.id).unwrap().len(), 1);
    game.mark_announcement_read(&a.id, &announcement.id)
        .unwrap();
    assert!(game.unread_announcements(&a.id).unwrap().is_empty());
    clock.advance(2_000);
    assert!(game.active_announcements(&a.id).unwrap().is_empty());

    game.set_account_region(&d.id, "us").unwrap();
    let us_only = game
        .publish_announcement(
            AnnouncementDraft::new("US event", "Double XP").with_regions(["us".into()]),
        )
        .unwrap();
    let eu_ids: Vec<_> = game
        .active_announcements(&a.id)
        .unwrap()
        .into_iter()
        .map(|announcement| announcement.id)
        .collect();
    assert!(!eu_ids.contains(&us_only.id));
    assert!(game
        .active_announcements(&d.id)
        .unwrap()
        .iter()
        .any(|announcement| announcement.id == us_only.id));
}

#[test]
fn npc_runtime_applies_actions() {
    let (game, _) = game_with_clock();
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(10))
        .unwrap();
    let a = account(&game, "a");
    let outsider = account(&game, "outsider");
    let room = game
        .create_room(&a.id, "Tavern", &"eu".into(), 8, BTreeMap::new())
        .unwrap();
    game.register_npc(
        NpcDef::new("barkeep", "Barkeep", "A cheerful tavern keeper.")
            .in_room(room.id.clone())
            .with_greeting("Welcome!"),
    )
    .unwrap();

    let mut runtime = NpcRuntime::new();
    runtime.register(
        "barkeep",
        Arc::new(
            RuleBasedNpc::new("Ask me about the quests!")
                .with_rule("hello", "Welcome to the Tavern!")
                .with_rule("potion", "Here, take one."),
        ),
    );

    let turn = block_on(runtime.converse(&game, &"barkeep".into(), &a.id, "hello there")).unwrap();
    assert_eq!(turn.player_message.body, "hello there");
    assert_eq!(turn.npc_messages.len(), 1);
    assert_eq!(turn.npc_messages[0].body, "Welcome to the Tavern!");
    assert_eq!(turn.applied_actions, 0);

    assert!(matches!(
        block_on(runtime.converse(&game, &"barkeep".into(), &outsider.id, "hi")),
        Err(NpcError::Game(GameError::ChannelAccessDenied(_)))
    ));

    struct ShopkeeperAgent;
    #[async_trait::async_trait]
    impl NpcAgent for ShopkeeperAgent {
        async fn respond(
            &self,
            context: &NpcContext,
        ) -> std::result::Result<NpcDecision, NpcError> {
            Ok(NpcDecision {
                reply: Some("A gift for you.".into()),
                actions: vec![
                    NpcAction::Say {
                        text: "The shopkeeper smiles.".into(),
                    },
                    NpcAction::GiveReward {
                        account: context.speaker.clone(),
                        reward: Reward::new().currency("gold", 5).item("potion", 2),
                    },
                ],
            })
        }
    }
    runtime.register("barkeep", Arc::new(ShopkeeperAgent));
    let turn = block_on(runtime.converse(&game, &"barkeep".into(), &a.id, "can I have something?"))
        .unwrap();
    assert_eq!(turn.applied_actions, 1);
    assert_eq!(turn.npc_messages.len(), 2); // Say action + reply
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 5);
    assert_eq!(game.item_count(&a.id, &"potion".into()).unwrap(), 2);
}

#[test]
fn snapshot_round_trips_and_persists() {
    let (game, clock) = game_with_clock();
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(10))
        .unwrap();
    game.register_skin(SkinDef::new("frame", "Frame", SkinCategory::Frame))
        .unwrap();
    let a = account(&game, "a");
    let b = account(&game, "b");
    game.add_item(&a.id, &"potion".into(), 7).unwrap();
    game.add_currency(&a.id, &"gold".into(), 123).unwrap();
    game.grant_skin(&a.id, &"frame".into()).unwrap();
    game.equip_skin(&a.id, &"frame".into()).unwrap();
    game.join_room(
        &b.id,
        &game
            .create_room(&a.id, "Room", &"eu".into(), 4, BTreeMap::new())
            .unwrap()
            .id,
    )
    .unwrap();
    let dm = ChatChannel::private(a.id.clone(), b.id.clone());
    game.send_message(&a.id, dm.clone(), "saved message")
        .unwrap();
    game.register_leaderboard(Leaderboard::new(
        "score",
        "Score",
        ScoreOrder::HigherIsBetter,
    ))
    .unwrap();
    game.set_leaderboard_score(&"score".into(), &a.id, 42)
        .unwrap();
    game.publish_announcement(AnnouncementDraft::new("News", "Hello"))
        .unwrap();

    let json = serde_json::to_string(&game.snapshot()).unwrap();
    let restored_state: GameState = serde_json::from_str(&json).unwrap();
    let restored = Game::from_state(restored_state, game.config().clone(), clock.clone());

    assert_eq!(restored.account_count(), 2);
    assert_eq!(
        restored.currency_balance(&a.id, &"gold".into()).unwrap(),
        123
    );
    assert_eq!(restored.item_count(&a.id, &"potion".into()).unwrap(), 7);
    assert!(restored.owns_skin(&a.id, &"frame".into()).unwrap());
    assert_eq!(
        restored
            .equipped_skins(&a.id)
            .unwrap()
            .get(&SkinCategory::Frame)
            .unwrap(),
        &SkinId::from("frame")
    );
    assert_eq!(restored.recent_messages(&b.id, dm, 10).unwrap().len(), 1);
    assert_eq!(
        restored.leaderboard_score(&"score".into(), &a.id).unwrap(),
        Some(42)
    );
    assert_eq!(restored.rooms(None).len(), 1);

    let store = MemoryStore::new();
    restored.save_to(&store).unwrap();
    let loaded = Game::load_from_with_clock(&store, restored.config().clone(), clock)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.account_count(), 2);
    assert_eq!(loaded.active_announcements(&a.id).unwrap().len(), 1);
}
