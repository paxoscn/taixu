# API 参考

`taixu` 的完整签名、字段和枚举由 Rust 文档生成：

```bash
cargo doc --no-deps --open
```

本页是**运营视角的索引**：按领域列出门面方法、锁类型、原子性与常见错误。每个方法的权威签名和 rustdoc 请以 `cargo doc` 为准。

## 锁约定

| 标记 | 含义 |
| --- | --- |
| `R` | 只读锁 |
| `W` | 写锁；同一线程持有读锁 guard 时调用会死锁 |
| `W*` | 写锁，因为资源会惰性再生或任务会刷新完成状态 |
| `R!` | `read_state()` 返回 guard；guard 存活期间禁止调用任何 `W` 方法 |

短只读查询优先用 `Game::with_state(|state| ...)`，不要让 `read_state()` guard 逃逸作用域。

所有时间均为 Unix 毫秒（`i64`）。

## 构造与状态

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `Game::new()` | - | 默认配置 + 系统时钟 |
| `Game::with_config(config)` | - | 自定义限制 |
| `Game::with_clock(clock)` | - | 注入 `ManualClock` 等 |
| `Game::with_config_and_clock(config, clock)` | - | 完整构造 |
| `Game::from_state(state, config, clock)` | - | 恢复快照；`version == 0` 归一化为当前版本 |
| `Game::snapshot()` | R | 克隆完整 `GameState`，包含 `version` |
| `Game::state_version()` | R | 当前快照 schema 版本 |
| `Game::config()` | - | 构造时的配置（**不随快照持久化**） |
| `Game::now_ms()` | - | 注入时钟的当前时间 |
| `Game::with_state(query)` | R | 安全的短查询 |

## 区域与账号

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `create_region(id, name, capacity, endpoint)` | W | 注册节点；容量必须 > 0 |
| `region(id)` / `regions()` | R | 查询区域 |
| `set_region_status(id, status)` | W | `Open / Maintenance / Closed`，矩阵见 [state-machines.md](state-machines.md) |
| `region_player_count(id)` | R | 归属账号数，不是在线数 |
| `create_account(id, name, region)` | W | 外部鉴权后创建游戏档案；区域必须 `Open` |
| `ensure_account(id, name, region)` | W | 单锁内“存在则返回，否则创建”，并发安全 |
| `account(id)` / `accounts_in_region(region)` | R | 查询档案 |
| `login` / `logout` / `heartbeat` | W | 在线标记与 `last_seen_ms` |
| `update_display_name` / `set_level` / `add_xp` | W | 档案更新；不会自动升级 |
| `set_account_metadata` / `remove_account_metadata` | W | 宿主自定义字段 |
| `set_account_region(id, region)` | W | 目标区域必须 `Open`；会退出当前房间 |

账号鉴权不在库内：`AccountId` 由宿主认证后传入。

## 道具、背包、钱包与奖励

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_item(def)` / `item(id)` / `items()` | W/R | 道具目录 |
| `register_resource(def)` / `resources()` | W/R | 资源目录，含 cap 与每小时再生 |
| `inventory(account)` | R | 返回背包克隆（槽位顺序稳定） |
| `item_count(account, item)` | R | 跨槽位总数 |
| `add_item` / `remove_item` | W | 全有或全无；`add_item` 会刷新任务完成状态 |
| `set_inventory_capacity(account, capacity)` | W | 扩容直接生效；缩容要求被裁掉的槽位为空 |
| `wallet(account)` | W* | 返回已结算再生后的余额快照 |
| `currency_balance` / `resource_balance` | R/W* | 资源余额读取会惰性再生 |
| `add_currency` / `spend_currency` | W | 溢出/不足返回错误 |
| `add_resource` / `spend_resource` / `set_resource` | W* | 资源受 cap 限制；`set_resource` 清空再生余量 |
| `grant_reward(account, reward)` | W | 原子发放货币/资源/道具/皮肤/XP |

货币是动态 ID，没有内置货币目录；名称、图标和展示顺序由宿主配置。

## 皮肤与头像

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_skin(def)` / `skins()` | W/R | 皮肤目录 |
| `grant_skin` / `owns_skin` / `owned_skins` | W/R | 发奖幂等：已拥有返回 `false` 或保持拥有 |
| `equip_skin(account, skin)` | W | 按 `SkinCategory` 覆盖同名槽；Frame/Badge 会同步到 Avatar |
| `unequip_skin(account, category)` | W | 卸下；Frame/Badge 会清空 Avatar 对应字段 |
| `equipped_skins(account)` | R | 分类 → 皮肤 |
| `update_avatar(account, avatar)` | W | Avatar 为权威状态；Frame/Badge 同步到 equipped，`None` 即卸下；校验分类与拥有关系 |

## 好友

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `send_friend_request(from, to, message)` | W | 双向拉黑、已好友、已有待处理申请会被拒绝 |
| `accept_friend_request(receiver, sender)` | W | 建立双向好友关系 |
| `decline_friend_request` / `cancel_friend_request` | W | 只删除待处理申请 |
| `remove_friend(a, b)` | W | 移除双向关系 |
| `block_account(blocker, blocked)` | W | 加黑并同时清理好友关系和双向待处理申请 |
| `unblock_account` | W | 解除拉黑，不恢复旧关系 |
| `friends` / `friend_requests` / `outgoing_friend_requests` / `blocked_accounts` | R | 查询列表 |

拉黑是单向存储，但请求与私聊检查取双向结果。

## 房间

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `create_room(owner, name, region, capacity, metadata)` | W | 房主自动加入；同一账号只能在一个房间 |
| `rooms(region)` | R | 按创建时间升序 |
| `room(id)` / `room_members(id)` / `room_id_of(account)` | R | 查询 |
| `join_room(account, room)` | W | 区域必须匹配；容满返回 `RoomFull` |
| `leave_room(account)` | W | 房主离开时转交给最小 `AccountId`；房间空则删除 |
| `close_room(owner, room)` / `kick_from_room(owner, room, target)` | W | 仅房主 |
| `update_room_metadata` / `remove_room_metadata` | W | 仅房主 |

`Maintenance` 区域允许已有账号建房/进房，`Closed` 禁止。

## 聊天

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `send_message(account, channel, body)` | W | 校验频道权限、区域状态、内容长度 |
| `send_message_with_metadata(...)` | W | 同上，附带 `BTreeMap` |
| `recent_messages(account, channel, limit)` | R | 按时间正序返回最近 N 条 |
| `channel_history(channel, limit)` | R | 运营/审核用，无账号权限校验 |
| `channels_for(account)` | R | 可访问且有历史的频道，按最新消息倒序 |
| `send_system_message(channel, body)` | W | 运营消息；校验 Region/Room/Private 目标存在 |
| `send_npc_message(npc, channel, body)` | W | 只能写入该 NPC 绑定频道 |
| `delete_message` / `clear_channel` | W | 内容管理 |

频道字符串格式、私聊排序与作者 tag 见 [json-contracts.md](json-contracts.md)。

## 商店

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_shop(shop)` / `shop(id)` / `shops()` | W/R | 商店与商品目录 |
| `add_offer(shop, offer)` / `remove_offer` | W | 商品 ID 在商店内唯一 |
| `purchase(account, shop, offer, quantity)` | W | 校验时间窗/区域/库存/限购 → 扣费 → 发货 → 记录购买；失败零副作用 |
| `purchased_quantity(account, shop, offer)` | R | 历史累计购买单位数 |

`Reward::scaled(quantity)` 会缩放货币/资源/道具/XP；皮肤是唯一资产，多份购买只授予一次。

## 交易

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `create_trade(initiator, partner)` | W | 双方必须存在且区域不是 `Closed` |
| `set_trade_side(account, trade, side)` | W | 校验当前持仓；双方确认标记重置 |
| `confirm_trade(account, trade)` | W | 双方确认后原子结算；失败则保持开放并清空双方确认 |
| `cancel_trade(account, trade)` | W | 允许清理，`Closed` 区域也可取消 |
| `trade(id)` / `trades_for(account)` | R | `trades_for` 按创建时间倒序 |
| `expire_trades()` | W | 宿主周期调用；标记超时开放交易为 `Expired` |

结算顺序：双方移出挂单资产 → 双方放入对方资产 → 同时提交。资产不足或背包放不下时回滚。

## 任务

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_task(def)` / `task_definition(id)` / `task_definitions()` | W/R | 任务目录；隐藏任务仍可由 ID 查询/接取 |
| `task_view(account, id)` / `task_views(account)` | W | 刷新完成状态后返回视图 |
| `available_tasks` / `active_tasks` / `completed_tasks` | W | 按状态筛选 |
| `accept_task(account, id)` | W | 校验等级/区域/前置/时间窗/冷却；`Closed` 区域拒绝 |
| `abandon_task(account, id)` | W | 已领奖任务不能放弃 |
| `claim_task(account, id)` | W | 完成且未领奖时原子发货 |

任务状态、冷却与限时语义见 [state-machines.md](state-machines.md)。

## 成就与收集

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_achievement` / `achievement` / `achievements` | W/R | 成就目录 |
| `record_event(account, metric, amount)` | W | 同时推进成就与所有进行中任务，返回 `MetricOutcome` |
| `record_metric(...)` | W | 兼容入口：推进两者，只返回新完成成就 ID |
| `achievement_view` / `achievements_for` | R | 隐藏成就完成后才出现在列表中 |
| `claim_achievement(account, id)` | W | 一次性领奖 |
| `register_collection` / `collection` / `collections` | W/R | 图鉴目录 |
| `collection_progress(account, id)` | R | 实时计算道具/皮肤/成就条目 |
| `claim_collection_milestone(account, id, index)` | W | 达到阈值后一次性领奖 |

## 排行榜

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_leaderboard(board)` / `leaderboard(id)` | W/R | 支持高/低分、Top-N、赛季 |
| `set_leaderboard_score(board, account, score)` | W | 覆盖分数 |
| `add_leaderboard_score(board, account, delta)` | W | 增量，溢出返回错误 |
| `leaderboard_score` / `leaderboard_rank` | R | 查询 |
| `leaderboard_top(board, limit)` | R | 并列名次共享 rank |
| `remove_leaderboard_score` / `reset_leaderboard_season` | W | 删除/开启新赛季 |

## 公告

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `publish_announcement(draft)` | W | 生成 ID；校验标题/正文/时间窗/区域 |
| `announcement` / `announcements` | R | 运营查询 |
| `active_announcements(account)` / `active_announcements_for_region(region)` | R | 按区域与时间过滤，优先级降序 |
| `unread_announcements(account)` | R | 活跃且未读 |
| `mark_announcement_read(account, id)` | W | 每个账号独立已读 |
| `set_announcement_enabled` / `remove_announcement` | W | 停用/删除（删除会清理已读集合） |

## NPC

| 方法 | 锁 | 说明 |
| --- | --- | --- |
| `register_npc(npc)` / `npc(id)` / `npcs()` | W/R | 与房间/区域绑定 |
| `set_npc_enabled` / `remove_npc` | W | 启停/删除定义 |
| `npc_context(npc, account, message)` | R | 校验频道/区域/房间并构造 Agent 上下文 |
| `apply_npc_turn(npc, speaker, channel, player_message, actions, npc_messages)` | W | 校验全部动作后一次性提交奖励、排行榜与聊天 |
| `NpcRuntime::converse(...)` | async | 构造上下文 → 等待 Agent → 通过 `apply_npc_turn` 原子提交 |

详见 [npc.md](npc.md)。

## 错误与原子性

错误变体与各操作的失败副作用见 [errors.md](errors.md)。核心原则：

- 购买、交易结算、任务/成就/收集领奖、NPC 回合都是原子的。
- 房间、好友、区域等单步操作在失败前完成全部校验。
- 宿主应把 `GameError` 映射为协议错误码；错误本身不是 serde 类型。
