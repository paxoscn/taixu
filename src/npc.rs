//! NPC definitions and pluggable NPC agents.
//!
//! NPCs are static world entities (global, region-bound or room-bound). An
//! [`NpcAgent`] decides how an NPC responds to a player message; the built-in
//! [`RuleBasedNpc`] needs no external services, while LLM-backed agents can
//! implement the same async trait.
//!
//! [`NpcRuntime`] orchestrates a turn:
//! 1. builds a [`NpcContext`] (persona, channel, nearby players, recent chat),
//! 2. awaits the agent,
//! 3. validates every structured [`NpcAction`] and message,
//! 4. atomically commits actions (rewards, scores) and chat messages through
//!    [`crate::Game::apply_npc_turn`]. A failed action leaves no side effects.
//!
//! Agent decisions can be deserialized directly from JSON via
//! [`NpcDecision`], which is convenient for tool-calling / structured-output
//! LLM integrations.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chat::{ChatChannel, ChatMessage};
use crate::error::GameError;
use crate::ids::{AccountId, LeaderboardId, NpcId, RegionId, RoomId};
use crate::reward::Reward;
use crate::Game;

/// Static definition of an NPC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NpcDef {
    pub id: NpcId,
    pub name: String,
    /// Persona / behaviour prompt for the agent.
    pub persona: String,
    /// Region placement. Ignored when `room` is set.
    pub region: Option<RegionId>,
    /// Room placement; takes precedence over `region`.
    pub room: Option<RoomId>,
    pub enabled: bool,
    pub greeting: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl NpcDef {
    pub fn new(id: impl Into<NpcId>, name: impl Into<String>, persona: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            persona: persona.into(),
            region: None,
            room: None,
            enabled: true,
            greeting: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn in_region(mut self, region: impl Into<RegionId>) -> Self {
        self.region = Some(region.into());
        self
    }

    pub fn in_room(mut self, room: impl Into<RoomId>) -> Self {
        self.room = Some(room.into());
        self
    }

    pub fn with_greeting(mut self, greeting: impl Into<String>) -> Self {
        self.greeting = Some(greeting.into());
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Chat channel the NPC listens on.
    pub fn channel(&self) -> ChatChannel {
        if let Some(room) = &self.room {
            ChatChannel::Room(room.clone())
        } else if let Some(region) = &self.region {
            ChatChannel::Region(region.clone())
        } else {
            ChatChannel::Global
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NpcRegistry {
    npcs: BTreeMap<NpcId, NpcDef>,
}

impl NpcRegistry {
    pub fn register(&mut self, npc: NpcDef) -> crate::error::Result<()> {
        if npc.name.trim().is_empty() {
            return Err(GameError::InvalidInput("npc name must not be empty".into()));
        }
        if self.npcs.contains_key(&npc.id) {
            return Err(GameError::NpcAlreadyExists(npc.id));
        }
        self.npcs.insert(npc.id.clone(), npc);
        Ok(())
    }

    pub fn get(&self, id: &NpcId) -> Option<&NpcDef> {
        self.npcs.get(id)
    }

    pub fn get_mut(&mut self, id: &NpcId) -> Option<&mut NpcDef> {
        self.npcs.get_mut(id)
    }

    pub fn require(&self, id: &NpcId) -> crate::error::Result<&NpcDef> {
        self.get(id)
            .ok_or_else(|| GameError::NpcNotFound(id.clone()))
    }

    pub fn remove(&mut self, id: &NpcId) -> Option<NpcDef> {
        self.npcs.remove(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &NpcDef> {
        self.npcs.values()
    }

    pub fn len(&self) -> usize {
        self.npcs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.npcs.is_empty()
    }

    pub fn set_enabled(&mut self, id: &NpcId, enabled: bool) -> crate::error::Result<()> {
        let npc = self
            .npcs
            .get_mut(id)
            .ok_or_else(|| GameError::NpcNotFound(id.clone()))?;
        npc.enabled = enabled;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NearbyPlayer {
    pub account: AccountId,
    pub display_name: String,
    pub level: u32,
}

/// Everything an NPC agent needs to decide its turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NpcContext {
    pub npc: NpcDef,
    pub channel: ChatChannel,
    pub speaker: AccountId,
    pub speaker_name: String,
    pub speaker_level: u32,
    pub message: String,
    /// Recent channel history (oldest first), excluding the new message.
    pub recent_messages: Vec<ChatMessage>,
    pub nearby_players: Vec<NearbyPlayer>,
    pub now_ms: i64,
}

impl NpcContext {
    /// Render a compact, provider-agnostic prompt for an LLM-backed agent.
    pub fn system_prompt(&self) -> String {
        let mut prompt = String::new();
        prompt.push_str(&format!(
            "You are {}, an NPC in a multiplayer game.\nPersona: {}\n",
            self.npc.name, self.npc.persona
        ));
        prompt.push_str(&format!(
            "You are speaking in channel `{}` with player {} ({}), level {}.\n",
            self.channel, self.speaker_name, self.speaker, self.speaker_level
        ));
        if !self.nearby_players.is_empty() {
            prompt.push_str("Nearby players:\n");
            for player in &self.nearby_players {
                prompt.push_str(&format!(
                    "- {} ({}) level {}\n",
                    player.display_name, player.account, player.level
                ));
            }
        }
        if !self.recent_messages.is_empty() {
            prompt.push_str("Recent conversation:\n");
            for message in &self.recent_messages {
                prompt.push_str(&format!("- {:?}: {}\n", message.author, message.body));
            }
        }
        prompt.push_str(&format!("\nPlayer message: {}\n", self.message));
        prompt.push_str(
            "Reply in character. Return JSON with shape \
             {\"reply\": string|null, \"actions\": [ \
             {\"type\":\"say\",\"text\":string} | \
             {\"type\":\"give_reward\",\"account\":string,\"reward\":Reward} | \
             {\"type\":\"set_score\",\"leaderboard\":string,\"account\":string,\"score\":number} \
             ]}.",
        );
        prompt
    }

    pub fn allowed_account(&self, account: &AccountId) -> bool {
        *account == self.speaker
            || self
                .nearby_players
                .iter()
                .any(|player| player.account == *account)
    }
}

/// Structured action an agent can take after speaking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NpcAction {
    /// Say something in the current channel.
    Say { text: String },
    /// Grant a reward to the speaker or a nearby player.
    GiveReward { account: AccountId, reward: Reward },
    /// Set a leaderboard score for the speaker or a nearby player.
    SetScore {
        leaderboard: LeaderboardId,
        account: AccountId,
        score: i64,
    },
}

/// The agent's complete output for one turn.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NpcDecision {
    #[serde(default)]
    pub reply: Option<String>,
    #[serde(default)]
    pub actions: Vec<NpcAction>,
}

impl NpcDecision {
    pub fn reply(text: impl Into<String>) -> Self {
        Self {
            reply: Some(text.into()),
            actions: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum NpcError {
    #[error("npc agent error: {0}")]
    Agent(String),
    #[error("no agent registered for npc {0}")]
    NoAgent(NpcId),
    #[error(transparent)]
    Game(#[from] GameError),
}

impl NpcError {
    pub fn agent(message: impl Into<String>) -> Self {
        NpcError::Agent(message.into())
    }
}

/// Async decision engine attached to an NPC.
///
/// Implementations may call an LLM, a local model, a rules engine, or simply
/// return canned dialogue. They must not block the executor.
#[async_trait]
pub trait NpcAgent: Send + Sync {
    async fn respond(&self, context: &NpcContext) -> Result<NpcDecision, NpcError>;
}

/// Simple keyword-triggered agent useful as a default / for tests.
#[derive(Clone, Debug)]
pub struct RuleBasedNpc {
    rules: Vec<(String, String)>,
    fallback: String,
}

impl RuleBasedNpc {
    pub fn new(fallback: impl Into<String>) -> Self {
        Self {
            rules: Vec::new(),
            fallback: fallback.into(),
        }
    }

    /// Trigger on a case-insensitive substring.
    pub fn with_rule(mut self, trigger: impl Into<String>, response: impl Into<String>) -> Self {
        self.rules
            .push((trigger.into().to_lowercase(), response.into()));
        self
    }
}

#[async_trait]
impl NpcAgent for RuleBasedNpc {
    async fn respond(&self, context: &NpcContext) -> Result<NpcDecision, NpcError> {
        let message = context.message.to_lowercase();
        let reply = self
            .rules
            .iter()
            .find(|(trigger, _)| message.contains(trigger))
            .map(|(_, response)| response.clone())
            .unwrap_or_else(|| self.fallback.clone());
        Ok(NpcDecision::reply(reply))
    }
}

/// Outcome of a single NPC turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NpcTurn {
    pub channel: ChatChannel,
    /// The player message (posted only after the agent successfully responds).
    pub player_message: ChatMessage,
    /// NPC chat messages in the order they were posted.
    pub npc_messages: Vec<ChatMessage>,
    /// Number of world actions applied (rewards, scores, ...).
    pub applied_actions: usize,
}

/// Holds live agent instances keyed by NPC id.
///
/// Agents are intentionally kept outside [`Game`] state: they are not
/// serializable and may hold network clients.
#[derive(Default)]
pub struct NpcRuntime {
    agents: BTreeMap<NpcId, Arc<dyn NpcAgent>>,
}

impl std::fmt::Debug for NpcRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpcRuntime")
            .field("npcs", &self.agents.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl NpcRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach an agent, replacing any previous one. Returns the old agent.
    pub fn register(
        &mut self,
        npc: impl Into<NpcId>,
        agent: Arc<dyn NpcAgent>,
    ) -> Option<Arc<dyn NpcAgent>> {
        self.agents.insert(npc.into(), agent)
    }

    pub fn remove(&mut self, npc: &NpcId) -> Option<Arc<dyn NpcAgent>> {
        self.agents.remove(npc)
    }

    pub fn has_agent(&self, npc: &NpcId) -> bool {
        self.agents.contains_key(npc)
    }

    /// Run a full conversational turn.
    ///
    /// Player and NPC messages are only posted after the agent responds. The
    /// whole turn — actions and messages — is validated and committed
    /// atomically by [`crate::Game::apply_npc_turn`], so an invalid action
    /// leaves no rewards, scores or chat messages behind. Action targets are
    /// restricted to the context (speaker + nearby players).
    pub async fn converse(
        &self,
        game: &Game,
        npc_id: &NpcId,
        account: &AccountId,
        message: &str,
    ) -> Result<NpcTurn, NpcError> {
        let agent = self
            .agents
            .get(npc_id)
            .cloned()
            .ok_or_else(|| NpcError::NoAgent(npc_id.clone()))?;

        let context = game.npc_context(npc_id, account, message)?;
        let decision = agent.respond(&context).await?;

        for action in &decision.actions {
            match action {
                NpcAction::Say { text } => {
                    if text.trim().is_empty() {
                        return Err(NpcError::agent("agent returned an empty say action"));
                    }
                }
                NpcAction::GiveReward { account, .. } | NpcAction::SetScore { account, .. } => {
                    if !context.allowed_account(account) {
                        return Err(NpcError::agent(format!(
                            "agent tried to act on non-nearby account {account}"
                        )));
                    }
                }
            }
        }

        let NpcDecision { reply, actions } = decision;
        let mut npc_texts: Vec<String> = actions
            .iter()
            .filter_map(|action| match action {
                NpcAction::Say { text } => Some(text.clone()),
                _ => None,
            })
            .collect();
        if let Some(reply) = reply {
            if !reply.trim().is_empty() {
                npc_texts.push(reply);
            }
        }

        // Game applies the complete turn under one lock. A failing action
        // therefore leaves no rewards, scores or chat messages behind.
        Ok(game.apply_npc_turn(
            npc_id,
            account,
            context.channel,
            message,
            &actions,
            &npc_texts,
        )?)
    }
}
