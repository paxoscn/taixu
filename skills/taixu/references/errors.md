# 错误与原子性

`GameError` 是唯一的领域错误类型；宿主应映射为稳定的协议错误码（错误本身不是 serde 类型）。

## 错误变体

### 账号与区域

| 变体 | 含义 |
| --- | --- |
| `AccountNotFound(id)` | 账号不存在 |
| `AccountAlreadyExists(id)` | 创建了已存在的账号 |
| `InvalidName(reason)` | 显示名/房间名不合法 |
| `RegionNotFound(id)` | 区域不存在 |
| `RegionAlreadyExists(id)` | 重复注册区域 |
| `RegionNotOpen(id)` | 区域非 `Open`（创建/迁入），或账号所在区域为 `Closed` 的新活动 |
| `RegionFull(id)` | 区域归属容量已满 |

### 房间

| 变体 | 含义 |
| --- | --- |
| `RoomNotFound(id)` | 房间不存在 |
| `RoomFull(id)` | 达到房间容量 |
| `AlreadyInRoom { account, room }` | 账号已在另一个房间 |
| `NotInRoom(account)` | 账号没有房间 |
| `NotRoomMember(account, room)` | 不是房间成员 |
| `NotRoomOwner` | 需要房主权限 |
| `InvalidRoomCapacity { requested, max }` | 容量为 0 或超过配置上限 |

### 通用与钱包

| 变体 | 含义 |
| --- | --- |
| `InvalidInput(reason)` | 参数/状态不合法 |
| `InvalidQuantity(n)` | 数量必须 > 0 |
| `AmountOverflow` | 金额/计数溢出 |
| `AlreadyExists(name)` | 泛型重复实体 |
| `UnknownResource(id)` | 资源未注册 |
| `ResourceAlreadyExists(id)` | 重复注册资源 |
| `InsufficientFunds { currency, needed, available }` | 货币不足 |
| `InsufficientResources { resource, needed, available }` | 资源不足 |

### 道具与背包

| 变体 | 含义 |
| --- | --- |
| `UnknownItem(id)` | 道具未注册 |
| `ItemAlreadyExists(id)` | 重复注册道具 |
| `InsufficientItems { item, needed, available }` | 道具不足 |
| `InventoryFull` | 槽位不足（原子失败，不部分写入） |

### 好友

| 变体 | 含义 |
| --- | --- |
| `CannotFriendSelf` | 不能加自己 |
| `AlreadyFriends(id)` | 已是好友 |
| `FriendRequestPending` | 已有待处理申请 |
| `FriendRequestNotFound` | 申请不存在 |
| `Blocked` | 双向拉黑阻断 |
| `AlreadyBlocked` / `NotBlocked` | 拉黑状态不符 |

### 交易

| 变体 | 含义 |
| --- | --- |
| `TradeNotFound(id)` | 交易不存在 |
| `TradeNotOpen` | 已完成/取消/过期 |
| `NotTradeParticipant(id)` | 非交易参与方 |
| `CannotTradeWithSelf` | 不能与自己交易 |
| `EmptyTrade` | 双方挂单都为空 |
| `TradeExpired` | 超过有效期 |
| `TradeAssetsUnavailable` | 资产不可用（保留变体，结算通常报具体不足） |

### 商店

| 变体 | 含义 |
| --- | --- |
| `ShopNotFound(id)` / `OfferNotFound(id)` | 商店/商品不存在 |
| `ShopAlreadyExists(id)` / `OfferAlreadyExists(id)` | 重复注册 |
| `OfferUnavailable` | 时间窗/区域/商店关闭 |
| `OutOfStock` | 库存不足 |
| `PurchaseLimitReached` | 账号限购已满 |

### 外观、收集、排行榜、成就

| 变体 | 含义 |
| --- | --- |
| `SkinNotFound(id)` | 皮肤未注册 |
| `SkinAlreadyExists(id)` | 重复注册皮肤 |
| `SkinNotOwned(id)` | 未拥有皮肤 |
| `SkinCategoryMismatch { skin, expected, actual }` | 皮肤分类与头像槽不符 |
| `CollectionNotFound(id)` / `CollectionAlreadyExists(id)` | 图鉴不存在/重复 |
| `MilestoneNotFound(index)` | 里程碑下标越界 |
| `MilestoneLocked` | 收集数未达里程碑 |
| `AlreadyClaimed` | 收集里程碑重复领奖 |
| `LeaderboardNotFound(id)` / `LeaderboardAlreadyExists(id)` | 排行榜不存在/重复 |
| `AchievementNotFound(id)` / `AchievementAlreadyExists(id)` | 成就不存在/重复 |
| `AchievementNotCompleted` / `AchievementAlreadyClaimed` | 成就领奖前置/重复 |

### 任务

| 变体 | 含义 |
| --- | --- |
| `TaskNotFound(id)` / `TaskAlreadyExists(id)` | 任务不存在/重复 |
| `TaskLocked(id)` | 等级/区域/前置/时间窗不满足 |
| `TaskAlreadyAccepted(id)` | 已在进行中 |
| `TaskAlreadyCompleted(id)` | 已完成待领奖 |
| `TaskNotAccepted(id)` | 未接取或进度不存在 |
| `TaskNotCompleted(id)` | 目标未完成 |
| `TaskAlreadyClaimed(id)` | 已领奖（Once 任务不可再接） |
| `TaskOnCooldown { task, available_at_ms }` | 重复任务冷却未结束 |

### 公告、聊天、NPC、持久化

| 变体 | 含义 |
| --- | --- |
| `AnnouncementNotFound(id)` / `InvalidAnnouncementWindow` | 公告不存在/时间窗非法 |
| `ChannelAccessDenied(channel)` | 无权读/写频道，或 NPC 频道不匹配 |
| `MessageNotFound(id)` | 消息不存在 |
| `EmptyContent` | 内容为空（trim 后） |
| `ContentTooLong { max }` | 超过字符上限 |
| `NpcNotFound(id)` / `NpcAlreadyExists(id)` / `NpcDisabled(id)` | NPC 不存在/重复/停用 |
| `Persistence(message)` | 快照/存储错误 |

## 原子性矩阵

| 操作 | 失败时的行为 |
| --- | --- |
| 商店 `purchase` | 不扣费、不发货、不消耗库存/限购 |
| 交易 `confirm_trade` | 转回 `Open`、双方确认清零；双方资产不变 |
| `claim_task` / `claim_achievement` / `claim_collection_milestone` | 不发奖、不标记已领 |
| `grant_reward` / `apply_npc_turn` | 不写任何奖励；NPC 回合也不写聊天/分数 |
| `add_item` / `remove_item` | 背包保持原状（全有或全无） |
| `add_currency` / `spend_currency` | 余额保持原状（溢出/不足） |
| `add_resource` / `spend_resource` | cap/不足检查失败时余额不变（再生可能已结算，但不改变可消费额度） |
| `accept_task` | 不写入新进度；冷却/锁定错误无副作用 |
| `create_trade` / `set_trade_side` | 不创建/不修改挂单 |
| `join_room` / `create_room` | 不修改成员关系/不创建房间 |
| `send_message` | 不写入聊天日志 |
| `set_account_region` | 校验失败不迁移、不退出房间 |

## 方法 → 常见错误

| 方法 | 常见错误 |
| --- | --- |
| `create_account` / `ensure_account` | `AccountAlreadyExists`, `InvalidName`, `RegionNotFound`, `RegionNotOpen`, `RegionFull` |
| `set_account_region` | `AccountNotFound`, `RegionNotFound`, `RegionNotOpen`, `RegionFull` |
| `create_room` | `AccountNotFound`, `InvalidName`, `InvalidRoomCapacity`, `RegionNotOpen`, `AlreadyInRoom`, `RoomNotFound` |
| `join_room` | `AccountNotFound`, `RoomNotFound`, `RoomFull`, `AlreadyInRoom`, `RegionNotOpen`, `InvalidInput` |
| `purchase` | `AccountNotFound`, `ShopNotFound`, `OfferNotFound`, `OfferUnavailable`, `OutOfStock`, `PurchaseLimitReached`, `InsufficientFunds`, `InsufficientResources`, `UnknownResource`, `UnknownItem`, `SkinNotFound`, `InventoryFull`, `AmountOverflow`, `RegionNotOpen` |
| `create_trade` | `AccountNotFound`, `CannotTradeWithSelf`, `RegionNotOpen` |
| `set_trade_side` / `confirm_trade` | `TradeNotFound`, `TradeNotOpen`, `NotTradeParticipant`, `TradeExpired`, `InsufficientItems`, `InsufficientFunds`, `UnknownItem`, `InventoryFull`, `EmptyTrade`, `RegionNotOpen` |
| `accept_task` | `TaskNotFound`, `TaskLocked`, `TaskAlreadyAccepted`, `TaskAlreadyCompleted`, `TaskAlreadyClaimed`, `TaskOnCooldown`, `RegionNotOpen` |
| `claim_task` | `TaskNotFound`, `TaskNotAccepted`, `TaskNotCompleted`, `TaskAlreadyClaimed`, `InventoryFull`, `UnknownItem`, `UnknownResource`, `SkinNotFound`, `AmountOverflow` |
| `send_message` | `AccountNotFound`, `RegionNotOpen`, `ChannelAccessDenied`, `Blocked`, `RoomNotFound`, `EmptyContent`, `ContentTooLong` |
| `send_system_message` | `RegionNotFound`, `RoomNotFound`, `AccountNotFound`, `InvalidInput`, `EmptyContent`, `ContentTooLong` |
| `channels_for` | `AccountNotFound` |
| `apply_npc_turn` / `converse` | `NpcNotFound`, `NpcDisabled`, `ChannelAccessDenied`, `RegionNotOpen`, `LeaderboardNotFound`, `AccountNotFound`, `UnknownItem`, `UnknownResource`, `SkinNotFound`, `InventoryFull`, `EmptyContent`, `ContentTooLong`, `InvalidInput` |

## 重试建议

- 不要自动重试扣费/发货/交易确认类失败；先刷新客户端状态。
- 只读查询和“资源不足”类错误可安全重试，但要重新读取最新状态。
- 并发首登使用 `ensure_account`，不要自己先读后写。
- 版本不匹配的 `Persistence` 错误不可重试，应走迁移。
