//! Chat channels, message history and DM blocking.
//!
//! Supported scopes: global, region, room and private (direct message).
//! History is retained per channel with a bounded capacity. Permission checks
//! live in [`crate::Game`]; this module is the storage / validation layer.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{GameError, Result};
use crate::ids::{AccountId, MessageId, NpcId, RegionId, RoomId};

/// Destination of a chat message.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChatChannel {
    Global,
    Region(RegionId),
    Room(RoomId),
    /// Direct message channel. Participants are stored in sorted order.
    Private {
        a: AccountId,
        b: AccountId,
    },
}

impl ChatChannel {
    /// Build a normalized private channel for two participants.
    pub fn private(a: AccountId, b: AccountId) -> Self {
        if a <= b {
            ChatChannel::Private { a, b }
        } else {
            ChatChannel::Private { a: b, b: a }
        }
    }

    pub fn is_private(&self) -> bool {
        matches!(self, ChatChannel::Private { .. })
    }

    pub fn participants(&self) -> Option<(&AccountId, &AccountId)> {
        match self {
            ChatChannel::Private { a, b } => Some((a, b)),
            _ => None,
        }
    }
}

impl fmt::Display for ChatChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatChannel::Global => f.write_str("global"),
            ChatChannel::Region(region) => write!(f, "region:{region}"),
            ChatChannel::Room(room) => write!(f, "room:{room}"),
            ChatChannel::Private { a, b } => {
                // Length-prefix the first id so IDs containing ':' remain safe.
                write!(f, "dm:{}:{}{}", a.as_str().len(), a, b)
            }
        }
    }
}

impl Serialize for ChatChannel {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ChatChannel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == "global" {
            return Ok(ChatChannel::Global);
        }
        let (kind, rest) = value
            .split_once(':')
            .ok_or_else(|| D::Error::custom(format!("invalid chat channel: {value}")))?;
        match kind {
            "region" => Ok(ChatChannel::Region(RegionId::from(rest))),
            "room" => Ok(ChatChannel::Room(RoomId::from(rest))),
            "dm" => {
                let (len, rest) = rest
                    .split_once(':')
                    .ok_or_else(|| D::Error::custom(format!("invalid DM channel: {value}")))?;
                let len: usize = len
                    .parse()
                    .map_err(|_| D::Error::custom(format!("invalid DM channel: {value}")))?;
                if !rest.is_char_boundary(len) {
                    return Err(D::Error::custom(format!(
                        "invalid DM channel boundary: {value}"
                    )));
                }
                let (a, b) = rest.split_at(len);
                if b.is_empty() {
                    return Err(D::Error::custom(format!("invalid DM channel: {value}")));
                }
                Ok(ChatChannel::private(a.into(), b.into()))
            }
            _ => Err(D::Error::custom(format!(
                "unknown chat channel kind: {kind}"
            ))),
        }
    }
}

/// Who authored a chat message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatAuthor {
    Account { account: AccountId },
    Npc { npc: NpcId },
    System,
}

impl ChatAuthor {
    pub fn account(account: impl Into<AccountId>) -> Self {
        ChatAuthor::Account {
            account: account.into(),
        }
    }

    pub fn npc(npc: impl Into<NpcId>) -> Self {
        ChatAuthor::Npc { npc: npc.into() }
    }

    pub fn account_id(&self) -> Option<&AccountId> {
        match self {
            ChatAuthor::Account { account } => Some(account),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: MessageId,
    pub channel: ChatChannel,
    pub author: ChatAuthor,
    pub body: String,
    pub sent_at_ms: i64,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChatLog {
    messages: VecDeque<ChatMessage>,
}

impl ChatLog {
    fn new() -> Self {
        Self {
            messages: VecDeque::new(),
        }
    }

    fn push(&mut self, message: ChatMessage, capacity: usize) {
        if capacity == 0 {
            return;
        }
        self.messages.push_back(message);
        while self.messages.len() > capacity {
            self.messages.pop_front();
        }
    }
}

/// Bounded per-channel message store.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatService {
    capacity: usize,
    logs: BTreeMap<ChatChannel, ChatLog>,
}

impl Default for ChatService {
    fn default() -> Self {
        Self::new(100)
    }
}

impl ChatService {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            logs: BTreeMap::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn post(
        &mut self,
        channel: ChatChannel,
        author: ChatAuthor,
        body: &str,
        metadata: BTreeMap<String, String>,
        now_ms: i64,
        max_len: usize,
    ) -> Result<ChatMessage> {
        let body = body.trim();
        if body.is_empty() {
            return Err(GameError::EmptyContent);
        }
        if body.chars().count() > max_len {
            return Err(GameError::ContentTooLong { max: max_len });
        }
        let message = ChatMessage {
            id: MessageId::new(),
            channel: channel.clone(),
            author,
            body: body.to_owned(),
            sent_at_ms: now_ms,
            metadata,
        };
        let capacity = self.capacity;
        self.logs
            .entry(channel)
            .or_insert_with(ChatLog::new)
            .push(message.clone(), capacity);
        Ok(message)
    }

    /// Recent messages in chronological order (oldest first).
    pub fn recent(&self, channel: &ChatChannel, limit: usize) -> Vec<ChatMessage> {
        let Some(log) = self.logs.get(channel) else {
            return Vec::new();
        };
        let skip = log.messages.len().saturating_sub(limit);
        log.messages.iter().skip(skip).cloned().collect()
    }

    pub fn messages_since(&self, channel: &ChatChannel, since_ms: i64) -> Vec<ChatMessage> {
        self.logs
            .get(channel)
            .map(|log| {
                log.messages
                    .iter()
                    .filter(|message| message.sent_at_ms >= since_ms)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn delete(&mut self, channel: &ChatChannel, message_id: &MessageId) -> bool {
        let Some(log) = self.logs.get_mut(channel) else {
            return false;
        };
        let before = log.messages.len();
        log.messages.retain(|message| message.id != *message_id);
        before != log.messages.len()
    }

    pub fn clear(&mut self, channel: &ChatChannel) -> bool {
        self.logs.remove(channel).is_some()
    }

    pub fn channel_count(&self) -> usize {
        self.logs.len()
    }

    pub fn message_count(&self, channel: &ChatChannel) -> usize {
        self.logs
            .get(channel)
            .map(|log| log.messages.len())
            .unwrap_or(0)
    }

    pub fn channels(&self) -> impl Iterator<Item = &ChatChannel> {
        self.logs.keys()
    }
}
