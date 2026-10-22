//! Task objectives, repeats and fresh-cycle views.
//!
//! Run with: `cargo run --example tasks`

use std::sync::Arc;

use taixu::prelude::*;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("asia", "Asia", None, None)?;
    let alice = game.create_account("alice", "Alice", "asia")?;

    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
    game.register_skin(SkinDef::new("frame", "Frame", SkinCategory::Frame))?;
    game.register_achievement(AchievementDef::new(
        "first_blood",
        "First blood",
        "kills",
        2,
        Reward::new().currency("gold", 5),
    ))?;

    game.register_task(
        TaskDef::new("collector", "Collector")
            .with_category("daily")
            .with_objective(TaskObjective::metric("kills", 2))
            .with_objective(TaskObjective::item("potion", 1))
            .with_objective(TaskObjective::skin("frame"))
            .with_objective(TaskObjective::achievement("first_blood"))
            .with_reward(Reward::new().currency("gold", 100))
            .with_repeat(TaskRepeat::cooldown(1_000)),
    )?;
    game.accept_task(&alice.id, &"collector".into())?;

    // Item and skin objectives are evaluated from live state.
    game.add_item(&alice.id, &"potion".into(), 1)?;
    game.grant_skin(&alice.id, &"frame".into())?;

    // Metric objective + achievement + item/skin objectives complete together.
    let outcome = game.record_event(&alice.id, "kills", 2)?;
    println!(
        "achievements={:?} tasks={:?}",
        outcome.achievements, outcome.tasks
    );

    let view = game.task_view(&alice.id, &"collector".into())?;
    println!("status={:?} objectives={:?}", view.status, view.objectives);
    game.claim_task(&alice.id, &"collector".into())?;

    clock.advance(1_000);
    let fresh = game.task_view(&alice.id, &"collector".into())?;
    println!(
        "after cooldown: status={:?} progress={} claimed_at={:?}",
        fresh.status, fresh.objectives[0].progress, fresh.claimed_at_ms
    );
    Ok(())
}
