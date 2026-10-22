//! Regression tests for the seven defects reported against the library.

use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};

use async_trait::async_trait;
use futures::executor::block_on;
use taixu::npc::NpcError;
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
// F1: NPC actions must be all-or-nothing.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct MissingLeaderboardAgent;

#[async_trait]
impl NpcAgent for MissingLeaderboardAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("here you go".into()),
            actions: vec![
                NpcAction::GiveReward {
                    account: ctx.speaker.clone(),
                    reward: Reward::new().currency("gold", 100),
                },
                NpcAction::SetScore {
                    leaderboard: "does_not_exist".into(),
                    account: ctx.speaker.clone(),
                    score: 1,
                },
            ],
        })
    }
}

#[derive(Debug)]
struct MissingItemAgent;

#[async_trait]
impl NpcAgent for MissingItemAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("two gifts".into()),
            actions: vec![
                NpcAction::GiveReward {
                    account: ctx.speaker.clone(),
                    reward: Reward::new().currency("gold", 100),
                },
                NpcAction::GiveReward {
                    account: ctx.speaker.clone(),
                    reward: Reward::new().item("no_such_item", 1),
                },
            ],
        })
    }
}

#[test]
fn failed_npc_turn_leaves_no_rewards_scores_or_chat() {
    let (game, _) = setup();
    let alice = account(&game, "alice");
    game.register_npc(NpcDef::new("guide", "Guide", "test"))
        .unwrap();

    let mut runtime = NpcRuntime::new();
    runtime.register("guide", Arc::new(MissingLeaderboardAgent));
    let error = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "hello"))
        .expect_err("missing leaderboard must fail the turn");
    assert!(matches!(
        error,
        NpcError::Game(GameError::LeaderboardNotFound(_))
    ));
    assert_eq!(game.currency_balance(&alice.id, &"gold".into()).unwrap(), 0);
    assert!(game.channel_history(&ChatChannel::Global, 10).is_empty());

    // An earlier reward is applied to a clone and must be rolled back when a
    // later reward references a missing item.
    runtime.register("guide", Arc::new(MissingItemAgent));
    let error = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "hello"))
        .expect_err("missing item must fail the turn");
    assert!(matches!(error, NpcError::Game(GameError::UnknownItem(_))));
    assert_eq!(game.currency_balance(&alice.id, &"gold".into()).unwrap(), 0);
    assert!(game.channel_history(&ChatChannel::Global, 10).is_empty());
}

#[derive(Debug)]
struct FullTurnAgent;

#[async_trait]
impl NpcAgent for FullTurnAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("done".into()),
            actions: vec![
                NpcAction::Say {
                    text: "a reward".into(),
                },
                NpcAction::GiveReward {
                    account: ctx.speaker.clone(),
                    reward: Reward::new().currency("gold", 5),
                },
                NpcAction::SetScore {
                    leaderboard: "board".into(),
                    account: ctx.speaker.clone(),
                    score: 42,
                },
            ],
        })
    }
}

#[test]
fn successful_npc_turn_commits_messages_rewards_and_scores() {
    let (game, _) = setup();
    let alice = account(&game, "alice");
    game.register_npc(NpcDef::new("guide", "Guide", "test"))
        .unwrap();
    game.register_leaderboard(Leaderboard::new(
        "board",
        "Board",
        ScoreOrder::HigherIsBetter,
    ))
    .unwrap();

    let mut runtime = NpcRuntime::new();
    runtime.register("guide", Arc::new(FullTurnAgent));
    let turn = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "hello")).unwrap();

    assert_eq!(turn.applied_actions, 2);
    assert_eq!(turn.npc_messages.len(), 2); // Say action + reply
    assert_eq!(game.currency_balance(&alice.id, &"gold".into()).unwrap(), 5);
    assert_eq!(
        game.leaderboard_score(&"board".into(), &alice.id).unwrap(),
        Some(42)
    );
    assert_eq!(
        game.channel_history(&ChatChannel::Global, 10)
            .iter()
            .map(|message| message.body.as_str())
            .collect::<Vec<_>>(),
        vec!["hello", "a reward", "done"]
    );
}

// ---------------------------------------------------------------------------
// F2: concurrent lazy account creation.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_ensure_account_resolves_to_one_account() {
    let game = Arc::new(Game::new());
    game.create_region("asia", "Asia", None, None).unwrap();

    let barrier = Arc::new(Barrier::new(16));
    let mut handles = Vec::new();
    for _ in 0..16 {
        let game = game.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            game.ensure_account("alice", "Alice", "asia")
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        results.iter().all(std::result::Result::is_ok),
        "all concurrent initializations must succeed: {results:?}"
    );
    assert_eq!(game.account_count(), 1);
    assert_eq!(game.account(&"alice".into()).unwrap().display_name, "Alice");
}

// ---------------------------------------------------------------------------
// F3: repeated wallet reads at one timestamp.
// ---------------------------------------------------------------------------

#[test]
fn repeated_wallet_reads_do_not_duplicate_regeneration() {
    let (game, clock) = setup();
    let alice = account(&game, "alice");
    game.register_resource(
        ResourceDef::new("plasma", "Plasma")
            .with_cap(1_000_000)
            .with_regen_per_hour(10_000_000),
    )
    .unwrap();
    game.set_resource(&alice.id, &"plasma".into(), 0).unwrap();

    clock.advance(1);
    let balance =
        |game: &Game| game.wallet(&alice.id).unwrap().resources[&ResourceId::from("plasma")];
    let first = balance(&game);
    let second = balance(&game);
    let third = balance(&game);
    assert_eq!(first, 2);
    assert_eq!(second, first);
    assert_eq!(third, first);
}

// ---------------------------------------------------------------------------
// F4: tasks completed inside their deadline remain claimable.
// ---------------------------------------------------------------------------

#[test]
fn timed_item_task_completed_in_window_survives_expiry() {
    let (game, clock) = setup();
    let alice = account(&game, "alice");
    game.register_item(ItemDef::new("potion", "Potion"))
        .unwrap();
    game.register_task(
        TaskDef::new("timed", "Timed delivery")
            .with_objective(TaskObjective::item("potion", 1))
            .with_reward(Reward::new().currency("gold", 50))
            .with_time_limit_ms(1_000),
    )
    .unwrap();
    game.accept_task(&alice.id, &"timed".into()).unwrap();

    clock.advance(500);
    game.add_item(&alice.id, &"potion".into(), 1).unwrap();
    // No task query between acquisition and expiry.
    clock.advance(1_500);
    game.claim_task(&alice.id, &"timed".into()).unwrap();
    assert_eq!(
        game.currency_balance(&alice.id, &"gold".into()).unwrap(),
        50
    );

    // Acquiring the item only after the deadline must not count.
    game.register_item(ItemDef::new("scroll", "Scroll"))
        .unwrap();
    game.register_task(
        TaskDef::new("late", "Too late")
            .with_objective(TaskObjective::item("scroll", 1))
            .with_reward(Reward::new().currency("gold", 10))
            .with_time_limit_ms(1_000),
    )
    .unwrap();
    game.accept_task(&alice.id, &"late".into()).unwrap();
    clock.advance(2_000);
    game.add_item(&alice.id, &"scroll".into(), 1).unwrap();
    assert!(matches!(
        game.claim_task(&alice.id, &"late".into()),
        Err(GameError::TaskNotCompleted(_))
    ));
}

// ---------------------------------------------------------------------------
// F5: maintenance keeps existing players active; closed stops activity.
// ---------------------------------------------------------------------------

#[test]
fn maintenance_allows_existing_players_to_use_rooms() {
    let game = Game::new();
    game.create_region("asia", "Asia", None, None).unwrap();
    game.create_region("eu", "Europe", None, None).unwrap();
    let a = account(&game, "a");
    let b = account(&game, "b");
    let d = account(&game, "d");
    let outsider = game.create_account("outsider", "Outsider", "eu").unwrap();

    game.set_region_status(&"asia".into(), RegionStatus::Maintenance)
        .unwrap();

    let room = game
        .create_room(&a.id, "Hangout", &"asia".into(), 8, BTreeMap::new())
        .expect("existing players can create rooms during maintenance");
    game.join_room(&b.id, &room.id)
        .expect("existing players can join rooms during maintenance");

    // New entry remains blocked.
    assert!(matches!(
        game.create_account("new", "New", "asia"),
        Err(GameError::RegionNotOpen(_))
    ));
    assert!(matches!(
        game.set_account_region(&outsider.id, "asia"),
        Err(GameError::RegionNotOpen(_))
    ));

    // Closed blocks room activity too.
    game.set_region_status(&"asia".into(), RegionStatus::Closed)
        .unwrap();
    assert!(matches!(
        game.create_room(&d.id, "Late", &"asia".into(), 4, BTreeMap::new()),
        Err(GameError::RegionNotOpen(_))
    ));
}

// ---------------------------------------------------------------------------
// F6: operator messages must not create orphan channels.
// ---------------------------------------------------------------------------

#[test]
fn system_messages_validate_channel_targets() {
    let (game, _) = setup();
    let a = account(&game, "a");
    let room = game
        .create_room(&a.id, "Room", &"asia".into(), 4, BTreeMap::new())
        .unwrap();

    assert!(matches!(
        game.send_system_message(ChatChannel::Room("missing".into()), "hello"),
        Err(GameError::RoomNotFound(_))
    ));
    assert!(matches!(
        game.send_system_message(ChatChannel::Region("missing".into()), "hello"),
        Err(GameError::RegionNotFound(_))
    ));
    assert!(matches!(
        game.send_system_message(
            ChatChannel::private(a.id.clone(), "missing".into()),
            "hello"
        ),
        Err(GameError::AccountNotFound(_))
    ));
    assert_eq!(game.read_state().chat.channel_count(), 0);

    let message = game
        .send_system_message(ChatChannel::Room(room.id.clone()), "maintenance soon")
        .unwrap();
    assert_eq!(message.body, "maintenance soon");
    assert_eq!(
        game.channel_history(&ChatChannel::Room(room.id), 10)
            .last()
            .unwrap()
            .body,
        "maintenance soon"
    );
}

// ---------------------------------------------------------------------------
// F7: prelude must not shadow std::result::Result inside NPC agent impls.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ReadmeAgent;

#[async_trait]
impl NpcAgent for ReadmeAgent {
    async fn respond(&self, _ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision::reply("Welcome, traveler!"))
    }
}

fn game_result_works() -> GameResult<()> {
    Ok(())
}

#[test]
fn prelude_result_does_not_conflict_with_trait_results() {
    let (game, _) = setup();
    let alice = account(&game, "alice");
    game.register_npc(NpcDef::new("guide", "Guide", "friend"))
        .unwrap();
    let mut runtime = NpcRuntime::new();
    runtime.register("guide", Arc::new(ReadmeAgent));
    let turn = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "hi")).unwrap();
    assert_eq!(turn.npc_messages[0].body, "Welcome, traveler!");
    game_result_works().unwrap();
}
