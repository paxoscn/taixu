//! Snapshot persistence hooks.
//!
//! The core service is in-memory and runtime-agnostic. To persist, serialize
//! [`GameState`] (it derives `serde::Serialize` /
//! `Deserialize`) through a [`GameStore`] implementation — for example one
//! backed by Postgres, Redis, S3 or a file.
//!
//! [`MemoryStore`] is provided for testing and for hosts that persist the
//! state themselves.

use std::sync::Arc;

use parking_lot::Mutex;
use thiserror::Error;

use crate::clock::{Clock, SystemClock};
use crate::error::{GameError, Result};
use crate::{Game, GameConfig, GameState};

#[derive(Debug, Error)]
#[error("store error: {0}")]
pub struct StoreError(pub String);

impl StoreError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Synchronous snapshot storage.
///
/// Implementations should treat [`GameStore::save`] as an atomic replacement
/// of the whole snapshot.
pub trait GameStore: Send + Sync {
    fn load(&self) -> std::result::Result<Option<GameState>, StoreError>;
    fn save(&self, state: &GameState) -> std::result::Result<(), StoreError>;
}

/// In-process store, useful for tests and graceful hand-off between instances.
#[derive(Default)]
pub struct MemoryStore {
    state: Mutex<Option<GameState>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_state(state: GameState) -> Self {
        Self {
            state: Mutex::new(Some(state)),
        }
    }

    pub fn clear(&self) {
        *self.state.lock() = None;
    }
}

impl GameStore for MemoryStore {
    fn load(&self) -> std::result::Result<Option<GameState>, StoreError> {
        Ok(self.state.lock().clone())
    }

    fn save(&self, state: &GameState) -> std::result::Result<(), StoreError> {
        *self.state.lock() = Some(state.clone());
        Ok(())
    }
}

impl Game {
    /// Serialize the current state into a store.
    pub fn save_to(&self, store: &dyn GameStore) -> Result<()> {
        store
            .save(&self.snapshot())
            .map_err(|error| GameError::Persistence(error.to_string()))
    }

    /// Load a game from a store using the system clock.
    pub fn load_from(store: &dyn GameStore, config: GameConfig) -> Result<Option<Game>> {
        Self::load_from_with_clock(store, config, Arc::new(SystemClock))
    }

    /// Load a game from a store with an explicit clock.
    pub fn load_from_with_clock(
        store: &dyn GameStore,
        config: GameConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Option<Game>> {
        let state = store
            .load()
            .map_err(|error| GameError::Persistence(error.to_string()))?;
        Ok(state.map(|state| Game::from_state(state, config, clock)))
    }
}
