use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{GameState, Snapshot};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayArtifact {
    pub version: u8,
    pub kind: String,
    pub start_tick: u64,
    pub simulation_length: u64,
    pub expected_final_tick: u64,
    pub initial_snapshot_hash: String,
    pub expected_final_snapshot_hash: String,
    pub initial_snapshot: Snapshot,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayVerification {
    pub initial_snapshot_hash: String,
    pub expected_final_snapshot_hash: String,
    pub actual_final_snapshot_hash: String,
    pub final_tick: u64,
    pub matches_expected: bool,
}

impl ReplayArtifact {
    pub fn new(
        initial_snapshot: Snapshot,
        simulation_length: u64,
        expected_final_snapshot_hash: String,
        metadata: Value,
    ) -> Result<Self, serde_json::Error> {
        let start_tick = initial_snapshot.tick;
        let initial_snapshot_hash = initial_snapshot.deterministic_hash_hex()?;
        Ok(Self {
            version: 1,
            kind: "deterministic_replay".to_string(),
            start_tick,
            simulation_length,
            expected_final_tick: start_tick.saturating_add(simulation_length),
            initial_snapshot_hash,
            expected_final_snapshot_hash,
            initial_snapshot,
            metadata,
        })
    }

    pub fn replay(&self) -> Result<ReplayVerification, serde_json::Error> {
        let initial_snapshot_hash = self.initial_snapshot.deterministic_hash_hex()?;
        let mut game = GameState::from_replay_snapshot(self.initial_snapshot.clone());
        for _ in 0..self.simulation_length {
            game.tick();
        }
        let final_snapshot = game.snapshot();
        let actual_final_snapshot_hash = final_snapshot.deterministic_hash_hex()?;
        Ok(ReplayVerification {
            initial_snapshot_hash,
            expected_final_snapshot_hash: self.expected_final_snapshot_hash.clone(),
            actual_final_snapshot_hash: actual_final_snapshot_hash.clone(),
            final_tick: final_snapshot.tick,
            matches_expected: actual_final_snapshot_hash == self.expected_final_snapshot_hash,
        })
    }
}
