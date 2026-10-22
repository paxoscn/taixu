use std::sync::Arc;

use taixu::prelude::*;

const DAY_MS: i64 = 86_400_000;

fn setup() -> (Game, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("eu", "Europe", None, None).unwrap();
    game.create_region("us", "United States", None, None)
        .unwrap();
    (game, clock)
}

fn account(game: &Game, id: &str) -> Account {
    game.create_account(id, id, "eu").unwrap()
}

fn status(game: &Game, account: &AccountId, task: &str) -> TaskStatus {
    game.task_view(account, &task.into()).unwrap().status
}

#[test]
fn task_lifecycle_progress_and_claim() {
    let (game, _) = setup();
    let a = account(&game, "a");
    game.register_task(
        TaskDef::new("first_steps", "First Steps")
            .with_category("main")
            .with_objective(TaskObjective::metric("kills", 3))
            .with_reward(Reward::new().currency("gold", 50)),
    )
    .unwrap();

    let available = game.available_tasks(&a.id).unwrap();
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].status, TaskStatus::Available);

    let view = game.accept_task(&a.id, &"first_steps".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Active);
    assert!(matches!(
        game.accept_task(&a.id, &"first_steps".into()),
        Err(GameError::TaskAlreadyAccepted(_))
    ));

    let outcome = game.record_event(&a.id, "kills", 2).unwrap();
    assert!(outcome.tasks.is_empty());
    let view = game.task_view(&a.id, &"first_steps".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Active);
    assert_eq!(view.objectives[0].progress, 2);
    assert_eq!(view.objectives[0].target, 3);
    assert!(!view.objectives[0].complete);

    // The legacy achievement entry point also advances tasks.
    let achievements = game.record_metric(&a.id, "kills", 1).unwrap();
    assert!(achievements.is_empty());
    let view = game.task_view(&a.id, &"first_steps".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Completed);
    assert!(view.completed_at_ms.is_some());
    assert_eq!(game.completed_tasks(&a.id).unwrap().len(), 1);
    assert!(game
        .available_tasks(&a.id)
        .unwrap()
        .iter()
        .all(|task| task.id != "first_steps".into()));

    let reward = game.claim_task(&a.id, &"first_steps".into()).unwrap();
    assert_eq!(reward.currencies[0].amount, 50);
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 50);
    assert_eq!(status(&game, &a.id, "first_steps"), TaskStatus::Claimed);
    assert!(matches!(
        game.claim_task(&a.id, &"first_steps".into()),
        Err(GameError::TaskAlreadyClaimed(_))
    ));
    assert!(matches!(
        game.accept_task(&a.id, &"first_steps".into()),
        Err(GameError::TaskAlreadyClaimed(_))
    ));
    assert!(game.active_tasks(&a.id).unwrap().is_empty());
    assert!(game.completed_tasks(&a.id).unwrap().is_empty());
}

#[test]
fn task_prerequisites_level_region_window_and_hidden() {
    let (game, clock) = setup();
    let a = account(&game, "a");
    let b = game.create_account("b", "B", "us").unwrap();
    let start = clock.now_ms() + 10_000;

    game.register_task(
        TaskDef::new("intro", "Intro")
            .with_objective(TaskObjective::metric("step", 1))
            .with_reward(Reward::new().currency("gold", 1)),
    )
    .unwrap();
    game.register_task(
        TaskDef::new("followup", "Follow-up")
            .with_objective(TaskObjective::metric("step", 1))
            .with_prerequisite("intro")
            .with_min_level(2),
    )
    .unwrap();
    game.register_task(
        TaskDef::new("secret", "Secret")
            .with_objective(TaskObjective::metric("whispers", 1))
            .hidden(),
    )
    .unwrap();
    game.register_task(
        TaskDef::new("us_only", "US only")
            .with_objective(TaskObjective::metric("step", 1))
            .available_in(["us".into()]),
    )
    .unwrap();
    game.register_task(
        TaskDef::new("future", "Future")
            .with_objective(TaskObjective::metric("step", 1))
            .with_window(Some(start), None),
    )
    .unwrap();

    let available: Vec<_> = game
        .available_tasks(&a.id)
        .unwrap()
        .into_iter()
        .map(|task| task.id)
        .collect();
    assert_eq!(available, vec![TaskId::from("intro")]);
    assert_eq!(status(&game, &a.id, "followup"), TaskStatus::Locked);
    assert_eq!(status(&game, &a.id, "us_only"), TaskStatus::Locked);
    assert_eq!(status(&game, &a.id, "future"), TaskStatus::Locked);
    assert!(game
        .task_views(&a.id)
        .unwrap()
        .iter()
        .all(|task| task.id != "secret".into()));

    assert!(matches!(
        game.accept_task(&a.id, &"followup".into()),
        Err(GameError::TaskLocked(_))
    ));

    game.accept_task(&a.id, &"intro".into()).unwrap();
    game.record_event(&a.id, "step", 1).unwrap();
    game.claim_task(&a.id, &"intro".into()).unwrap();
    assert_eq!(status(&game, &a.id, "followup"), TaskStatus::Locked); // still level 1
    game.set_level(&a.id, 2).unwrap();
    assert_eq!(status(&game, &a.id, "followup"), TaskStatus::Available);

    // Hidden tasks can be accepted by id and then become visible.
    game.accept_task(&a.id, &"secret".into()).unwrap();
    assert!(game
        .task_views(&a.id)
        .unwrap()
        .iter()
        .any(|task| task.id == "secret".into()));

    // Region targeting.
    let b_available: Vec<_> = game
        .available_tasks(&b.id)
        .unwrap()
        .into_iter()
        .map(|task| task.id)
        .collect();
    assert!(b_available.contains(&"us_only".into()));
    assert!(!game
        .available_tasks(&a.id)
        .unwrap()
        .iter()
        .any(|task| task.id == "us_only".into()));

    // Time window.
    clock.set(start);
    assert_eq!(status(&game, &a.id, "future"), TaskStatus::Available);
}

#[test]
fn task_repeat_expiry_abandon_and_item_objectives() {
    let (game, clock) = setup();
    let a = account(&game, "a");
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))
        .unwrap();

    game.register_task(
        TaskDef::new("daily_hunt", "Daily hunt")
            .with_category("daily")
            .with_objective(TaskObjective::metric("hunt", 1))
            .with_reward(Reward::new().currency("gold", 10))
            .with_repeat(TaskRepeat::daily()),
    )
    .unwrap();
    game.accept_task(&a.id, &"daily_hunt".into()).unwrap();
    game.record_event(&a.id, "hunt", 1).unwrap();
    game.claim_task(&a.id, &"daily_hunt".into()).unwrap();

    assert_eq!(status(&game, &a.id, "daily_hunt"), TaskStatus::Claimed);
    let view = game.task_view(&a.id, &"daily_hunt".into()).unwrap();
    assert_eq!(view.next_available_at_ms, Some(clock.now_ms() + DAY_MS));
    assert!(matches!(
        game.accept_task(&a.id, &"daily_hunt".into()),
        Err(GameError::TaskOnCooldown { .. })
    ));

    clock.advance(DAY_MS);
    assert_eq!(status(&game, &a.id, "daily_hunt"), TaskStatus::Available);
    let view = game.accept_task(&a.id, &"daily_hunt".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Active);
    assert_eq!(view.objectives[0].progress, 0, "counters reset per cycle");

    // Abandoning an accepted task makes it available again immediately.
    game.abandon_task(&a.id, &"daily_hunt".into()).unwrap();
    assert_eq!(status(&game, &a.id, "daily_hunt"), TaskStatus::Available);
    assert!(matches!(
        game.abandon_task(&a.id, &"daily_hunt".into()),
        Err(GameError::TaskNotAccepted(_))
    ));
    game.accept_task(&a.id, &"daily_hunt".into()).unwrap();
    assert!(matches!(
        game.claim_task(&a.id, &"daily_hunt".into()),
        Err(GameError::TaskNotCompleted(_))
    ));

    // A timed task expires and can be retried; completion before the deadline
    // remains claimable afterwards.
    game.register_task(
        TaskDef::new("timed", "Timed")
            .with_objective(TaskObjective::metric("dash", 1))
            .with_time_limit_ms(1_000),
    )
    .unwrap();
    game.accept_task(&a.id, &"timed".into()).unwrap();
    clock.advance(2_000);
    assert_eq!(status(&game, &a.id, "timed"), TaskStatus::Expired);
    game.accept_task(&a.id, &"timed".into()).unwrap();
    assert_eq!(status(&game, &a.id, "timed"), TaskStatus::Active);
    game.record_event(&a.id, "dash", 1).unwrap();
    clock.advance(2_000);
    let view = game.task_view(&a.id, &"timed".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Completed);
    game.claim_task(&a.id, &"timed".into()).unwrap();

    // Item objectives are evaluated from live inventory, even without events.
    game.register_task(
        TaskDef::new("gather", "Gather potions")
            .with_objective(TaskObjective::item("potion", 3))
            .with_reward(Reward::new().currency("gold", 5)),
    )
    .unwrap();
    game.accept_task(&a.id, &"gather".into()).unwrap();
    game.add_item(&a.id, &"potion".into(), 2).unwrap();
    assert_eq!(status(&game, &a.id, "gather"), TaskStatus::Active);
    game.add_item(&a.id, &"potion".into(), 1).unwrap();
    assert_eq!(status(&game, &a.id, "gather"), TaskStatus::Completed);
    game.claim_task(&a.id, &"gather".into()).unwrap();
}

#[test]
fn record_event_completes_achievement_and_task_together() {
    let (game, _) = setup();
    let a = account(&game, "a");
    game.register_achievement(AchievementDef::new(
        "slayer",
        "Slayer",
        "kills",
        2,
        Reward::new().currency("gold", 1),
    ))
    .unwrap();
    game.register_task(
        TaskDef::new("slayer_task", "Slayer task")
            .with_objective(TaskObjective::achievement("slayer"))
            .with_reward(Reward::new().currency("gold", 20)),
    )
    .unwrap();

    game.accept_task(&a.id, &"slayer_task".into()).unwrap();
    let outcome = game.record_event(&a.id, "kills", 2).unwrap();
    assert_eq!(outcome.achievements, vec![AchievementId::from("slayer")]);
    assert_eq!(outcome.tasks, vec![TaskId::from("slayer_task")]);
    assert_eq!(status(&game, &a.id, "slayer_task"), TaskStatus::Completed);
    game.claim_task(&a.id, &"slayer_task".into()).unwrap();
    assert_eq!(game.currency_balance(&a.id, &"gold".into()).unwrap(), 20);
}

#[test]
fn task_state_survives_snapshot_roundtrip() {
    let (game, clock) = setup();
    let a = account(&game, "a");
    game.register_task(
        TaskDef::new("persistent", "Persistent")
            .with_objective(TaskObjective::metric("steps", 5))
            .with_repeat(TaskRepeat::weekly()),
    )
    .unwrap();
    game.accept_task(&a.id, &"persistent".into()).unwrap();
    game.record_event(&a.id, "steps", 2).unwrap();

    let json = serde_json::to_string(&game.snapshot()).unwrap();
    let restored_state: GameState = serde_json::from_str(&json).unwrap();
    let restored = Game::from_state(restored_state, game.config().clone(), clock);

    let view = restored.task_view(&a.id, &"persistent".into()).unwrap();
    assert_eq!(view.status, TaskStatus::Active);
    assert_eq!(view.objectives[0].progress, 2);
    assert_eq!(view.repeat, TaskRepeat::weekly());
}
