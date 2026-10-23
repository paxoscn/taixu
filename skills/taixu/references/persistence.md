# 持久化与迁移

`taixu` 的持久化单位是完整的 `GameState` 快照，它实现了 `serde::{Serialize, Deserialize}`。

## 快照内容

```rust
let state: GameState = game.snapshot();
let json = serde_json::to_string(&state)?;
```

快照包含：

- `version`：schema 版本，当前为 `GAME_STATE_VERSION`（`1`）。
- 账号、区域、房间、好友、聊天、目录、钱包、背包、商店、交易、皮肤、图鉴、排行榜、成就、任务、公告、NPC 定义。
- **不包含** `GameConfig`、注入的 `Clock`、`NpcRuntime` 中的 Agent 实例、宿主的外部鉴权数据。

因此恢复时必须提供相同的配置和时钟：

```rust
use std::sync::Arc;
use taixu::prelude::*;

let state: GameState = serde_json::from_str(&json)?;
let game = Game::from_state(
    state,
    GameConfig::default(),
    Arc::new(SystemClock),
);
```

`GameConfig.chat_history_per_channel` 在**构造时**决定聊天日志容量；已持久化的频道保留自己的日志，新建 Game 时请使用同一配置。`default_inventory_capacity` 用于新账号和缺失背包的兜底。

## 版本与迁移

- 新快照写入当前 `GAME_STATE_VERSION`。
- 缺失 `version` 的旧快照由 serde 默认函数读成当前版本。
- `version == 0` 在 `Game::from_state` 中归一化为当前版本。
- 快照版本与当前版本不一致、且结构不兼容时，宿主必须先迁移。

推荐迁移流程：

```rust
use serde_json::Value;
use taixu::prelude::*;

fn migrate(mut value: Value) -> taixu::Result<GameState> {
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(GAME_STATE_VERSION as u64) as u32;

    match version {
        GAME_STATE_VERSION => {}
        0 => {}
        other => {
            return Err(GameError::Persistence(format!(
                "unsupported snapshot version {other}"
            )));
        }
    }

    // 在结构变更后，在这里把旧字段转换为新结构。
    serde_json::from_value(value)
        .map_err(|error| GameError::Persistence(error.to_string()))
}
```

规则：**先迁移 JSON 结构，再反序列化为 `GameState`**；不要依赖 serde 对字段缺失的静默容错来改变语义。

## `GameStore`

`GameStore` 是同步 trait：

```rust
use taixu::store::{GameStore, StoreError};
use taixu::GameState;

struct MyStore { /* db client */ }

impl GameStore for MyStore {
    fn load(&self) -> Result<Option<GameState>, StoreError> {
        // SELECT payload FROM game_snapshot WHERE id = 1
        unimplemented!()
    }

    fn save(&self, state: &GameState) -> Result<(), StoreError> {
        // INSERT ... ON CONFLICT DO UPDATE
        unimplemented!()
    }
}
```

`Game::save_to(&store)` 会先 `snapshot()` 再调用 `save`；`Game::load_from(&store, config)` 读取快照并构造新 Game。

### 同步 trait 与异步存储

`GameStore` 不在库内引入运行时。接入 Postgres/Redis 等异步客户端时有三种做法：

1. 在宿主侧用独立线程/阻塞连接池实现 trait，并在请求路径之外执行；
2. 不实现 `GameStore`，直接用 `snapshot()` + 自己的异步持久化；
3. 每个事件记录状态变更日志（增量），周期性做全量快照。

不要在网络请求处理线程里同步等待慢数据库；全量快照本身也是 O(state size)。

## 保存频率与一致性

- 全量快照适合中小规模；大世界建议按模块/账号分片或使用增量事件。
- 快照是某一时刻的完整状态；保存时应连同 `version` 一起写入。
- 若使用 WAL/事件流，必须能重建出相同 `GameState`，否则恢复时会出现资产不一致。
- 保存失败时应保留上一份成功快照，并告警；不要在失败后丢弃内存状态。

## 快照增长与清理

当前库没有自动保留策略，以下集合会持续增长：

- 商店购买记录（用于限购统计），按 `(shop, offer, account)` 累计；
- 已完成/取消/过期交易；
- 每个聊天频道的历史（每频道有上限，但频道数量无界）；
- 公告已读集合；
- 排行榜/图鉴/任务进度按账号存储。

宿主应定期归档或重写快照，并自行决定保留周期。账号删除/匿名化尚未内置，属于需要产品与合规定义的独立能力。

## 锁与持久化

`snapshot()` 只持有读锁完成克隆。不要在持有 `read_state()` guard 时调用任何写锁方法（包括 `wallet`、`task_views`、`purchase`），否则同一线程会死锁。短查询用：

```rust
let account_count = game.with_state(|state| state.accounts.len());
```
