# `GameConfig` 参考

`GameConfig` 不随 `GameState` 持久化。恢复快照时必须提供与预期一致的配置。

```rust
use taixu::prelude::*;

let game = Game::with_config(GameConfig {
    default_inventory_capacity: 80,
    trade_expiry_secs: 3_600,
    ..GameConfig::default()
});
```

| 字段 | 默认值 | 作用 |
| --- | ---: | --- |
| `default_inventory_capacity` | 50 | 新账号默认背包槽位；也用于缺失背包的兜底 |
| `max_inventory_capacity` | 500 | `set_inventory_capacity` 允许的最大槽位 |
| `max_room_capacity` | 200 | 新建房间容量上限 |
| `max_name_len` | 32 | 账号/房间名最大字符数 |
| `max_chat_message_len` | 2000 | 聊天与 NPC 消息最大字符数 |
| `chat_history_per_channel` | 100 | 构造时每个聊天频道的保留消息数 |
| `trade_expiry_secs` | 86400 | 新交易有效期（秒） |
| `npc_context_messages` | 20 | 注入 NPC 上下文的最近消息条数 |
| `npc_nearby_limit` | 50 | NPC 上下文/区域附近玩家上限 |
| `max_friend_request_len` | 200 | 好友申请附言最大字符数 |
| `max_avatar_title_len` | 48 | 头像头衔最大字符数 |
| `max_avatar_image_len` | 2048 | 头像图片标识/URL 最大字符数 |

## 注意事项

- **聊天容量在构造时固化**：`ChatService::new(chat_history_per_channel)` 决定新频道的日志上限；已有快照频道保留自己的容量。修改配置不会重写历史。
- **交易有效期为秒**：内部转换为毫秒；`expire_trades()` 由宿主周期调用，交易变更也会惰性过期检查。
- **NPC 上限只影响上下文**：不限制 Agent 实际返回的动作数量，宿主应自行限制 token/动作/奖励额度。
- **背包最大值**：`set_inventory_capacity` 超过该值返回 `InvalidInput`；值为 0 也非法。
- **名称/内容按 Unicode 字符数**：不是字节数。
- **时钟不在此配置**：通过 `Game::with_clock` / `with_config_and_clock` 注入。
- **货币/道具目录不在配置中**：通过注册 API 提供。
