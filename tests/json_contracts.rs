//! Locks down the documented serde wire shapes.

use std::collections::BTreeMap;

use taixu::prelude::*;

#[test]
fn chat_channels_use_documented_string_encoding() {
    assert_eq!(
        serde_json::to_string(&ChatChannel::Global).unwrap(),
        "\"global\""
    );
    assert_eq!(
        serde_json::to_string(&ChatChannel::Region("asia".into())).unwrap(),
        "\"region:asia\""
    );
    assert_eq!(
        serde_json::to_string(&ChatChannel::Room("room_1".into())).unwrap(),
        "\"room:room_1\""
    );

    let dm = ChatChannel::private("bob".into(), "alice".into());
    let json = serde_json::to_string(&dm).unwrap();
    assert_eq!(json, "\"dm:5:alicebob\"");
    assert_eq!(serde_json::from_str::<ChatChannel>(&json).unwrap(), dm);

    // The first id may contain ':' because the prefix is byte-length encoded.
    let tricky = ChatChannel::private("a:b".into(), "c".into());
    let json = serde_json::to_string(&tricky).unwrap();
    assert_eq!(json, "\"dm:3:a:bc\"");
    assert_eq!(serde_json::from_str::<ChatChannel>(&json).unwrap(), tricky);
}

#[test]
fn chat_author_and_message_shapes() {
    assert_eq!(
        serde_json::to_value(ChatAuthor::account("alice")).unwrap(),
        serde_json::json!({"kind": "account", "account": "alice"})
    );
    assert_eq!(
        serde_json::to_value(ChatAuthor::npc("guide")).unwrap(),
        serde_json::json!({"kind": "npc", "npc": "guide"})
    );
    assert_eq!(
        serde_json::to_value(ChatAuthor::System).unwrap(),
        serde_json::json!({"kind": "system"})
    );

    let message = ChatMessage {
        id: MessageId::from("msg_1"),
        channel: ChatChannel::Global,
        author: ChatAuthor::account("alice"),
        body: "hi".into(),
        sent_at_ms: 1_700_000_000_000,
        metadata: BTreeMap::from([("client_id".into(), "abc".into())]),
    };
    let value = serde_json::to_value(&message).unwrap();
    assert_eq!(value["channel"], "global");
    assert_eq!(value["author"]["kind"], "account");
    assert_eq!(value["metadata"]["client_id"], "abc");
}

#[test]
fn channel_summary_shape() {
    let summary = ChannelSummary {
        channel: ChatChannel::Room("room_1".into()),
        last_message_at_ms: Some(1_700_000_000_000),
        message_count: 42,
    };
    assert_eq!(
        serde_json::to_value(&summary).unwrap(),
        serde_json::json!({
            "channel": "room:room_1",
            "last_message_at_ms": 1_700_000_000_000_i64,
            "message_count": 42
        })
    );
}

#[test]
fn task_objectives_and_repeats_are_tagged() {
    assert_eq!(
        serde_json::to_value(TaskObjective::metric("kills", 3)).unwrap(),
        serde_json::json!({"type": "metric", "metric": "kills", "target": 3})
    );
    assert_eq!(
        serde_json::to_value(TaskObjective::item("potion", 2)).unwrap(),
        serde_json::json!({"type": "item", "item": "potion", "quantity": 2})
    );
    assert_eq!(
        serde_json::to_value(TaskObjective::skin("gold_frame")).unwrap(),
        serde_json::json!({"type": "skin", "skin": "gold_frame"})
    );
    assert_eq!(
        serde_json::to_value(TaskObjective::achievement("first_blood")).unwrap(),
        serde_json::json!({"type": "achievement", "achievement": "first_blood"})
    );

    assert_eq!(
        serde_json::to_value(TaskRepeat::Once).unwrap(),
        serde_json::json!({"kind": "once"})
    );
    assert_eq!(
        serde_json::to_value(TaskRepeat::daily()).unwrap(),
        serde_json::json!({"kind": "cooldown", "duration_ms": 86_400_000_i64})
    );
    assert_eq!(
        serde_json::to_value(TaskStatus::Locked).unwrap(),
        serde_json::json!("locked")
    );
}

#[test]
fn task_view_exposes_documented_fields() {
    let game = Game::new();
    game.create_region("asia", "Asia", None, None).unwrap();
    let alice = game.create_account("alice", "Alice", "asia").unwrap();
    game.register_task(
        TaskDef::new("daily", "Daily")
            .with_objective(TaskObjective::metric("kills", 2))
            .with_prerequisite("intro")
            .available_in(["asia".into()])
            .with_window(Some(0), Some(10_000))
            .with_repeat(TaskRepeat::daily()),
    )
    .unwrap();

    let value = serde_json::to_value(game.task_view(&alice.id, &"daily".into()).unwrap()).unwrap();
    for field in [
        "id",
        "name",
        "description",
        "category",
        "status",
        "objectives",
        "reward",
        "accepted_at_ms",
        "expires_at_ms",
        "completed_at_ms",
        "claimed_at_ms",
        "next_available_at_ms",
        "repeat",
        "min_level",
        "requires",
        "regions",
        "starts_at_ms",
        "ends_at_ms",
        "hidden",
    ] {
        assert!(value.get(field).is_some(), "missing TaskView.{field}");
    }
    assert_eq!(value["status"], "locked");
    assert_eq!(value["requires"], serde_json::json!(["intro"]));
    assert!(value["accepted_at_ms"].is_null());
}

#[test]
fn skin_category_encodes_as_string_and_map_key() {
    assert_eq!(
        serde_json::to_string(&SkinCategory::Frame).unwrap(),
        "\"frame\""
    );
    assert_eq!(
        serde_json::to_string(&SkinCategory::Other("mood".into())).unwrap(),
        "\"other:mood\""
    );
    let equipped = BTreeMap::from([(SkinCategory::Frame, SkinId::from("gold_frame"))]);
    let value = serde_json::to_value(&equipped).unwrap();
    assert_eq!(value["frame"], "gold_frame");
    let decoded: BTreeMap<SkinCategory, SkinId> = serde_json::from_value(value).unwrap();
    assert_eq!(decoded[&SkinCategory::Frame], SkinId::from("gold_frame"));
}

#[test]
fn rewards_and_costs_use_documented_bundles() {
    let reward = Reward::new()
        .currency("gold", 100)
        .resource("energy", 10)
        .item("potion", 2)
        .skin("gold_frame")
        .xp(50);
    assert_eq!(
        serde_json::to_value(&reward).unwrap(),
        serde_json::json!({
            "currencies": [{"currency": "gold", "amount": 100}],
            "resources": [{"resource": "energy", "amount": 10}],
            "items": [{"item": "potion", "quantity": 2}],
            "skins": ["gold_frame"],
            "xp": 50
        })
    );

    let cost = Cost::new().currency("gold", 20).resource("energy", 1);
    assert_eq!(
        serde_json::to_value(&cost).unwrap(),
        serde_json::json!({
            "currencies": [{"currency": "gold", "amount": 20}],
            "resources": [{"resource": "energy", "amount": 1}]
        })
    );
}

#[test]
fn collectibles_use_adjacent_tags() {
    assert_eq!(
        serde_json::to_value(Collectible::Item("potion".into())).unwrap(),
        serde_json::json!({"kind": "item", "id": "potion"})
    );
    assert_eq!(
        serde_json::to_value(Collectible::Skin("gold_frame".into())).unwrap(),
        serde_json::json!({"kind": "skin", "id": "gold_frame"})
    );
    assert_eq!(
        serde_json::to_value(Collectible::Achievement("first_blood".into())).unwrap(),
        serde_json::json!({"kind": "achievement", "id": "first_blood"})
    );
}

#[test]
fn npc_decision_json_roundtrips() {
    let json = serde_json::json!({
        "reply": "hi",
        "actions": [
            {"type": "say", "text": "ok"},
            {
                "type": "give_reward",
                "account": "alice",
                "reward": {
                    "currencies": [{"currency": "gold", "amount": 5}],
                    "resources": [],
                    "items": [],
                    "skins": [],
                    "xp": 0
                }
            },
            {"type": "set_score", "leaderboard": "weekly", "account": "alice", "score": 100}
        ]
    });
    let decision: NpcDecision = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(decision.reply.as_deref(), Some("hi"));
    assert_eq!(decision.actions.len(), 3);
    assert!(matches!(decision.actions[0], NpcAction::Say { .. }));
    assert!(matches!(decision.actions[1], NpcAction::GiveReward { .. }));
    assert!(matches!(decision.actions[2], NpcAction::SetScore { .. }));
    assert_eq!(serde_json::to_value(&decision).unwrap(), json);
}

#[test]
fn game_state_has_version_and_documented_top_level_keys() {
    let value = serde_json::to_value(Game::new().snapshot()).unwrap();
    assert_eq!(
        value["version"].as_u64(),
        Some(u64::from(GAME_STATE_VERSION))
    );
    for field in [
        "version",
        "accounts",
        "regions",
        "rooms",
        "friends",
        "chat",
        "items",
        "resources",
        "wallets",
        "inventories",
        "shops",
        "trades",
        "skins",
        "cosmetics",
        "collections",
        "leaderboards",
        "achievements",
        "tasks",
        "announcements",
        "npcs",
    ] {
        assert!(value.get(field).is_some(), "missing GameState.{field}");
    }
}

#[test]
fn other_documented_enums_are_snake_case() {
    assert_eq!(
        serde_json::to_value(RegionStatus::Closed).unwrap(),
        "closed"
    );
    assert_eq!(
        serde_json::to_value(TradeState::Expired).unwrap(),
        "expired"
    );
    assert_eq!(
        serde_json::to_value(AnnouncementKind::Maintenance).unwrap(),
        "maintenance"
    );
    assert_eq!(
        serde_json::to_value(ScoreOrder::LowerIsBetter).unwrap(),
        "lower_is_better"
    );
    assert_eq!(
        serde_json::to_value(Rarity::Legendary).unwrap(),
        "legendary"
    );
}
