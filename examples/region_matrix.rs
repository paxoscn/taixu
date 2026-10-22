//! Maintenance vs Closed region behavior.
//!
//! Run with: `cargo run --example region_matrix`

use std::collections::BTreeMap;

use taixu::prelude::*;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let game = Game::new();
    game.create_region("asia", "Asia", None, None)?;
    game.create_region("eu", "Europe", None, None)?;
    let alice = game.create_account("alice", "Alice", "asia")?;
    let bob = game.create_account("bob", "Bob", "asia")?;
    let outsider = game.create_account("outsider", "Outsider", "eu")?;

    game.set_region_status(&"asia".into(), RegionStatus::Maintenance)?;
    let room = game.create_room(&alice.id, "Hangout", &"asia".into(), 8, BTreeMap::new())?;
    game.join_room(&bob.id, &room.id)?;
    game.send_message(
        &alice.id,
        ChatChannel::Room(room.id),
        "maintenance chat works",
    )?;
    println!("maintenance: existing players can use rooms and chat");

    match game.create_account("newbie", "Newbie", "asia") {
        Err(error) => println!("maintenance: new account rejected: {error}"),
        Ok(_) => println!("unexpected new account"),
    }
    match game.set_account_region(&outsider.id, "asia") {
        Err(error) => println!("maintenance: migration rejected: {error}"),
        Ok(_) => println!("unexpected migration"),
    }

    game.set_region_status(&"asia".into(), RegionStatus::Closed)?;
    match game.send_message(&alice.id, ChatChannel::Global, "closed") {
        Err(error) => println!("closed: player chat rejected: {error}"),
        Ok(_) => println!("unexpected chat"),
    }
    match game.create_room(&alice.id, "Another", &"asia".into(), 4, BTreeMap::new()) {
        Err(error) => println!("closed: room creation rejected: {error}"),
        Ok(_) => println!("unexpected room"),
    }
    // Reading and reward claims remain available while closed.
    println!(
        "closed: read-only account query = {:?}",
        game.account(&alice.id)?.region
    );
    Ok(())
}
