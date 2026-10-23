# Cookbook

以下完整示例位于 taixu 仓库的 `examples/`，**可选**；下面各节的使用方式不依赖阅读上游源码。

- [basic.rs](https://github.com/paxoscn/taixu/blob/main/examples/basic.rs)：世界、经济、房间、交易、任务、NPC、快照
- [tasks.rs](https://github.com/paxoscn/taixu/blob/main/examples/tasks.rs)：四类目标、冷却与新周期
- [trade_rollback.rs](https://github.com/paxoscn/taixu/blob/main/examples/trade_rollback.rs)：结算失败回滚
- [npc_actions.rs](https://github.com/paxoscn/taixu/blob/main/examples/npc_actions.rs)：自定义 Agent 与原子动作
- [persistence.rs](https://github.com/paxoscn/taixu/blob/main/examples/persistence.rs)：版本快照、`GameStore`、恢复
- [region_matrix.rs](https://github.com/paxoscn/taixu/blob/main/examples/region_matrix.rs)：Maintenance / Closed 行为

## 启动世界

```rust
use std::collections::BTreeMap;
use taixu::prelude::*;

let game = Game::new();
game.create_region("asia", "Asia", Some(5_000), Some("wss://asia".into()))?;

game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
game.register_resource(
    ResourceDef::new("energy", "Energy").with_cap(100).with_regen_per_hour(20),
)?;

let alice = game.ensure_account("alice", "Alice", "asia")?;
game.login(&alice.id)?;
```

## 购买与背包

```rust
let mut shop = Shop::new("general", "General Store");
shop.add_offer(
    ShopOffer::new(
        "potion_pack",
        "Potion pack",
        Cost::new().currency("gold", 100),
        Reward::new().item("potion", 4).skin("gold_frame").xp(10),
    )
    .with_per_account_limit(5)
    .with_stock(1_000),
)?;
game.register_shop(shop)?;

let receipt = game.purchase(&alice.id, &"general".into(), &"potion_pack".into(), 1)?;
println!("paid {:?}, got {:?}", receipt.paid, receipt.received);
```

失败（余额不足、背包满、库存/限购、区域关闭）时，钱包与背包都不会改变。

## 交易与回滚

```rust
let bob = game.create_account("bob", "Bob", "asia")?;
let trade = game.create_trade(&alice.id, &bob.id)?;

game.set_trade_side(
    &alice.id,
    &trade.id,
    TradeSide::new().with_items(BTreeMap::from([("potion".into(), 2)])),
)?;
game.set_trade_side(
    &bob.id,
    &trade.id,
    TradeSide::new().with_currencies(BTreeMap::from([("gold".into(), 50)])),
)?;

game.confirm_trade(&alice.id, &trade.id)?;
// 第二次确认触发结算；失败会保持 Open 并清空双方确认。
let completed = game.confirm_trade(&bob.id, &trade.id)?;
assert_eq!(completed.state, TradeState::Completed);
```

## 任务四类目标

```rust
game.register_task(
    TaskDef::new("collector", "Collector")
        .with_objective(TaskObjective::metric("kills", 3))
        .with_objective(TaskObjective::item("potion", 2))
        .with_objective(TaskObjective::skin("gold_frame"))
        .with_objective(TaskObjective::achievement("first_blood"))
        .with_reward(Reward::new().currency("gold", 100))
        .with_repeat(TaskRepeat::daily()),
)?;
game.accept_task(&alice.id, &"collector".into())?;

let outcome = game.record_event(&alice.id, "kills", 3)?;
// 道具/皮肤/成就目标在资产变化或查询/领奖时重新计算。
if game.task_view(&alice.id, &"collector".into())?.status == TaskStatus::Completed {
    game.claim_task(&alice.id, &"collector".into())?;
}
```

冷却结束后 `TaskView` 自动进入新周期；如果当前条件不满足，状态是 `Locked`，可用 `requires` / `regions` / `starts_at_ms` / `ends_at_ms` / `min_level` 解释原因。

## 频道列表

```rust
for summary in game.channels_for(&alice.id)? {
    println!(
        "{:?} messages={} last={:?}",
        summary.channel, summary.message_count, summary.last_message_at_ms
    );
}
```

只返回有历史且当前可读的频道（Global、本区域、所在房间、未拉黑私聊），按最新消息倒序。

## NPC 动作

```rust
struct GiftAgent;
#[async_trait::async_trait]
impl NpcAgent for GiftAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        Ok(NpcDecision {
            reply: Some("For you.".into()),
            actions: vec![NpcAction::GiveReward {
                account: ctx.speaker.clone(),
                reward: Reward::new().currency("gold", 5),
            }],
        })
    }
}

game.register_npc(NpcDef::new("guide", "Guide", "helpful").in_region("asia"))?;
let mut runtime = NpcRuntime::new();
runtime.register("guide", std::sync::Arc::new(GiftAgent));

let turn = runtime.converse(&game, &"guide".into(), &alice.id, "hello").await?;
println!("{} actions applied", turn.applied_actions);
```

Agent 返回的任何非法动作都会让整轮失败：无奖励、无分数、无聊天。

## 快照与恢复

```rust
let state = game.snapshot();
let json = serde_json::to_string(&state)?;
let decoded: GameState = serde_json::from_str(&json)?;
let restored = Game::from_state(decoded, game.config().clone(), std::sync::Arc::new(SystemClock));
```

迁移与 `GameStore` 见 [persistence.md](persistence.md)。

## 维护与关闭

```rust
game.set_region_status(&"asia".into(), RegionStatus::Maintenance)?;
// 已有玩家仍可建房/聊天/交易；新账号归属和跨区迁入被拒绝。

game.set_region_status(&"asia".into(), RegionStatus::Closed)?;
// 额外拒绝房间、聊天、购买、交易、好友申请、接任务、NPC 回合。
// 只读、领奖、资料/外观编辑仍可用。
```
