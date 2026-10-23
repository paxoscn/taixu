# JSON / serde 契约

本页描述 `serde_json` 下客户端可见类型的线格式。除非特别说明：

- 时间戳为 Unix 毫秒（`i64`）。
- `Option<T>` 字段序列化为 `null`，不会省略键。
- 所有 ID 都是透明字符串 newtype（`AccountId`、`TaskId`、`ShopId`…）。
- 客户端不应依赖结构体字段顺序。

契约由库内的 round-trip 测试守护；使用方按本页构造/解析 JSON 即可。

## 频道与消息

### `ChatChannel`

`ChatChannel` 序列化为一个字符串（因此也可作为 JSON map key）：

| 频道 | 格式 | 示例 |
| --- | --- | --- |
| 全局 | `global` | `"global"` |
| 区域 | `region:<region_id>` | `"region:asia"` |
| 房间 | `room:<room_id>` | `"room:room_ab12"` |
| 私聊 | `dm:<a的UTF-8字节长度>:<a><b>` | `"dm:5:alicebob"` |

私聊频道会按 `AccountId` 字典序归一化，因此 `private(alice, bob)` 与 `private(bob, alice)` 是同一个频道。前缀长度是**字节数**，用来支持 ID 中包含 `:` 等字符；客户端若自行构造，按 UTF-8 字节长度计算。

### `ChatAuthor`

```json
{ "kind": "account", "account": "alice" }
{ "kind": "npc", "npc": "barkeep" }
{ "kind": "system" }
```

### `ChatMessage`

```json
{
  "id": "msg_...",
  "channel": "dm:5:alicebob",
  "author": { "kind": "account", "account": "alice" },
  "body": "hello",
  "sent_at_ms": 1700000000000,
  "metadata": { "client_id": "abc" }
}
```

### `ChannelSummary`

`Game::channels_for(account)` 返回，按 `last_message_at_ms` 降序：

```json
{
  "channel": "room:room_ab12",
  "last_message_at_ms": 1700000000000,
  "message_count": 42
}
```

只包含当前有历史日志的频道；频道在第一条消息时惰性创建。

## 任务

### `TaskObjective`

内部 tag 为 `type`：

```json
{ "type": "metric", "metric": "kills", "target": 3 }
{ "type": "item", "item": "potion", "quantity": 2 }
{ "type": "skin", "skin": "gold_frame" }
{ "type": "achievement", "achievement": "first_blood" }
```

### `TaskRepeat`

内部 tag 为 `kind`：

```json
{ "kind": "once" }
{ "kind": "cooldown", "duration_ms": 86400000 }
```

`daily()` = 86,400,000 ms，`weekly()` = 604,800,000 ms；从**领奖时间**开始计算。

### `TaskStatus` / `TaskView`

状态是 snake_case 字符串：`"available" | "locked" | "active" | "completed" | "claimed" | "expired"`。

```json
{
  "id": "daily_bounty",
  "name": "Daily bounty",
  "description": "",
  "category": "daily",
  "status": "available",
  "objectives": [
    {
      "objective": { "type": "metric", "metric": "bounty", "target": 3 },
      "progress": 0,
      "target": 3,
      "complete": false
    }
  ],
  "reward": {
    "currencies": [{ "currency": "gold", "amount": 200 }],
    "resources": [],
    "items": [],
    "skins": [],
    "xp": 0
  },
  "accepted_at_ms": null,
  "expires_at_ms": null,
  "completed_at_ms": null,
  "claimed_at_ms": null,
  "next_available_at_ms": null,
  "repeat": { "kind": "cooldown", "duration_ms": 86400000 },
  "min_level": 5,
  "requires": ["intro"],
  "regions": ["asia"],
  "starts_at_ms": null,
  "ends_at_ms": null,
  "hidden": false
}
```

任务进入新周期（重复任务冷却结束）时，指标进度与 `accepted/completed/claimed/next_available` 都会归零；道具/皮肤/成就目标按当前实时状态展示。若冷却结束但等级/区域/前置/时间窗不满足，`status` 为 `"locked"`。

## 奖励与成本

```json
{
  "currencies": [{ "currency": "gold", "amount": 100 }],
  "resources": [{ "resource": "energy", "amount": 10 }],
  "items": [{ "item": "potion", "quantity": 2 }],
  "skins": ["gold_frame"],
  "xp": 50
}
```

`Cost` 只有 `currencies` 与 `resources`。皮肤是唯一资产：`Reward::scaled(qty)` 不会把皮肤重复授予多份。

## 皮肤分类

`SkinCategory` 序列化为字符串：

```text
character | weapon | avatar | frame | badge | background | emote | other:<name>
```

`other:` 后的名称原样保存。分类也作为 `equipped` map 的 key：

```json
{
  "owned": ["gold_frame"],
  "equipped": { "frame": "gold_frame" }
}
```

## 收集

`Collectible` 使用相邻 tag/content：

```json
{ "kind": "item", "id": "potion" }
{ "kind": "skin", "id": "gold_frame" }
{ "kind": "achievement", "id": "first_blood" }
```

## NPC

### `NpcDecision`

```json
{
  "reply": "Welcome, traveler!",
  "actions": [
    { "type": "say", "text": "The shop is to the north." },
    {
      "type": "give_reward",
      "account": "alice",
      "reward": {
        "currencies": [{ "currency": "gold", "amount": 10 }],
        "resources": [],
        "items": [],
        "skins": [],
        "xp": 0
      }
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

- `reply` 为 `null` 时只执行 actions。
- `give_reward` / `set_score` 的 `account` 必须是说话者或 NPC 上下文中的附近玩家。
- 动作顺序不影响原子性：任何动作失败，整轮不提交。

`NpcDef` 示例：

```json
{
  "id": "barkeep",
  "name": "Barkeep",
  "persona": "A cheerful tavern keeper.",
  "region": "asia",
  "room": null,
  "enabled": true,
  "greeting": "Welcome!",
  "metadata": {}
}
```

## 快照 `GameState`

顶层包含 `version` 与全部模块状态：

```json
{
  "version": 1,
  "accounts": {},
  "regions": {},
  "rooms": {},
  "friends": {},
  "chat": {},
  "items": {},
  "resources": {},
  "wallets": {},
  "inventories": {},
  "shops": {},
  "trades": {},
  "skins": {},
  "cosmetics": {},
  "collections": {},
  "leaderboards": {},
  "achievements": {},
  "tasks": {},
  "announcements": {},
  "npcs": {}
}
```

缺少 `version` 的旧快照会按当前版本反序列化；结构不兼容时应由宿主迁移，见 [persistence.md](persistence.md)。

## 其他枚举

| 类型 | 值 |
| --- | --- |
| `RegionStatus` | `"open"`, `"maintenance"`, `"closed"` |
| `TradeState` | `"open"`, `"completed"`, `"cancelled"`, `"expired"` |
| `AnnouncementKind` | `"news"`, `"event"`, `"maintenance"`, `"promotion"`, `"system"` |
| `ScoreOrder` | `"higher_is_better"`, `"lower_is_better"` |
| `Rarity` | `"common"`, `"uncommon"`, `"rare"`, `"epic"`, `"legendary"`, `"mythic"` |

## 错误不是 wire type

`GameError` 没有实现 `Serialize`。宿主应把它映射成自己的协议错误码，例如：

```rust
match error {
    GameError::InsufficientFunds { .. } => "INSUFFICIENT_FUNDS",
    GameError::RegionNotOpen(_) => "REGION_CLOSED",
    GameError::TaskLocked(_) => "TASK_LOCKED",
    _ => "GAME_ERROR",
}
```

完整列表见 [errors.md](errors.md)。
