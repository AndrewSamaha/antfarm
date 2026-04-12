use antfarm_core::{
    FullSyncChunk, FullSyncComplete, FullSyncStart, MoveDir, PatchFrame, PlaceMaterial, Player,
    ServerMessage, Snapshot, World, default_server_config,
};
use std::time::Instant;

#[derive(Debug)]
pub(crate) struct App {
    pub(crate) player_name: String,
    pub(crate) player_id: u8,
    pub(crate) snapshot: Snapshot,
    pub(crate) show_help: bool,
    pub(crate) show_params: bool,
    pub(crate) params_scroll: u16,
    pub(crate) show_events: bool,
    pub(crate) pending_command: PendingCommand,
    pub(crate) command_input: Option<String>,
    pub(crate) command_feedback: Option<String>,
    pub(crate) command_history: Vec<String>,
    pub(crate) command_history_index: Option<usize>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_info: Option<String>,
    pub(crate) max_history: usize,
    pub(crate) action_animation: Option<ActionAnimation>,
    pub(crate) sync_state: SyncState,
}

#[derive(Debug, Clone)]
pub(crate) enum SyncState {
    Connecting,
    Syncing { received_rows: i32, total_rows: i32 },
    Ready,
    Reconnecting,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PendingCommand {
    None,
    PlaceMaterial,
    PlaceDirection(PlaceMaterial),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActionAnimation {
    pub(crate) dir: MoveDir,
    pub(crate) until: Instant,
}

impl App {
    pub(crate) fn new(
        player_name: String,
        player_id: u8,
        snapshot: Snapshot,
        show_help: bool,
        max_history: usize,
    ) -> Self {
        Self {
            player_name,
            player_id,
            snapshot,
            show_help,
            show_params: false,
            params_scroll: 0,
            show_events: false,
            pending_command: PendingCommand::None,
            command_input: None,
            command_feedback: None,
            command_history: Vec::new(),
            command_history_index: None,
            last_error: None,
            last_info: None,
            max_history,
            action_animation: None,
            sync_state: SyncState::Connecting,
        }
    }

    pub(crate) fn player(&self) -> Option<&Player> {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == self.player_id)
    }

    pub(crate) fn tick_animation(&mut self) {
        if self
            .action_animation
            .is_some_and(|animation| Instant::now() >= animation.until)
        {
            self.action_animation = None;
        }
    }

    pub(crate) fn enter_reconnecting(&mut self, message: String) {
        self.sync_state = SyncState::Reconnecting;
        self.pending_command = PendingCommand::None;
        self.command_input = None;
        self.command_feedback = None;
        self.command_history_index = None;
        self.show_params = false;
        self.params_scroll = 0;
        self.action_animation = None;
        self.set_error(message);
    }

    pub(crate) fn begin_syncing(&mut self) {
        self.sync_state = SyncState::Syncing {
            received_rows: 0,
            total_rows: 0,
        };
        self.clear_status();
    }

    pub(crate) fn start_full_sync(&mut self, start: &FullSyncStart) {
        self.player_id = start.player_id;
        self.snapshot.tick = start.tick;
        self.snapshot.world = World::empty(start.world_width, start.world_height);
        self.snapshot.players.clear();
        self.snapshot.npcs.clear();
        self.snapshot.event_log.clear();
        self.snapshot.config = default_server_config();
        self.sync_state = SyncState::Syncing {
            received_rows: 0,
            total_rows: start.total_rows,
        };
    }

    pub(crate) fn apply_full_sync_chunk(&mut self, chunk: &FullSyncChunk) {
        for (offset, row) in chunk.rows.iter().enumerate() {
            self.snapshot
                .world
                .set_row_tiles(chunk.start_row + offset as i32, row);
        }
        if let SyncState::Syncing {
            received_rows,
            total_rows: _,
        } = &mut self.sync_state
        {
            *received_rows = (*received_rows).max(chunk.start_row + chunk.rows.len() as i32);
        }
    }

    pub(crate) fn finish_full_sync(&mut self, complete: FullSyncComplete) {
        self.snapshot.players = complete.players;
        self.snapshot.npcs = complete.npcs;
        self.snapshot.event_log = complete.event_log;
        self.snapshot.config = complete.config;
        self.sync_state = SyncState::Ready;
        self.clear_status();
    }

    pub(crate) fn open_params(&mut self) {
        self.show_params = true;
        self.params_scroll = 0;
    }

    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.sync_state, SyncState::Ready)
    }

    pub(crate) fn set_error(&mut self, message: impl Into<String>) {
        self.last_error = Some(message.into());
        self.last_info = None;
    }

    pub(crate) fn set_info(&mut self, message: impl Into<String>) {
        self.last_info = Some(message.into());
        self.last_error = None;
    }

    pub(crate) fn clear_status(&mut self) {
        self.last_error = None;
        self.last_info = None;
    }
}

pub(crate) fn handle_server_message(app: &mut App, message: ServerMessage) {
    match message {
        ServerMessage::FullSyncStart(start) => app.start_full_sync(&start),
        ServerMessage::FullSyncChunk(chunk) => app.apply_full_sync_chunk(&chunk),
        ServerMessage::FullSyncComplete(complete) => app.finish_full_sync(complete),
        ServerMessage::Patch(patch) => apply_patch_frame(app, patch),
        ServerMessage::Error { message } => app.set_error(message),
    }
}

fn apply_patch_frame(app: &mut App, patch: PatchFrame) {
    app.snapshot.tick = patch.tick;

    for update in patch.tiles {
        let _ = app.snapshot.world.set_tile(update.pos, update.tile);
    }

    if let Some(players) = patch.players {
        app.snapshot.players = players;
    }
    if let Some(npcs) = patch.npcs {
        app.snapshot.npcs = npcs;
    }
    if let Some(event_log) = patch.event_log {
        app.snapshot.event_log = event_log;
    }
    if let Some(config) = patch.config {
        app.snapshot.config = config;
    }
}
