# Changelog

## [Unreleased]

### Added

- Task / quest system: metric, item, skin and achievement objectives; prerequisites,
  min level, region targeting, availability windows, time limits and
  daily/weekly/cooldown repeats.
- `Game::record_event` advances achievements and active tasks together.
- `Game::channels_for` + `ChannelSummary` for per-account channel discovery.
- `Game::with_state` for guard-free read queries.
- `GameState.version` / `GAME_STATE_VERSION` for snapshot migrations.
- Documentation handbook under `docs/`: API index, JSON wire contracts, state
  machines, persistence, hosting, NPC contract, configuration, errors, cookbook.
- JSON contract tests and additional runnable examples.
- Regression suites for the first- and second-round findings.

### Changed

- `TaskView` now exposes `requires`, `regions`, `starts_at_ms`, `ends_at_ms`, and
  presents a fresh cycle after a repeatable task's cooldown. Cooldown elapsed but
  ineligible tasks report `Locked` instead of `Claimed`.
- `Game::trades_for` returns newest-first.
- `RegionStatus::Closed` now blocks player chat, purchases, trades, friend
  requests, task acceptance and NPC turns for accounts in that region.
  Read-only queries, reward claims, profile/cosmetic edits and cleanup remain
  available.
- `update_avatar` mirrors frame/badge into equipped skins (and `None` unequips);
  `equip_skin` / `unequip_skin` mirror back into the avatar.
- Resource regeneration now persists a fractional remainder and advances the
  reference timestamp on every read, preventing repeated-read inflation.
- `Game::snapshot` includes a schema version.
- The prelude no longer exports `Result`; use `GameResult<T>` or
  `std::result::Result`.

### Fixed

- NPC structured actions and chat are now committed atomically through
  `Game::apply_npc_turn`.
- `ensure_account` is race-free under concurrent first login.
- Timed tasks completed inside their deadline remain claimable after expiry.
- Maintenance regions allow existing players to keep using rooms; Closed still
  blocks new activity.
- System messages validate their target channel/entities.
- Repeatable task views no longer leak the previous cycle's counters or
  timestamps.
- Avatar frame/badge and equipped skin state stay in sync.
- README examples are compiled as doctests.

### Known limitations

- No default retention/GC policy for purchase history, trades, chat channels,
  announcement read sets, or account deletion/anonymization. Hosts must define
  retention and erase workflows.
- `GameStore` is synchronous; async database drivers need a blocking adapter or
  a host-side snapshot pipeline.
- Snapshot migration is versioned but not automatic; hosts own schema
  transformations.
- Currency display metadata (names/icons) is host-defined; only currency ids
  exist in the library.
