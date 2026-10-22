//! Versioned snapshot round-trip and the in-memory GameStore.
//!
//! Run with: `cargo run --example persistence`

use std::sync::Arc;

use taixu::prelude::*;
use taixu::store::MemoryStore;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("asia", "Asia", None, None)?;
    let alice = game.create_account("alice", "Alice", "asia")?;
    game.register_item(ItemDef::new("potion", "Potion"))?;
    game.add_item(&alice.id, &"potion".into(), 3)?;

    let json = serde_json::to_string(&game.snapshot())?;
    let state: GameState = serde_json::from_str(&json)?;
    println!("snapshot version = {}", state.version);
    assert_eq!(state.version, GAME_STATE_VERSION);

    let restored = Game::from_state(state, game.config().clone(), clock.clone());
    println!(
        "restored accounts={} potions={}",
        restored.account_count(),
        restored.item_count(&alice.id, &"potion".into())?
    );

    let store = MemoryStore::new();
    game.save_to(&store)?;
    let loaded =
        Game::load_from_with_clock(&store, game.config().clone(), clock)?.expect("snapshot exists");
    println!(
        "store reload accounts={} version={}",
        loaded.account_count(),
        loaded.state_version()
    );
    Ok(())
}
