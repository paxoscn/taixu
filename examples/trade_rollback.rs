//! A trade whose offered asset disappears before settlement rolls back.
//!
//! Run with: `cargo run --example trade_rollback`

use std::collections::BTreeMap;

use taixu::prelude::*;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let game = Game::new();
    game.create_region("asia", "Asia", None, None)?;
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
    game.register_item(ItemDef::new("sword", "Sword").unique())?;

    let alice = game.create_account("alice", "Alice", "asia")?;
    let bob = game.create_account("bob", "Bob", "asia")?;
    game.add_item(&alice.id, &"potion".into(), 3)?;
    game.add_item(&bob.id, &"sword".into(), 1)?;

    let trade = game.create_trade(&alice.id, &bob.id)?;
    game.set_trade_side(
        &alice.id,
        &trade.id,
        TradeSide::new().with_items(BTreeMap::from([("potion".into(), 2)])),
    )?;
    game.set_trade_side(
        &bob.id,
        &trade.id,
        TradeSide::new().with_items(BTreeMap::from([("sword".into(), 1)])),
    )?;
    game.confirm_trade(&alice.id, &trade.id)?;

    // Bob consumes the sword after both sides were set.
    game.remove_item(&bob.id, &"sword".into(), 1)?;

    match game.confirm_trade(&bob.id, &trade.id) {
        Ok(_) => println!("unexpected success"),
        Err(error) => println!("settlement failed as expected: {error}"),
    }

    let after = game.trade(&trade.id)?;
    println!(
        "state={:?} alice_confirmed={} bob_confirmed={} alice_potions={}",
        after.state,
        after.initiator_confirmed,
        after.partner_confirmed,
        game.item_count(&alice.id, &"potion".into())?
    );
    assert_eq!(after.state, TradeState::Open);
    assert!(!after.initiator_confirmed && !after.partner_confirmed);
    assert_eq!(game.item_count(&alice.id, &"potion".into())?, 3);
    Ok(())
}
