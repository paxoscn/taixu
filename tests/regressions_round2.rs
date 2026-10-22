//! Regression tests for the second-round findings (G1–G9).

use std::collections::BTreeMap;
use std::sync::Arc;

use taixu::prelude::*;

fn setup() -> (Game, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("asia", "Asia", None, None).unwrap();
    game.create_region("eu", "Europe", None, None).unwrap();
    (game, clock)
}

fn account(game: &Game, id: &str) -> Account {
    game.create_account(id, id, "asia").unwrap()
}

// ---------------------------------------------------------------------------
// G1: a repeatable task presents a fresh cycle after cooldown.
// ---------------------------------------------------------------------------

#[test]
fn repeatable_task_starts_fresh_cycle_at_cooldown_boundary() {
    let (game, clock) = setup();
    let alice = account(&game, "alice");
    game.register_task(
        TaskDef::new("daily", "Daily")
            .with_category("daily")
            .with_objective(TaskObjective::metric("kills", 1))
            .with_reward(Reward::new().currency("gold", 10))
            .with_repeat(TaskRepeat::cooldown(1_000)),
    )
    .unwrap();

    game.accept_task(&alice.id, &"daily".into()).unwrap();
    game.record_event(&alice.id, "kills", 1).unwrap();
    game.claim_task(&alice.id, &"daily".into()).unwrap();

    clock.advance(999);
    let cooling = game.task_view(&alice.id, &"daily".into()).unwrap();
    assert_eq!(cooling.status, TaskStatus::Claimed);
    assert_eq!(cooling.objectives[0].progress, 1);
    assert!(cooling.next_available_at_ms.is_some());

    clock.advance(1); // exact cooldown boundary
    let fresh = game.task_view(&alice.id, &"daily".into()).unwrap();
    assert_eq!(fresh.status, TaskStatus::Available);
    assert_eq!(fresh.objectives[0].progress, 0);
    assert_eq!(fresh.accepted_at_ms, None);
    assert_eq!(fresh.completed_at_ms, None);
    assert_eq!(fresh.claimed_at_ms, None);
    assert_eq!(fresh.next_available_at_ms, None);
}

#[test]
fn repeatable_task_locked_after_cooldown_is_locked_with_fresh_data() {
    let (game, clock) = setup();
    let alice = account(&game, "alice");
    game.register_task(
        TaskDef::new("regional", "Regional")
            .with_objective(TaskObjective::metric("kills", 1))
            .with_repeat(TaskRepeat::cooldown(1_000))
            .available_in(["asia".into()]),
    )
    .unwrap();

    game.accept_task(&alice.id, &"regional".into()).unwrap();
    game.record_event(&alice.id, "kills", 1).unwrap();
    game.claim_task(&alice.id, &"regional".into()).unwrap();
    clock.advance(1_000);
    game.set_account_region(&alice.id, "eu").unwrap();

    let view = game.task_view(&alice.id, &"regional".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Locked);
    assert_eq!(view.objectives[0].progress, 0);
    assert_eq!(view.claimed_at_ms, None);
    assert_eq!(view.regions.as_ref().unwrap().len(), 1);
}

#[test]
fn repeatable_item_objective_has_no_stale_cycle_timestamps() {
    let (game, clock) = setup();
    let alice = account(&game, "alice");
    game.register_item(ItemDef::new("potion", "Potion"))
        .unwrap();
    game.register_task(
        TaskDef::new("delivery", "Delivery")
            .with_objective(TaskObjective::item("potion", 1))
            .with_repeat(TaskRepeat::cooldown(1_000)),
    )
    .unwrap();

    game.accept_task(&alice.id, &"delivery".into()).unwrap();
    game.add_item(&alice.id, &"potion".into(), 1).unwrap();
    game.claim_task(&alice.id, &"delivery".into()).unwrap();
    clock.advance(1_000);

    let view = game.task_view(&alice.id, &"delivery".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Available);
    assert_eq!(view.accepted_at_ms, None);
    assert_eq!(view.completed_at_ms, None);
    assert_eq!(view.claimed_at_ms, None);
    // Item progress is derived from live inventory, so the still-held potion
    // shows 1/1 even in a fresh cycle.
    assert_eq!(view.objectives[0].progress, 1);

    game.remove_item(&alice.id, &"potion".into(), 1).unwrap();
    let view = game.task_view(&alice.id, &"delivery".into()).unwrap();
    assert_eq!(view.objectives[0].progress, 0);
}

// ---------------------------------------------------------------------------
// G2: lock reasons are exposed.
// ---------------------------------------------------------------------------

#[test]
fn task_view_exposes_requirements_region_and_window() {
    let (game, _) = setup();
    let alice = account(&game, "alice");
    game.register_task(
        TaskDef::new("locked", "Locked")
            .with_objective(TaskObjective::metric("kills", 1))
            .with_prerequisite("intro")
            .with_min_level(5)
            .available_in(["asia".into()])
            .with_window(Some(2_000_000), Some(3_000_000)),
    )
    .unwrap();

    let view = game.task_view(&alice.id, &"locked".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Locked);
    assert_eq!(view.requires, vec![TaskId::from("intro")]);
    assert_eq!(view.min_level, Some(5));
    assert!(view
        .regions
        .as_ref()
        .is_some_and(|regions| regions.contains(&RegionId::from("asia"))));
    assert_eq!(view.starts_at_ms, Some(2_000_000));
    assert_eq!(view.ends_at_ms, Some(3_000_000));
    let value = serde_json::to_value(&view).unwrap();
    for field in ["requires", "regions", "starts_at_ms", "ends_at_ms"] {
        assert!(value.get(field).is_some());
    }
}

// ---------------------------------------------------------------------------
// G3: per-account channel discovery.
// ---------------------------------------------------------------------------

#[test]
fn channels_for_returns_only_accessible_channels_newest_first() {
    let (game, clock) = setup();
    let a = account(&game, "a");
    let b = account(&game, "b");
    let c = account(&game, "c");
    let room = game
        .create_room(&a.id, "Room", &"asia".into(), 8, BTreeMap::new())
        .unwrap();
    game.join_room(&b.id, &room.id).unwrap();

    game.send_message(&a.id, ChatChannel::Global, "global")
        .unwrap();
    clock.advance(1);
    game.send_message(&a.id, ChatChannel::Region("asia".into()), "region")
        .unwrap();
    clock.advance(1);
    game.send_message(&a.id, ChatChannel::Room(room.id.clone()), "room")
        .unwrap();
    clock.advance(1);
    game.send_message(
        &a.id,
        ChatChannel::private(a.id.clone(), b.id.clone()),
        "dm a-b",
    )
    .unwrap();
    clock.advance(1);
    game.send_message(
        &b.id,
        ChatChannel::private(b.id.clone(), c.id.clone()),
        "dm b-c",
    )
    .unwrap();

    let a_channels = game.channels_for(&a.id).unwrap();
    assert_eq!(a_channels.len(), 4);
    assert!(a_channels
        .windows(2)
        .all(|pair| pair[0].last_message_at_ms >= pair[1].last_message_at_ms));
    assert!(!a_channels
        .iter()
        .any(|summary| { summary.channel == ChatChannel::private(b.id.clone(), c.id.clone()) }));

    let c_channels = game.channels_for(&c.id).unwrap();
    assert!(!c_channels
        .iter()
        .any(|summary| summary.channel == ChatChannel::Room(room.id.clone())));

    game.block_account(&b.id, &a.id).unwrap();
    let a_channels = game.channels_for(&a.id).unwrap();
    assert!(!a_channels
        .iter()
        .any(|summary| summary.channel == ChatChannel::private(a.id.clone(), b.id.clone())));
}

// ---------------------------------------------------------------------------
// G4: trade listing is chronological.
// ---------------------------------------------------------------------------

#[test]
fn trades_for_is_newest_first() {
    let (game, clock) = setup();
    let a = account(&game, "a");
    let b = account(&game, "b");
    for _ in 0..8 {
        game.create_trade(&a.id, &b.id).unwrap();
        clock.advance(1);
    }
    let trades = game.trades_for(&a.id).unwrap();
    assert!(trades
        .windows(2)
        .all(|pair| pair[0].created_at_ms >= pair[1].created_at_ms));
}

// ---------------------------------------------------------------------------
// G6: Closed regions reject new player activity.
// ---------------------------------------------------------------------------

#[test]
fn closed_region_rejects_new_player_activity() {
    let (game, _) = setup();
    let a = account(&game, "a");
    let b = account(&game, "b");
    game.register_item(ItemDef::new("potion", "Potion"))
        .unwrap();
    game.add_currency(&a.id, &"gold".into(), 100).unwrap();
    let mut shop = Shop::new("shop", "Shop");
    shop.add_offer(ShopOffer::new(
        "offer",
        "Offer",
        Cost::new().currency("gold", 10),
        Reward::new().item("potion", 1),
    ))
    .unwrap();
    game.register_shop(shop).unwrap();
    game.register_task(
        TaskDef::new("task", "Task").with_objective(TaskObjective::metric("kills", 1)),
    )
    .unwrap();

    game.set_region_status(&"asia".into(), RegionStatus::Closed)
        .unwrap();

    assert!(matches!(
        game.purchase(&a.id, &"shop".into(), &"offer".into(), 1),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.send_message(&a.id, ChatChannel::Global, "hello"),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.send_message(&a.id, ChatChannel::Region("asia".into()), "hello"),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.send_friend_request(&a.id, &b.id, None),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.accept_task(&a.id, &"task".into()),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.create_trade(&a.id, &b.id),
        Err(GameError::RegionNotOpen(_))
    ));

    // Read-only and cosmetic operations remain available.
    assert_eq!(game.account(&a.id).unwrap().region, "asia".into());
    game.update_avatar(&a.id, Avatar::default()).unwrap();
}

// ---------------------------------------------------------------------------
// G7: snapshot schema version.
// ---------------------------------------------------------------------------

#[test]
fn game_state_carries_a_schema_version() {
    let game = Game::new();
    let value = serde_json::to_value(game.snapshot()).unwrap();
    assert_eq!(
        value["version"].as_u64(),
        Some(u64::from(GAME_STATE_VERSION))
    );
    assert_eq!(game.state_version(), GAME_STATE_VERSION);

    // Snapshots from before the version field default to the current schema.
    let mut legacy = value.clone();
    legacy.as_object_mut().unwrap().remove("version");
    let state: GameState = serde_json::from_value(legacy).unwrap();
    assert_eq!(state.version, GAME_STATE_VERSION);

    let unset = GameState {
        version: 0,
        ..GameState::default()
    };
    let clock = Arc::new(ManualClock::new(0));
    let restored = Game::from_state(unset, GameConfig::default(), clock);
    assert_eq!(restored.state_version(), GAME_STATE_VERSION);
}

// ---------------------------------------------------------------------------
// G8: guard-free read access.
// ---------------------------------------------------------------------------

#[test]
fn with_state_runs_a_short_read_query() {
    let (game, _) = setup();
    account(&game, "a");
    let count = game.with_state(|state| state.accounts.len());
    assert_eq!(count, 1);
    // The guard is dropped when the closure returns, so write methods follow
    // safely.
    game.add_currency(&"a".into(), &"gold".into(), 5).unwrap();
}

// ---------------------------------------------------------------------------
// G9: avatar and equipped skins stay in sync.
// ---------------------------------------------------------------------------

#[test]
fn avatar_and_equipped_skins_stay_in_sync() {
    let (game, _) = setup();
    let a = account(&game, "a");
    for skin in [
        SkinDef::new("frame1", "Frame 1", SkinCategory::Frame),
        SkinDef::new("frame2", "Frame 2", SkinCategory::Frame),
        SkinDef::new("badge", "Badge", SkinCategory::Badge),
    ] {
        game.register_skin(skin).unwrap();
    }
    for skin in ["frame1", "frame2", "badge"] {
        game.grant_skin(&a.id, &skin.into()).unwrap();
    }

    game.equip_skin(&a.id, &"frame1".into()).unwrap();
    assert_eq!(
        game.account(&a.id).unwrap().avatar.frame,
        Some(SkinId::from("frame1"))
    );

    game.update_avatar(
        &a.id,
        Avatar {
            frame: Some("frame2".into()),
            badge: Some("badge".into()),
            ..Avatar::default()
        },
    )
    .unwrap();
    let equipped = game.equipped_skins(&a.id).unwrap();
    assert_eq!(equipped[&SkinCategory::Frame], SkinId::from("frame2"));
    assert_eq!(equipped[&SkinCategory::Badge], SkinId::from("badge"));

    game.unequip_skin(&a.id, &SkinCategory::Frame).unwrap();
    assert_eq!(game.account(&a.id).unwrap().avatar.frame, None);
    assert!(!game
        .equipped_skins(&a.id)
        .unwrap()
        .contains_key(&SkinCategory::Frame));

    game.equip_skin(&a.id, &"frame1".into()).unwrap();
    game.update_avatar(&a.id, Avatar::default()).unwrap();
    let account = game.account(&a.id).unwrap();
    assert_eq!(account.avatar.frame, None);
    assert_eq!(account.avatar.badge, None);
    assert!(game.equipped_skins(&a.id).unwrap().is_empty());
}
