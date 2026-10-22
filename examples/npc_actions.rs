//! A custom NPC agent that grants a reward and writes a leaderboard score.
//!
//! Run with: `cargo run --example npc_actions`

use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use taixu::npc::NpcError;
use taixu::prelude::*;

struct GiftAgent;

#[async_trait]
impl NpcAgent for GiftAgent {
    async fn respond(&self, context: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("A gift and a place on the board for you.".into()),
            actions: vec![
                NpcAction::GiveReward {
                    account: context.speaker.clone(),
                    reward: Reward::new().currency("gold", 10),
                },
                NpcAction::SetScore {
                    leaderboard: "weekly".into(),
                    account: context.speaker.clone(),
                    score: 100,
                },
            ],
        })
    }
}

struct BrokenAgent;

#[async_trait]
impl NpcAgent for BrokenAgent {
    async fn respond(&self, context: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("This turn must not commit.".into()),
            actions: vec![
                NpcAction::GiveReward {
                    account: context.speaker.clone(),
                    reward: Reward::new().currency("gold", 999),
                },
                NpcAction::SetScore {
                    leaderboard: "missing".into(),
                    account: context.speaker.clone(),
                    score: 1,
                },
            ],
        })
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let game = Game::new();
    game.create_region("asia", "Asia", None, None)?;
    let alice = game.create_account("alice", "Alice", "asia")?;
    game.register_leaderboard(Leaderboard::new(
        "weekly",
        "Weekly",
        ScoreOrder::HigherIsBetter,
    ))?;
    game.register_npc(NpcDef::new("guide", "Guide", "A friendly guide.").in_region("asia"))?;

    let mut runtime = NpcRuntime::new();
    runtime.register("guide", Arc::new(GiftAgent));
    let turn = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "hello"))?;
    println!(
        "applied_actions={} gold={} score={:?}",
        turn.applied_actions,
        game.currency_balance(&alice.id, &"gold".into())?,
        game.leaderboard_score(&"weekly".into(), &alice.id)?
    );

    // The second action fails, so the whole turn (including the reward and
    // chat messages) is discarded.
    runtime.register("guide", Arc::new(BrokenAgent));
    let before = game.currency_balance(&alice.id, &"gold".into())?;
    let error = block_on(runtime.converse(&game, &"guide".into(), &alice.id, "again"))
        .expect_err("missing leaderboard must fail");
    println!("broken turn rejected: {error}");
    assert_eq!(game.currency_balance(&alice.id, &"gold".into())?, before);
    Ok(())
}
