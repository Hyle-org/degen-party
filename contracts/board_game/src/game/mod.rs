use anyhow::{anyhow, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use sdk::{ContractName, Identity, LaneId, StateCommitment};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod dice;
pub mod player;
pub mod utils;

const ROUNDS: usize = 10;
const MAX_PLAYERS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct GameState {
    pub players: Vec<Player>,
    pub max_players: usize,
    pub minigames: Vec<ContractName>,
    pub dice: dice::Dice,
    pub phase: GamePhase,
    pub round_started_at: u128,
    pub round: usize,
    pub bets: BTreeMap<Identity, u64>,
    pub all_or_nothing: bool,

    // Metadata to ensure the game runs smoothly
    pub backend_identity: Identity,
    pub last_interaction_time: u128,
    pub lane_id: LaneId,
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct Player {
    pub id: Identity,
    pub name: String,
    pub position: usize,
    pub coins: i32,
    pub used_uuids: Vec<u128>,
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct MinigameResult {
    pub contract_name: ContractName,
    pub player_results: Vec<PlayerMinigameResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct PlayerMinigameResult {
    pub player_id: Identity,
    pub coins_delta: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub enum GamePhase {
    Registration,
    Betting,
    WheelSpin,
    StartMinigame(ContractName),
    InMinigame(ContractName),
    FinalMinigame(ContractName),
    RewardsDistribution,
    GameOver,
}

pub type MinigameSetup = Vec<(Identity, String, u64)>;

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq)]
pub enum GameAction {
    EndGame,
    Initialize {
        minigames: Vec<String>,
        random_seed: u64,
    },
    RegisterPlayer {
        name: String,
        deposit: u64, // Initial deposit in coins
    },
    StartGame,
    PlaceBet {
        amount: u64,
    },
    SpinWheel,
    StartMinigame {
        minigame: ContractName,
        players: MinigameSetup,
    },
    EndMinigame {
        result: MinigameResult,
    },
    EndTurn,
    DistributeRewards,
}

#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum GameEvent {
    DiceRolled {
        player_id: Identity,
        value: u8,
    },
    PlayerMoved {
        player_id: Identity,
        new_position: usize,
    },
    CoinsChanged {
        player_id: Identity,
        amount: i32,
    },
    MinigameReady {
        minigame_type: String,
    },
    MinigameStarted {
        minigame_type: String,
    },
    MinigameEnded {
        result: MinigameResult,
    },
    TurnEnded {
        next_player: Identity,
    },
    GameEnded {
        winner_id: Identity,
        final_coins: i32,
    },
    GameInitialized {
        random_seed: u64,
    },
    PlayerRegistered {
        name: String,
        player_id: Identity,
    },
    GameStarted {
        player_count: usize,
    },
    BetPlaced {
        player_id: Identity,
        amount: u64,
    },
    WheelSpun {
        round: usize, // for convenience on frontend
        outcome: u8,
    },
    PlayersSwappedCoins {
        swaps: Vec<(Identity, Identity)>,
    },
    AllOrNothingActivated,
}

impl From<StateCommitment> for GameState {
    fn from(state: StateCommitment) -> Self {
        GameState::try_from_slice(&state.0).unwrap()
    }
}

impl GameState {
    pub fn new(backend_identity: Identity) -> Self {
        Self {
            players: Vec::new(),
            phase: GamePhase::GameOver,
            max_players: MAX_PLAYERS,
            minigames: Vec::new(),
            dice: dice::Dice::new(1, 10, 0),
            round_started_at: 0,
            round: 0,
            bets: BTreeMap::new(),
            all_or_nothing: false,

            backend_identity,
            last_interaction_time: 0,
            lane_id: LaneId::default(),
        }
    }

    pub fn reset(&mut self, minigames: Vec<ContractName>, random_seed: u64) {
        *self = Self {
            players: Vec::with_capacity(MAX_PLAYERS),
            phase: GamePhase::GameOver,
            max_players: MAX_PLAYERS,
            minigames,
            dice: dice::Dice::new(1, 10, random_seed),
            round_started_at: 0,
            round: 0,
            bets: BTreeMap::new(),
            all_or_nothing: false,

            backend_identity: self.backend_identity.clone(),
            last_interaction_time: self.last_interaction_time,
            lane_id: self.lane_id.clone(),
        }
    }

    // Helper function for updating coins and generating events
    fn update_player_coins(
        &mut self,
        player_index: usize,
        delta: i32,
        events: &mut Vec<GameEvent>,
    ) -> Result<()> {
        let Some(player) = self.players.get_mut(player_index) else {
            return Err(anyhow!("Player not found"));
        };
        player.coins = (player.coins + delta).max(0);
        events.push(GameEvent::CoinsChanged {
            player_id: player.id.clone(),
            amount: delta,
        });
        Ok(())
    }

    pub fn get_minigame_setup(&self) -> MinigameSetup {
        self.bets
            .iter()
            .filter_map(|(id, &bet)| {
                self.players
                    .iter()
                    .find(|p| p.id == *id && p.coins > 0)
                    .map(|p| (p.id.clone(), p.name.clone(), bet))
            })
            .collect()
    }

    // Helper function for handling minigame results
    fn apply_minigame_result(
        &mut self,
        player_index: usize,
        result: &PlayerMinigameResult,
        events: &mut Vec<GameEvent>,
    ) -> Result<()> {
        if result.coins_delta != 0 {
            self.update_player_coins(player_index, result.coins_delta, events)?;
        }
        Ok(())
    }

    fn is_registered(&self, caller: &Identity) -> bool {
        self.players.iter().any(|p| p.id == *caller && p.coins > 0)
    }

    /// Checks if the game should end due to players running out of coins.
    /// Emits a GameEnded event and sets phase if needed. Returns true if game ended.
    fn check_and_handle_game_over(&mut self, events: &mut Vec<GameEvent>) -> bool {
        let players_with_coins: Vec<_> = self.players.iter().filter(|p| p.coins > 0).collect();
        if players_with_coins.len() == 1 && self.players.len() > 1 {
            let winner = players_with_coins[0];
            events.push(GameEvent::GameEnded {
                winner_id: winner.id.clone(),
                final_coins: winner.coins,
            });
            self.phase = GamePhase::RewardsDistribution;
            true
        } else if players_with_coins.is_empty() {
            events.push(GameEvent::GameEnded {
                winner_id: Identity::default(),
                final_coins: 0,
            });
            self.phase = GamePhase::GameOver;
            true
        } else {
            false
        }
    }

    pub fn process_action(
        &mut self,
        caller: &Identity,
        _uuid: u128,
        action: GameAction,
        timestamp: u128,
    ) -> Result<Vec<GameEvent>> {
        let mut events = Vec::new();
        match (self.phase.clone(), action) {
            (_, GameAction::EndGame) => {
                let is_ended = self.phase == GamePhase::GameOver;
                let is_backend = self.backend_identity == *caller;
                let backend_timed_out = timestamp - self.last_interaction_time > 2 * 60 * 1000;
                let game_timed_out = timestamp - self.last_interaction_time > 10 * 60 * 1000;
                if is_ended || (is_backend && backend_timed_out) || game_timed_out {
                    events.push(GameEvent::GameEnded {
                        winner_id: Identity::default(),
                        final_coins: 0,
                    });
                    self.reset(self.minigames.clone(), self.dice.seed);
                } else {
                    return Err(anyhow!("Only the backend can end the game"));
                }
            }
            (
                GamePhase::GameOver,
                GameAction::Initialize {
                    minigames,
                    random_seed,
                },
            ) => {
                if minigames.is_empty() {
                    return Err(anyhow!("Minigames cannot be empty"));
                }
                self.reset(
                    minigames.into_iter().map(|x| x.into()).collect::<Vec<_>>(),
                    random_seed,
                );
                // Keep track of the time to know how long the registration phase lasts.
                self.round_started_at = timestamp;
                self.phase = GamePhase::Registration;
                events.push(GameEvent::GameInitialized { random_seed });
            }

            // Registration Phase
            (GamePhase::Registration, GameAction::RegisterPlayer { name, deposit }) => {
                if self.players.len() >= self.max_players {
                    return Err(anyhow!("Game is full"));
                }

                // Check if player already exists by public key
                if self.is_registered(caller) {
                    return Err(anyhow!("Player with identity {} already exists", caller));
                }

                // Check if player already exists by name
                if self.players.iter().any(|p| p.name == name) {
                    return Err(anyhow!("Player with name {} already exists", name));
                }

                // Check deposit amount
                if deposit == 0 {
                    return Err(anyhow!("Deposit must be greater than zero"));
                }
                if deposit > 10000000 {
                    return Err(anyhow!("Deposit exceeds maximum allowed amount"));
                }

                self.players.push(Player {
                    id: caller.clone(),
                    name: name.clone(),
                    position: 0,
                    coins: deposit as i32,
                    used_uuids: Vec::new(),
                });

                events.push(GameEvent::PlayerRegistered {
                    name: name.clone(),
                    player_id: caller.clone(),
                });
            }

            // Start Game Action
            (GamePhase::Registration, GameAction::StartGame) => {
                let is_full = self.players.len() == self.max_players;
                let registration_period_done =
                    self.round_started_at.saturating_add(55 * 1000) < timestamp;
                if !is_full && !registration_period_done {
                    return Err(anyhow!(
                        "Game is not full and registration period is not over"
                    ));
                }

                self.phase = GamePhase::Betting;
                self.round_started_at = timestamp;
                self.round = 0;
                events.push(GameEvent::GameStarted {
                    player_count: self.players.len(),
                });
            }

            // Betting Phase
            (GamePhase::Betting, GameAction::PlaceBet { amount }) => {
                if timestamp.saturating_sub(self.round_started_at) > 30_000 {
                    return Err(anyhow!("Betting time is over"));
                }
                if self.bets.contains_key(caller) {
                    return Err(anyhow!("Player has already placed a bet"));
                }
                let Some(player) = self.players.iter().find(|p| p.id == *caller) else {
                    return Err(anyhow!("Player {} not found", caller));
                };
                // Ignore players with zero coins
                if player.coins == 0 {
                    return Err(anyhow!("Player {} is out of the game (no coins)", caller));
                }
                if self.all_or_nothing {
                    if amount != player.coins as u64 {
                        return Err(anyhow!("All or nothing round: you must bet all your coins"));
                    }
                } else if player.coins < amount as i32 {
                    return Err(anyhow!("Player {} does not have enough coins", caller));
                }
                self.bets.insert(caller.clone(), amount);
                events.push(GameEvent::BetPlaced {
                    player_id: caller.clone(),
                    amount,
                });
                // Only require bets from players with coins > 0
                let active_players = self.players.iter().filter(|p| p.coins > 0).count();
                if self.bets.len() == active_players {
                    if self.round >= ROUNDS - 1 {
                        let Some(final_minigame) = self.minigames.first() else {
                            return Err(anyhow!("No final minigame available"));
                        };
                        events.push(GameEvent::MinigameReady {
                            minigame_type: final_minigame.0.clone(),
                        });
                        self.phase = GamePhase::FinalMinigame(final_minigame.clone());
                    } else {
                        self.phase = GamePhase::WheelSpin;
                    }
                } else {
                    self.phase = GamePhase::Betting;
                }
            }

            // Wheel Spin Phase
            (GamePhase::WheelSpin, GameAction::SpinWheel)
            | (GamePhase::Betting, GameAction::SpinWheel) => {
                if self.phase == GamePhase::Betting {
                    // Check we're over the timeout
                    if timestamp.saturating_sub(self.round_started_at) < 30_000 {
                        return Err(anyhow!("Not enough time has passed"));
                    }
                    // Collect indices of players to penalize
                    let to_penalize: Vec<_> = self
                        .players
                        .iter()
                        .enumerate()
                        .filter(|(_, player)| {
                            player.coins > 0 && !self.bets.contains_key(&player.id)
                        })
                        .map(|(i, _)| i)
                        .collect();
                    for &i in &to_penalize {
                        if self.round == 0 || self.all_or_nothing {
                            // In round 0 or all_or_nothing, set coins to 0
                            self.players[i].coins = 0;
                        } else {
                            // Otherwise, penalize 10 coins
                            self.update_player_coins(i, -10, &mut events)?;
                        }
                    }
                }
                // Reset after round
                self.all_or_nothing = false;
                // After coin updates, check for game over
                if self.check_and_handle_game_over(&mut events) {
                    return Ok(events);
                }
                // Use dice to determine the wheel outcome
                let outcome = self.dice.roll() % 5;
                events.push(GameEvent::WheelSpun {
                    outcome,
                    round: self.round,
                });
                match outcome {
                    0 => {
                        // Nothing happens, go to next round
                        self.round += 1;
                        self.bets.clear();
                        self.round_started_at = timestamp;
                        self.phase = GamePhase::Betting;
                    }
                    1 => {
                        // Randomly pay out the bets to players
                        let bet_entries: Vec<_> =
                            std::mem::take(&mut self.bets).into_iter().collect();
                        let mut player_indices: Vec<_> = (0..self.players.len())
                            .filter(|&i| self.players[i].coins > 0)
                            .collect();
                        self.dice.shuffle(&mut player_indices);
                        for (i, (bettor, amount)) in bet_entries.iter().enumerate() {
                            // Remove bet from bettor
                            let Some(bettor_idx) =
                                self.players.iter().position(|p| p.id == *bettor)
                            else {
                                return Err(anyhow!("Bettor not found"));
                            };
                            self.update_player_coins(bettor_idx, -(*amount as i32), &mut events)?;
                            // Pay out to a random player
                            let winner_idx = player_indices[i % player_indices.len()];
                            self.update_player_coins(winner_idx, *amount as i32, &mut events)?;
                        }
                        self.round += 1;
                        self.bets.clear();
                        self.round_started_at = timestamp;
                        self.phase = GamePhase::Betting;
                    }
                    2 => {
                        // All or nothing: players must bet all their coins next round
                        self.all_or_nothing = true;
                        events.push(GameEvent::AllOrNothingActivated);
                        self.round += 1;
                        self.bets.clear();
                        self.round_started_at = timestamp;
                        self.phase = GamePhase::Betting;
                    }
                    _ => {
                        // Minigame: emit MinigameReady and transition to InMinigame for StartMinigame
                        if let Some(minigame_type) = self.minigames.first() {
                            events.push(GameEvent::MinigameReady {
                                minigame_type: minigame_type.0.clone(),
                            });
                            self.phase = GamePhase::StartMinigame(minigame_type.clone());
                        } else {
                            // TODO: should be impossible
                            return Err(anyhow!("No minigame available"));
                        }
                    }
                }
            }

            (
                GamePhase::StartMinigame(expected_minigame),
                GameAction::StartMinigame { minigame, players },
            ) => {
                // Check the starting state is valid.
                if expected_minigame != minigame {
                    return Err(anyhow!("Minigame mismatch"));
                }
                let minigame_players = self.get_minigame_setup();
                if minigame_players != players {
                    return Err(anyhow!("Minigame players mismatch"));
                }
                events.push(GameEvent::MinigameStarted {
                    minigame_type: minigame.0.clone(),
                });
                self.phase = GamePhase::InMinigame(minigame);
            }

            (
                GamePhase::FinalMinigame(final_minigame),
                GameAction::StartMinigame { minigame, players },
            ) => {
                // Check the starting state is valid.
                if minigame != final_minigame {
                    return Err(anyhow!("Minigame mismatch"));
                }
                let minigame_players = self.get_minigame_setup();
                if minigame_players != players {
                    return Err(anyhow!("Minigame players mismatch"));
                }
                events.push(GameEvent::MinigameStarted {
                    minigame_type: minigame.0.clone(),
                });
                self.phase = GamePhase::InMinigame(minigame);
            }

            // InMinigame Phase
            (GamePhase::InMinigame(_), GameAction::EndMinigame { result }) => {
                // Apply results for each player
                for player_result in &result.player_results {
                    self.apply_minigame_result(
                        self.players
                            .iter()
                            .position(|p| p.id == player_result.player_id)
                            .ok_or_else(|| anyhow!("Player not found for minigame result"))?,
                        player_result,
                        &mut events,
                    )?;
                }

                // After coin updates, check for game over
                if self.check_and_handle_game_over(&mut events) {
                    return Ok(events);
                }

                events.push(GameEvent::MinigameEnded { result });

                // End the game if the round limit is reached
                if self.round >= ROUNDS - 1 {
                    let winner = self
                        .players
                        .iter()
                        .max_by_key(|p| p.coins)
                        .ok_or_else(|| anyhow!("No players found"))?;
                    events.push(GameEvent::GameEnded {
                        winner_id: winner.id.clone(),
                        final_coins: winner.coins,
                    });
                    self.phase = GamePhase::RewardsDistribution;
                } else {
                    self.round += 1;
                    self.bets.clear();
                    self.round_started_at = timestamp;
                    self.phase = GamePhase::Betting;
                }
            }
            // Rewards Distribution Phase
            (GamePhase::RewardsDistribution, GameAction::DistributeRewards) => {
                // Distribution is validated in lib.rs
                self.phase = GamePhase::GameOver;
            }

            // Invalid phase/action combinations
            (phase, action) => {
                return Err(anyhow!("Invalid action {:?} for phase {:?}", action, phase));
            }
        }

        Ok(events)
    }
}
