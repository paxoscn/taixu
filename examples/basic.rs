//! End-to-end tour of the library.
//!
//! Run with: `cargo run --example basic`

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::executor::block_on;
use taixu::prelude::*;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // ------------------------------------------------------------------
    // World setup: region (server node), accounts, catalogs.
    // ------------------------------------------------------------------
    let game = Game::new();
    game.create_region(
        "asia",
        "Asia",
        Some(5_000),
        Some("wss://asia.example.com".into()),
    )?;

    // Authentication happens outside this library; these ids are trusted.
    let alice = game.create_account("alice", "Alice", "asia")?;
    let bob = game.create_account("bob", "Bob", "asia")?;

    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
    game.register_item(ItemDef::new("sword", "Iron Sword").unique())?;
    game.register_resource(
        ResourceDef::new("energy", "Energy")
            .with_cap(100)
            .with_regen_per_hour(20),
    )?;
    game.register_skin(SkinDef::new(
        "gold_frame",
        "Gold Frame",
        SkinCategory::Frame,
    ))?;

    game.add_item(&alice.id, &"potion".into(), 6)?;
    game.add_currency(&alice.id, &"gold".into(), 500)?;
    game.set_resource(&alice.id, &"energy".into(), 80)?;
    game.add_item(&bob.id, &"sword".into(), 1)?;
    game.add_currency(&bob.id, &"gold".into(), 100)?;

    // ------------------------------------------------------------------
    // Shop: atomic purchase (payment + delivery + limit accounting).
    // ------------------------------------------------------------------
    let mut shop = Shop::new("general", "General Store");
    shop.add_offer(
        ShopOffer::new(
            "potion_pack",
            "Potion pack",
            Cost::new().currency("gold", 100),
            Reward::new().item("potion", 4).skin("gold_frame").xp(10),
        )
        .with_per_account_limit(5),
    )?;
    game.register_shop(shop)?;

    let receipt = game.purchase(&alice.id, &"general".into(), &"potion_pack".into(), 1)?;
    println!(
        "purchased {} x {} for {:?}",
        receipt.quantity, receipt.offer, receipt.paid
    );
    game.equip_skin(&alice.id, &"gold_frame".into())?;
    game.update_avatar(
        &alice.id,
        Avatar {
            image: Some("https://cdn.example.com/alice.png".into()),
            frame: Some("gold_frame".into()),
            title: Some("Trader".into()),
            ..Avatar::default()
        },
    )?;

    // ------------------------------------------------------------------
    // Rooms, chat, friends.
    // ------------------------------------------------------------------
    let room = game.create_room(
        &alice.id,
        "Hangout",
        &"asia".into(),
        8,
        BTreeMap::from([("topic".into(), "trading".into())]),
    )?;
    game.join_room(&bob.id, &room.id)?;
    game.send_message(&alice.id, ChatChannel::Room(room.id.clone()), "hello Bob!")?;

    game.send_friend_request(&alice.id, &bob.id, Some("from the hangout"))?;
    game.accept_friend_request(&bob.id, &alice.id)?;
    let dm = ChatChannel::private(alice.id.clone(), bob.id.clone());
    game.send_message(&alice.id, dm, "trade?")?;

    // ------------------------------------------------------------------
    // Player trade: both sides confirm, settlement is atomic.
    // ------------------------------------------------------------------
    let trade = game.create_trade(&alice.id, &bob.id)?;
    game.set_trade_side(
        &alice.id,
        &trade.id,
        TradeSide::new()
            .with_items(BTreeMap::from([("potion".into(), 3)]))
            .with_currencies(BTreeMap::from([("gold".into(), 20)])),
    )?;
    game.set_trade_side(
        &bob.id,
        &trade.id,
        TradeSide::new()
            .with_items(BTreeMap::from([("sword".into(), 1)]))
            .with_currencies(BTreeMap::from([("gold".into(), 30)])),
    )?;
    game.confirm_trade(&alice.id, &trade.id)?;
    let settled = game.confirm_trade(&bob.id, &trade.id)?;
    println!("trade state: {:?}", settled.state);

    // ------------------------------------------------------------------
    // Leaderboard, achievements, announcements, collections.
    // ------------------------------------------------------------------
    game.register_leaderboard(
        Leaderboard::new("weekly", "Weekly score", ScoreOrder::HigherIsBetter)
            .with_max_entries(100),
    )?;
    for (account, score) in [(&alice, 9_800), (&bob, 9_200)] {
        game.set_leaderboard_score(&"weekly".into(), &account.id, score)?;
    }
    println!("top: {:?}", game.leaderboard_top(&"weekly".into(), 10)?);

    game.register_achievement(AchievementDef::new(
        "first_trade",
        "First trade",
        "trades",
        1,
        Reward::new().currency("gold", 25),
    ))?;
    game.record_metric(&alice.id, "trades", 1)?;
    game.claim_achievement(&alice.id, &"first_trade".into())?;

    game.register_collection(
        CollectionDef::new("starter", "Starter album")
            .with_entries([
                Collectible::Item("potion".into()),
                Collectible::Skin("gold_frame".into()),
                Collectible::Achievement("first_trade".into()),
            ])
            .with_milestones([CollectionMilestone::new(
                2,
                Reward::new().currency("gold", 50),
            )]),
    )?;
    println!(
        "collection: {:?}",
        game.collection_progress(&alice.id, &"starter".into())?
    );
    game.claim_collection_milestone(&alice.id, &"starter".into(), 0)?;

    game.publish_announcement(
        AnnouncementDraft::new("Weekend event", "Double XP until Sunday!")
            .with_kind(AnnouncementKind::Event)
            .with_priority(10),
    )?;

    // ------------------------------------------------------------------
    // NPC agent in the room.
    // ------------------------------------------------------------------
    game.register_npc(
        NpcDef::new("barkeep", "Barkeep", "A cheerful tavern keeper.")
            .in_room(room.id.clone())
            .with_greeting("Welcome!"),
    )?;
    let mut npcs = NpcRuntime::new();
    npcs.register(
        "barkeep",
        Arc::new(
            RuleBasedNpc::new("Ask me about the quest board.")
                .with_rule("hello", "Welcome to the Hangout, friend!")
                .with_rule("potion", "Potions are always in demand here."),
        ),
    );
    let turn = block_on(npcs.converse(&game, &"barkeep".into(), &alice.id, "hello there"))?;
    println!("npc said: {}", turn.npc_messages[0].body);

    // ------------------------------------------------------------------
    // Snapshot for external persistence.
    // ------------------------------------------------------------------
    let json = serde_json::to_string(&game.snapshot())
        .map_err(|error| GameError::Persistence(error.to_string()))?;
    println!("state snapshot: {} bytes", json.len());

    println!(
        "alice: gold={} potions={} region={}",
        game.currency_balance(&alice.id, &"gold".into())?,
        game.item_count(&alice.id, &"potion".into())?,
        game.account(&alice.id)?.region
    );
    Ok(())
}
