use antfarm_core::{GameState, Player, ServerMessage, Snapshot};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
};
use tokio::sync::Mutex;

pub(crate) type ClientTx = tokio::sync::mpsc::UnboundedSender<ServerMessage>;

#[derive(Clone)]
pub(crate) struct ServerState {
    pub(crate) game: Arc<Mutex<GameState>>,
    pub(crate) clients: Arc<Mutex<HashMap<u8, ClientTx>>>,
    pub(crate) session_tokens: Arc<Mutex<HashMap<u8, String>>>,
    pub(crate) persistence_tx: mpsc::Sender<PersistMessage>,
}

pub(crate) enum PersistMessage {
    Save(Snapshot),
    UpsertPlayerProfile { token: String, player: Player },
    ClearPlayerProfiles,
}
