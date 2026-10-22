# 状态机与时间语义

所有计时都基于注入的 `Clock`，时间戳为 Unix 毫秒。测试与回放用 `ManualClock` 推进时间。

## 任务

### 状态

```text
                 accept                 objectives met
   Available ─────────────▶ Active ─────────────────▶ Completed
       ▲                      │                          │
       │                      │ abandon                  │ claim
       │                      ▼                          ▼
       │                   (removed)                  Claimed
       │                                                 │
       │             cooldown elapsed + eligible         │
       └─────────────────────────────────────────────────┘
```

- `Locked`：等级、区域、前置任务或开放时间窗不满足。
- `Expired`：限时任务在完成前超过 `expires_at_ms`。
- `Claimed`：已领奖，仍在重复冷却中（`next_available_at_ms` 有值）。
- 冷却结束后：
  - 条件仍满足 → `Available`；
  - 条件不满足 → `Locked`；
  - 视图进入新周期，指标进度与上一轮时间戳清零。
- 已领奖的 `TaskRepeat::Once` 永远不再可用。

### 接取

`accept_task` 校验：

1. 任务存在且账号存在；
2. 账号所在区域不是 `Closed`；
3. 等级 ≥ `min_level`；
4. 当前区域在 `regions` 内（若设置）；
5. `now` 在 `[starts_at_ms, ends_at_ms)` 内（若设置）；
6. `requires` 中的任务均已完成/领奖；
7. 没有进行中/待领奖的同一任务；重复任务冷却已结束（或已过期）。

接取会重置计数器并写入 `accepted_at_ms`、可选 `expires_at_ms`。道具/皮肤/成就目标按当前状态立即计算，可能接取后立刻 `Completed`。

### 推进与完成

- 指标目标：`record_event(account, metric, amount)` 推进所有进行中任务的同名计数器。
- 道具/皮肤/成就目标：实时计算；在 `add_item`、`grant_skin`、奖励发放、交易结算、任务查询/领奖时刷新完成时间。
- 完成时间只在期限内记录；在截止后才满足的目标不计入。
- 完成后即使道具被消耗，仍可领奖。

### 限时与重复

- `time_limit_ms` 从接取时间开始计算；完成后即使超过截止时间仍可领奖。
- 重复任务冷却从**领奖时间**开始，不是接取时间。
- 冷却时长：`TaskRepeat::daily()` = 24h，`weekly()` = 7d，`cooldown(ms)` 自定义。
- 新周期接取时计数器、完成/领奖时间全部重置。

## 交易

```text
Open ──(both confirm & settlement succeeds)──▶ Completed
 │
 ├──(either cancels)────────────▶ Cancelled
 └──(now >= expires_at_ms)──────▶ Expired
```

- 挂单不锁定资产；设置挂单时检查当前持仓，结算时再次检查。
- 双方确认后立即结算；任一方资产不足/背包放不下 → 交易保持 `Open`，双方确认清零，`confirm_trade` 返回错误。
- 结算顺序：双方先移除挂出资产，再互相放入，随后同时提交；因此交换同种道具不会因临时空间不足失败。
- `expire_trades()` 需要宿主周期调用；交易变更操作也会惰性检查过期。
- `cancel_trade` 在任何区域状态下都允许，便于清理。

## 区域

| 操作 | `Open` | `Maintenance` | `Closed` |
| --- | --- | --- | --- |
| 新账号归属 / 跨区迁入 | ✅ | ❌ | ❌ |
| 已有账号离开区域 | ✅ | ✅ | ✅ |
| 建房 / 进房 | ✅ | ✅ | ❌ |
| 玩家聊天（全局/区域/房间/私聊） | ✅ | ✅ | ❌ |
| 购买 | ✅ | ✅ | ❌ |
| 创建/修改/确认交易 | ✅ | ✅ | ❌ |
| 取消交易、领奖、放弃任务 | ✅ | ✅ | ✅ |
| 好友申请 / 接受申请 | ✅ | ✅ | ❌ |
| 接任务 / NPC 对话 | ✅ | ✅ | ❌ |
| 只读查询、资料/外观编辑 | ✅ | ✅ | ✅ |

`Closed` 的拒绝错误为 `GameError::RegionNotOpen(region_id)`。

## 房间

- 一个账号同时只能属于一个房间。
- 创建者自动加入并成为房主。
- 房主离开：所有权转交给剩余账号中最小的 `AccountId`。
- 最后一名玩家离开：房间删除。
- 房主关闭：所有成员解除成员关系。
- 房主踢人：被踢者变为无房间状态。
- 区域迁移会先退出当前房间。
- 区域 `Closed` 禁止建房/进房；`Maintenance` 允许已有账号继续。

## 资源再生

资源按 `regen_per_hour` 惰性再生，读取时结算：

```text
total_numerator = elapsed_ms * regen_per_hour + remainder
gained          = total_numerator / 3_600_000
remainder       = total_numerator % 3_600_000
```

- 每次读取都会把 `updated_at_ms` 推进到 `now`，因此同一时间戳重复读取不会重复发放。
- 余数会持久化，低速率资源跨多次短间隔读取不会丢进度。
- 达到 cap 后停止累积，余数清零。
- `set_resource` 会重置再生基准时间与余数。
- `add_resource` 在 cap 处截断；`spend_resource` 不足时返回错误且不扣除。

## 公告

- `starts_at_ms` 缺省为发布时间；`ends_at_ms` 为开区间上界。
- 活跃条件：`enabled && now >= starts_at_ms && now < ends_at_ms`（如有）。
- 区域定向：`regions = None` 表示全部；`Some(set)` 必须包含账号区域。
- 排序：优先级降序 → 创建时间降序 → ID。
- 已读状态按 `(公告, 账号)` 存储；公告删除会清除其已读集合。

## 在线状态

- `login` / `heartbeat`：`online = true`，更新 `last_seen_ms`。
- `logout`：`online = false`，更新 `last_seen_ms`。
- 库不自动做超时下线；宿主应根据 `last_seen_ms` 自行清理在线名单。
- 区域容量统计的是归属账号，不是在线账号。

## NPC 回合

1. `npc_context`：校验 NPC 启用、频道存在、说话者可访问，构造 persona、附近玩家、最近消息与 `now_ms`。
2. Agent 异步返回 `NpcDecision`。
3. 运行时检查动作目标是否在上下文允许集合（说话者 + 附近玩家）。
4. `apply_npc_turn` 在单锁内校验全部动作与消息，奖励先应用到克隆资产。
5. 全部通过后一次性提交奖励、排行榜分数、玩家消息、NPC 消息。
6. 任一动作失败：不写奖励、不写分数、不写聊天。

详见 [npc.md](npc.md)。
