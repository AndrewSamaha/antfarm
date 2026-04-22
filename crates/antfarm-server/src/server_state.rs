use antfarm_core::{GameState, Player, ServerMessage, Snapshot};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
};
use tokio::sync::{Mutex, Notify};

use crate::{debug_npc::NpcDebugSession, experiment::ExperimentRunContext};

pub(crate) type ClientTx = tokio::sync::mpsc::UnboundedSender<ServerMessage>;

#[derive(Clone)]
pub(crate) struct ServerState {
    pub(crate) game: Arc<Mutex<GameState>>,
    pub(crate) clients: Arc<Mutex<HashMap<u8, ClientTx>>>,
    pub(crate) session_tokens: Arc<Mutex<HashMap<u8, String>>>,
    pub(crate) persistence_tx: mpsc::Sender<PersistMessage>,
    pub(crate) npc_debug: Arc<Mutex<Option<NpcDebugSession>>>,
    pub(crate) experiment: Arc<Mutex<Option<ExperimentRunContext>>>,
    pub(crate) shutdown_notify: Arc<Notify>,
    pub(crate) tick_millis: u64,
}

pub(crate) enum PersistMessage {
    Save(Snapshot),
    UpsertPlayerProfile { token: String, player: Player },
    ClearPlayerProfiles,
}
