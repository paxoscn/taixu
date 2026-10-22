# NPC Agent 集成契约

NPC 定义是 `GameState` 的一部分；Agent 实例不进入快照，由宿主在启动时重新注册。

## 类型

`NpcDef`：

```rust
NpcDef::new("barkeep", "Barkeep", "A cheerful tavern keeper.")
    .in_room(room_id)        // 房间 NPC；优先于 region
    .with_greeting("Welcome!")
```

| 字段 | 说明 |
| --- | --- |
| `id` | NPC ID |
| `name` | 展示名 |
| `persona` | 人设/行为提示 |
| `region` | 区域绑定；与 `room` 同时设置时必须匹配房间区域 |
| `room` | 房间绑定；设置后频道为 `Room` |
| `enabled` | 停用后不能对话或发言 |
| `greeting` | 宿主可用作开场白 |
| `metadata` | 自定义配置 |

绑定规则：

- `room = Some` → 频道 `Room(id)`，说话者必须是房间成员；
- 否则 `region = Some` → 频道 `Region(id)`，说话者必须属于该区域；
- 否则 → `Global`，任何账号都可对话。

## Agent 接口

```rust
use async_trait::async_trait;
use taixu::npc::{NpcAgent, NpcContext, NpcDecision, NpcError};

struct MyAgent;

#[async_trait]
impl NpcAgent for MyAgent {
    async fn respond(&self, context: &NpcContext) -> Result<NpcDecision, NpcError> {
        // 可将 context.system_prompt() 发送给模型；
        // 把模型的结构化输出反序列化为 NpcDecision。
        Ok(NpcDecision {
            reply: Some("Welcome, traveler!".into()),
            actions: vec![],
        })
    }
}
```

`NpcContext` 包含：

- `npc`：完整 `NpcDef`；
- `channel`、`speaker`、`speaker_name`、`speaker_level`；
- `message`：玩家消息（已 trim、校验长度）；
- `recent_messages`：频道最近消息，数量上限 `GameConfig.npc_context_messages`；
- `nearby_players`：房间成员或区域玩家，上限 `npc_nearby_limit`；
- `now_ms`。

`NpcContext::system_prompt()` 生成 provider 无关的文本提示，包含人设、频道、玩家、附近玩家、最近对话和 JSON 输出要求。它**不**调用任何模型；宿主自行决定供应商与 token 预算。

`NpcContext::allowed_account(account)` 判断账号是否为说话者或附近玩家。

## 决策与动作

```json
{
  "reply": "Here is your reward.",
  "actions": [
    { "type": "say", "text": "The shop is north." },
    {
      "type": "give_reward",
      "account": "alice",
      "reward": { "currencies": [{ "currency": "gold", "amount": 10 }] }
    },
    {
      "type": "set_score",
      "leaderboard": "weekly",
      "account": "alice",
      "score": 100
    }
  ]
}
```

- `Say { text }`：在**当前 NPC 频道**以 NPC 身份发言；不能跨频道。
- `GiveReward { account, reward }`：奖励结构见 [json-contracts.md](json-contracts.md)。可包含货币/资源/道具/皮肤/XP。
- `SetScore { leaderboard, account, score }`：覆盖排行榜分数。

目标账号必须是说话者或 `nearby_players` 中的账号。运行时和 `Game::apply_npc_turn` 都会校验。

## 执行顺序与原子性

`NpcRuntime::converse`：

1. `Game::npc_context`：校验 NPC 启用、频道存在、说话者访问权限，读取历史；
2. `agent.respond(&context).await`：**不持锁**；
3. 校验 `Say` 文本非空、动作目标在允许集合；
4. 收集 `Say` 文本与 `reply`；
5. 调用 `Game::apply_npc_turn`：
   - 重新校验 NPC、频道、区域状态、消息长度、动作目标、奖励引用、排行榜；
   - 奖励应用到克隆资产；
   - 全部通过后一次性提交奖励、排行榜分数、玩家消息和 NPC 消息；
6. 任一动作失败：不写奖励、不写分数、不写聊天，不产生部分状态。

`NpcTurn` 返回：

- `channel`
- `player_message`：已写入的玩家消息
- `npc_messages`：本次写入的 NPC 消息（Say 顺序 + reply）
- `applied_actions`：成功提交的奖励/分数动作数量（`Say` 不计数）

Agent 错误（`NpcError::Agent`）不会写入聊天；`NoAgent` 表示未注册 Agent。

## 错误与安全建议

- Agent 输出不可信：只允许结构化动作，不要让 Agent 直接调用任意宿主 API。
- 奖励范围由上下文角色限制；如需更严格的经济控制，可在 Agent 包装层做额度/冷却。
- 限制单轮最大动作数量与奖励总额（运行时当前不设上限，宿主应自行限制）。
- LLM 超时/失败时返回 `NpcError::Agent`；不要在超时后重试已提交的回合。
- 隐藏 prompt 中的内部元数据；`system_prompt` 只包含公开上下文。

## 异步运行时

库不依赖 tokio/async-std；`NpcAgent` 通过 `async-trait` 返回 boxed future。宿主可用任意执行器：

```rust
let turn = futures::executor::block_on(runtime.converse(
    &game,
    &"barkeep".into(),
    &account.id,
    "hello",
))?;
```

生产环境应在共享异步运行时中调用，并给 Agent 设置超时。
