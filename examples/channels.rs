//! Per-account channel discovery with `channels_for`.
//!
//! Run with: `cargo run --example channels`

use std::collections::BTreeMap;
use std::sync::Arc;

use taixu::prelude::*;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let clock = Arc::new(ManualClock::new(1_000_000));
    let game = Game::with_clock(clock.clone());
    game.create_region("asia", "Asia", None, None)?;
    let alice = game.create_account("alice", "Alice", "asia")?;
    let bob = game.create_account("bob", "Bob", "asia")?;
    let room = game.create_room(&alice.id, "Room", &"asia".into(), 8, BTreeMap::new())?;
    game.join_room(&bob.id, &room.id)?;

    game.send_message(&alice.id, ChatChannel::Global, "global hello")?;
    clock.advance(1);
    game.send_message(
        &alice.id,
        ChatChannel::Region("asia".into()),
        "region hello",
    )?;
    clock.advance(1);
    game.send_message(&alice.id, ChatChannel::Room(room.id), "room hello")?;
    clock.advance(1);
    game.send_message(
        &alice.id,
        ChatChannel::private(alice.id.clone(), bob.id.clone()),
        "dm hello",
    )?;

    for summary in game.channels_for(&alice.id)? {
        println!(
            "{} messages={} last={:?}",
            summary.channel, summary.message_count, summary.last_message_at_ms
        );
    }
    Ok(())
}
