# 宿主集成指南

`taixu` 是同步、内存内、单进程的游戏领域库。宿主负责网络、鉴权、运行时调度和持久化。

## 宿主职责清单

| 职责 | 建议 |
| --- | --- |
| 鉴权 / 会话 | 在网关验证 token，把可信 `AccountId` 传给库；库不校验密码/token |
| 输入校验 | 库校验领域约束；长度、协议结构、限流由网关负责 |
| 在线状态 | 调用 `login` / `heartbeat` / `logout`；按 `last_seen_ms` 清理僵尸在线 |
| 交易过期 | 周期调用 `expire_trades()`（例如每分钟） |
| 公告/活动时间 | 库按注入时钟判断；确保时钟同步，测试用 `ManualClock` |
| 快照 | 周期性 `save_to` 或自行调用 `snapshot()`；保存失败要告警并保留旧快照 |
| 账号注销 | 当前未内置；需要业务定义删除/匿名化与保留策略 |
| NPC Agent | 在宿主运行时执行异步 LLM 调用；不要把网络请求放在锁内 |
| 临时组合流程 | 若需要跨多个库调用的原子事务，在宿主层做命令队列/补偿；库只保证自身的单步与内置复合操作 |
| 货币/道具配置 | 宿主提供展示名、图标、开放配置；库只保存 ID 与数值 |

## 单进程与横向扩展

`Game` 持有一把 `parking_lot::RwLock<GameState>`：

- 适合单进程、多线程；不支持把同一账号分片到多个进程后并发写。
- 横向扩展需要按账号/区域分片，或把状态迁移到外部存储并自行处理一致性。
- 所有操作在内存中完成；不要在写锁内做 I/O。

## 请求处理流程

```text
WebSocket/HTTP
   │  验证 token → AccountId
   ▼
game.ensure_account(id, name, region)     // 首次登录
game.login(id)
   │
   ├─ 只读：game.task_views / inventory / channels_for ...
   ├─ 战斗事件：game.record_event(id, "kills", 1)
   ├─ 经济：game.purchase / create_trade / confirm_trade
   └─ 聊天：game.send_message(...)
   │
   ▼
返回 JSON；GameError 映射为协议错误码
```

## 锁的使用

- `with_state(|state| ...)`：短只读查询，guard 不会逃逸。
- `read_state()`：仅适合极短的直接访问；**不要**在 guard 存活期间调用写锁方法。
- `wallet`、`task_views`、`purchase`、`record_event`、`add_item` 等都取写锁（有些因为惰性再生/任务刷新）。
- 不要在 `with_state` 闭包里调用 `game.xxx()` 写方法；闭包拿到的 state 与写锁属于同一把不可重入锁。

## 错误映射

`GameError` 不是 serde 类型，宿主应映射为自己稳定的错误码。示例：

```rust
use taixu::prelude::*;

fn error_code(error: &GameError) -> &'static str {
    match error {
        GameError::InsufficientFunds { .. } => "INSUFFICIENT_FUNDS",
        GameError::InventoryFull => "INVENTORY_FULL",
        GameError::RegionNotOpen(_) => "REGION_UNAVAILABLE",
        GameError::TaskLocked(_) => "TASK_LOCKED",
        GameError::TaskOnCooldown { .. } => "TASK_COOLDOWN",
        GameError::TradeExpired => "TRADE_EXPIRED",
        GameError::ChannelAccessDenied(_) => "CHANNEL_FORBIDDEN",
        _ => "GAME_ERROR",
    }
}
```

完整变体见 [errors.md](errors.md)。

## 维护 / 关闭区域

- `Open`：正常。
- `Maintenance`：禁止新账号归属和跨区迁入，已有玩家可继续房间/聊天/经济/任务。
- `Closed`：额外拒绝房间、玩家聊天、购买、交易、好友申请、接任务、NPC 回合；只读、领奖、资料/外观编辑仍可用。

运营切换前建议提前发布公告，并通过 `active_announcements` 展示。

## 任务与成就事件

推荐把所有玩法事件统一走：

```rust
let outcome = game.record_event(&account.id, "kills", 1)?;
```

它同时推进成就与任务。`record_metric` 保留兼容，只返回新完成成就 ID。

## NPC 集成

1. 用 `Game::register_npc` 注册 NPC（区域或房间绑定）。
2. 用 `NpcRuntime::register(id, Arc<dyn NpcAgent>)` 绑定 Agent。
3. 玩家消息到达时调用 `runtime.converse(...).await`。
4. Agent 只应返回 `NpcDecision`；奖励/分数动作必须指向上下文中允许的账号。
5. 运行时在 Agent 返回后调用 `Game::apply_npc_turn`，原子提交动作和聊天。

详见 [npc.md](npc.md)。

## 推荐上线检查

- [ ] 配置与快照版本一致，迁移脚本已演练
- [ ] 时钟同步，测试覆盖冷却/过期边界
- [ ] 周期任务：`expire_trades`、在线清理、快照
- [ ] 错误码映射与重试策略（不要自动重试扣费类失败）
- [ ] 限制单次聊天长度、房间数、Agent 并发与 token 预算
- [ ] 观测：快照大小、锁等待、NPC 延迟、失败率
