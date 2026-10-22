# Tai Xu（太虚）

Rust 通用游戏后端**库**：提供账号、区域、房间、社交、经济、养成、排行榜、公告与 NPC Agent 等可组合的游戏服务端领域能力。

Tai Xu 不包含网络层和鉴权：由你的网关完成登录/鉴权，然后把可信的 `AccountId` 传入 API；库负责游戏侧状态、规则与原子性，状态可序列化后交给任意数据库/对象存储持久化。

```toml
[dependencies]
taixu = "0.1"
```

## 功能

| 模块 | 能力 |
| --- | --- |
| 账号 `account` | 游戏侧档案、等级/经验、在线状态、元数据；**鉴权由外部提供** |
| 区域 `region` | 服务器节点注册、容量、`Open/Maintenance/Closed` 状态、账号归属与迁移 |
| 房间 `room` | 创建/加入/离开/关闭、容量、房主转移、踢人、元数据 |
| 交友 `friend` | 好友申请（接受/拒绝/取消）、关系、拉黑（双向阻断消息与申请） |
| 聊天 `chat` | 全局/区域/房间/私聊频道、有界历史、消息删除、系统与 NPC 消息 |
| 道具 `item` | 道具定义、稀有度、堆叠上限、唯一道具、槽位背包 |
| 背包 `inventory` | 固定槽位、自动叠堆、批量增删的**全有或全无**语义、容量调整 |
| 金钱与资源 `wallet` | 多货币；带上限与按小时被动恢复的资源（惰性结算） |
| 商店 `shop` | 多商店/商品、货币+资源定价、库存、限购、时间窗、区域限制 |
| 交易 `trade` | 双方挂单、确认成交、到期、原子结算与失败回滚 |
| 头像 `Avatar` | 图片/头衔/头像框/徽章，校验皮肤归属与分类 |
| 皮肤 `cosmetics` | 皮肤目录、拥有关系、按分类装备 |
| 收集 `collection` | 图鉴条目（道具/皮肤/成就）、里程碑与一次性领奖 |
| 排名 `leaderboard` | 高/低分榜、并列名次、Top-N、赛季重置 |
| 成就 `achievement` | 指标计数、自动解锁、一次性领奖、隐藏成就 |
| 任务 `task` | 接取/放弃/领奖、指标/道具/皮肤/成就目标、前置、等级/区域/时间窗、限时、每日/每周/冷却重复 |
| 公告 `announcement` | 时间窗、区域定向、优先级、已读/未读 |
| NPC Agent `npc` | 异步 Agent 接口、上下文/提示词、结构化动作、规则型实现 |

## 快速开始

```rust
use std::collections::BTreeMap;
use taixu::prelude::*;

fn main() -> taixu::Result<()> {
    let game = Game::new();

    // 区域（服务器节点）与账号。账号由外部鉴权系统提供。
    game.create_region("asia", "Asia", Some(5_000), Some("wss://asia.example.com".into()))?;
    let alice = game.create_account("alice", "Alice", "asia")?;
    let bob = game.create_account("bob", "Bob", "asia")?;

    // 道具、背包、货币与资源。
    game.register_item(ItemDef::new("potion", "Potion").with_stack_limit(20))?;
    game.register_resource(
        ResourceDef::new("energy", "Energy")
            .with_cap(100)
            .with_regen_per_hour(20),
    )?;
    game.add_item(&alice.id, &"potion".into(), 5)?;
    game.add_currency(&alice.id, &"gold".into(), 500)?;
    game.set_resource(&alice.id, &"energy".into(), 80)?;

    // 商店：用 100 金币换 2 瓶药水。
    let mut shop = Shop::new("general", "General Store");
    shop.add_offer(
        ShopOffer::new(
            "potion_pack",
            "Potion pack",
            Cost::new().currency("gold", 100),
            Reward::new().item("potion", 2),
        )
        .with_per_account_limit(5),
    )?;
    game.register_shop(shop)?;
    game.purchase(&alice.id, &"general".into(), &"potion_pack".into(), 1)?;

    // 房间与聊天。
    let room = game.create_room(&alice.id, "Hangout", &"asia".into(), 8, BTreeMap::new())?;
    game.join_room(&bob.id, &room.id)?;
    game.send_message(&alice.id, ChatChannel::Room(room.id.clone()), "hello!")?;

    // 好友与私聊。
    game.send_friend_request(&alice.id, &bob.id, Some("hi Bob"))?;
    game.accept_friend_request(&bob.id, &alice.id)?;
    game.send_message(
        &alice.id,
        ChatChannel::private(alice.id.clone(), bob.id.clone()),
        "wanna trade?",
    )?;

    // 排行榜与成就。
    game.register_leaderboard(
        Leaderboard::new("weekly", "Weekly score", ScoreOrder::HigherIsBetter)
            .with_max_entries(100),
    )?;
    game.set_leaderboard_score(&"weekly".into(), &alice.id, 9_800)?;

    game.register_achievement(AchievementDef::new(
        "first_blood",
        "First blood",
        "kills",
        1,
        Reward::new().currency("gold", 50),
    ))?;
    game.record_metric(&alice.id, "kills", 1)?;
    game.claim_achievement(&alice.id, &"first_blood".into())?;

    // 任务：接取 → 通过游戏事件推进 → 领奖。
    game.register_task(
        TaskDef::new("first_steps", "First Steps")
            .with_objective(TaskObjective::metric("kills", 3))
            .with_reward(Reward::new().currency("gold", 50)),
    )?;
    game.accept_task(&alice.id, &"first_steps".into())?;
    let outcome = game.record_event(&alice.id, "kills", 3)?;
    assert_eq!(outcome.tasks.len(), 1);
    game.claim_task(&alice.id, &"first_steps".into())?;

    // 公告。
    game.publish_announcement(
        AnnouncementDraft::new("Server event", "Double XP this weekend!")
            .with_kind(AnnouncementKind::Event)
            .with_priority(10),
    )?;

    println!(
        "alice gold = {}",
        game.currency_balance(&alice.id, &"gold".into())?
    );

    // 快照可序列化为 JSON / MessagePack 等，交给外部存储。
    let _state: GameState = game.snapshot();
    Ok(())
}
```

更多完整示例见 [`examples/basic.rs`](examples/basic.rs)：`cargo run --example basic`。

## 设计

### 身份与鉴权

`AccountId` 由外部系统签发且可信。Tai Xu 不做 token/session 校验，但每个操作都会校验账号存在性和资源归属，例如只有皮肤拥有者能装备、只有房主能关闭房间、只有交易参与方能确认。

### 原子性

所有跨模块操作都在 `Game` 的一把写锁内完成，并在修改前克隆相关账户资产（钱包、背包、皮肤）作为暂存区：

- 商店购买：校验 → 扣费 → 发货 → 更新库存/限购；任一步失败则全部不落盘。
- 交易成交：双方移出挂单资产 → 互相放入 → 双方同时提交；背包不足或资产已消失时交易保持未完成并重置确认。
- 成就/收集领奖：先发货成功，再标记已领取。
- 跨区域迁移：校验目标区域容量 → 切换归属 → 自动退出房间。

### 持久化

`GameState` 实现了 `serde::{Serialize, Deserialize}`，推荐方式：

```rust
let json = serde_json::to_string(&game.snapshot())?;
```

也可以通过 `GameStore` 接入数据库/对象存储：

```rust
use taixu::store::{GameStore, MemoryStore};

let store = MemoryStore::new();
game.save_to(&store)?;
let restored = taixu::Game::load_from(&store, game.config().clone())?
    .expect("snapshot exists");
```

实现 `GameStore` 即可替换为 Postgres / Redis / S3 等。建议在网关层周期性做全量快照，或按业务事件做增量持久化。

### 时间

所有时间戳是 Unix 毫秒（`i64`）。生产环境使用 `SystemClock`；测试/回放可注入 `ManualClock`：

```rust
use std::sync::Arc;
use taixu::prelude::*;

let clock = Arc::new(ManualClock::new(1_700_000_000_000));
let game = Game::with_clock(clock.clone());
clock.advance(60_000); // 推进一分钟
```

### 资源配置

`GameConfig` 控制背包默认容量、房间上限、聊天长度/保留条数、交易有效期、NPC 上下文条数等：

```rust
let game = Game::with_config(GameConfig {
    default_inventory_capacity: 80,
    trade_expiry_secs: 3_600,
    ..GameConfig::default()
});
```

## 任务系统

任务支持四种目标，并可自由组合：

- `TaskObjective::metric("kills", 3)`：由游戏事件计数推进；
- `TaskObjective::item("potion", 3)`：检查实时背包数量；
- `TaskObjective::skin("gold_frame")`：检查皮肤拥有；
- `TaskObjective::achievement("first_blood")`：检查成就完成。

```rust
game.register_task(
    TaskDef::new("daily_bounty", "Daily bounty")
        .with_category("daily")
        .with_objective(TaskObjective::metric("bounty", 3))
        .with_objective(TaskObjective::item("trophy", 1))
        .with_reward(Reward::new().currency("gold", 200))
        .with_min_level(5)
        .with_repeat(TaskRepeat::daily())
        .with_time_limit_ms(6 * 3_600_000),
)?;

game.accept_task(&alice.id, &"daily_bounty".into())?;

// 同一事件同时推进成就与所有进行中的任务。
let outcome = game.record_event(&alice.id, "bounty", 1)?;
println!("achievements={:?} tasks={:?}", outcome.achievements, outcome.tasks);

// 目标满足后可随时领奖（道具/皮肤/成就目标会在查询或领奖时重新计算）。
game.claim_task(&alice.id, &"daily_bounty".into())?;
```

- `available_tasks` / `active_tasks` / `completed_tasks` / `task_views` 提供不同视图；
- 任务可设置前置任务、最低等级、区域与开放时间窗；
- 限时任务过期后状态为 `Expired`，可重新接取；若在期限内完成，过期后仍可领奖；
- `TaskRepeat::daily()` / `weekly()` / `cooldown(ms)` 在领奖后开始冷却，接取新一轮会重置计数；
- `abandon_task` 放弃进行中的任务，`record_event` 返回本次完成的成就与任务 ID。

## NPC Agent

NPC 定义保存在 `GameState` 中；Agent 运行时（可能持有 LLM 客户端）独立于可序列化状态。

```rust
use std::sync::Arc;
use async_trait::async_trait;
use taixu::npc::{NpcAgent, NpcContext, NpcDecision, NpcError};
use taixu::prelude::*;

struct MyLlmAgent {
    // http client, model config ...
}

#[async_trait]
impl NpcAgent for MyLlmAgent {
    async fn respond(&self, ctx: &NpcContext) -> Result<NpcDecision, NpcError> {
        // ctx.system_prompt() 已包含 NPC 人设、频道、附近玩家和历史消息。
        // 将模型的结构化输出反序列化为 NpcDecision 即可。
        let _prompt = ctx.system_prompt();
        Ok(NpcDecision::reply("Welcome, traveler!"))
    }
}

let game = Game::new();
game.create_region("asia", "Asia", None, None)?;
let account = game.create_account("alice", "Alice", "asia")?;
game.register_npc(
    NpcDef::new("guide", "Guide", "A friendly town guide.")
        .in_region("asia"),
)?;

let mut runtime = NpcRuntime::new();
runtime.register("guide", Arc::new(MyLlmAgent { /* ... */ }));

// 异步执行一轮对话（宿主自行选择 tokio / async-std 等运行时）：
// let turn = runtime
//     .converse(&game, &"guide".into(), &account.id, "Where is the shop?")
//     .await?;
```

Agent 可以返回结构化动作，由运行时校验并原子执行：

- `Say { text }`：在当前频道以 NPC 身份发言；
- `GiveReward { account, reward }`：向说话者或附近玩家发放奖励；
- `SetScore { leaderboard, account, score }`：写入排行榜。

运行时只允许 Agent 操作上下文中的说话者或附近玩家；Agent 错误时不会写入任何聊天消息。

`NpcContext::system_prompt()` 同时给出了 JSON 输出结构，便于直接接入工具调用/结构化输出模型；`NpcDecision`、`NpcAction` 都实现了 `Serialize/Deserialize`。

## 目录

```
src/
  game.rs            集中式 Game 门面、原子经济操作、GameState
  account.rs         账号档案
  region.rs          区域/节点
  room.rs            房间
  friend.rs          好友与拉黑
  chat.rs            聊天频道与历史
  item.rs            道具与槽位背包
  wallet.rs          货币与恢复型资源
  shop.rs            商店与商品
  trade.rs           玩家交易
  cosmetics.rs       头像与皮肤
  collection.rs      图鉴收集
  leaderboard.rs     排行榜
  achievement.rs     成就
  task.rs            任务/任务进度
  announcement.rs    公告
  npc.rs             NPC Agent 运行时
  store.rs           快照持久化接口
tests/backend.rs     端到端测试
examples/basic.rs    可运行示例
```

## 测试

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## 许可证

MIT OR Apache-2.0
