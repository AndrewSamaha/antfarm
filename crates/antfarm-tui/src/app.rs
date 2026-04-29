use antfarm_core::{
    AsciiArtAsset, FullSyncChunk, FullSyncComplete, FullSyncStart, MoveDir, PatchFrame,
    PheromoneChannel, PheromoneMap, PlaceMaterial, Player, Position, ServerMessage, Snapshot,
    World, find_ascii_art_asset,
    default_server_config,
};
use crate::discovery::DiscoveredServer;
use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub(crate) struct App {
    pub(crate) player_name: String,
    pub(crate) player_id: u8,
    pub(crate) snapshot: Snapshot,
    pub(crate) mode: AppMode,
    pub(crate) persist_client_files: bool,
    pub(crate) camera_center: Position,
    pub(crate) show_help: bool,
    pending_startup_help: bool,
    pub(crate) show_params: bool,
    pub(crate) params_scroll: u16,
    pub(crate) show_events: bool,
    pub(crate) show_npc_bars: bool,
    pub(crate) pheromone_overlay: Option<PheromoneChannel>,
    pub(crate) pheromone_map: Option<PheromoneMap>,
    pub(crate) pending_command: PendingCommand,
    pub(crate) command_input: Option<String>,
    pub(crate) command_feedback: Option<String>,
    pub(crate) command_history: Vec<String>,
    pub(crate) command_history_index: Option<usize>,
    pub(crate) discovered_servers: Vec<DiscoveredServer>,
    pub(crate) selected_server_index: usize,
    pub(crate) selected_server_addr: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_info: Option<String>,
    pub(crate) max_history: usize,
    pub(crate) action_animation: Option<ActionAnimation>,
    pub(crate) queen_idle_states: HashMap<Position, QueenIdleState>,
    pub(crate) sync_state: SyncState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppMode {
    Live,
    Replay,
}

#[derive(Debug, Clone)]
pub(crate) enum SyncState {
    SelectingServer,
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct QueenIdleState {
    pub(crate) active_animation: Option<usize>,
    pub(crate) frame_index: usize,
    pub(crate) frame_until: Option<Instant>,
    pub(crate) next_trigger_at: Instant,
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
            camera_center: default_camera_center(&snapshot.world),
            snapshot,
            mode: AppMode::Live,
            persist_client_files: true,
            show_help: false,
            pending_startup_help: show_help,
            show_params: false,
            params_scroll: 0,
            show_events: false,
            show_npc_bars: false,
            pheromone_overlay: None,
            pheromone_map: None,
            pending_command: PendingCommand::None,
            command_input: None,
            command_feedback: None,
            command_history: Vec::new(),
            command_history_index: None,
            discovered_servers: Vec::new(),
            selected_server_index: 0,
            selected_server_addr: None,
            last_error: None,
            last_info: None,
            max_history,
            action_animation: None,
            queen_idle_states: HashMap::new(),
            sync_state: SyncState::Connecting,
        }
    }

    pub(crate) fn new_replay(snapshot: Snapshot, max_history: usize) -> Self {
        let camera_center = preferred_camera_center(&snapshot);
        Self {
            player_name: "replay-viewer".to_string(),
            player_id: 0,
            snapshot,
            mode: AppMode::Replay,
            persist_client_files: false,
            camera_center,
            show_help: true,
            pending_startup_help: false,
            show_params: false,
            params_scroll: 0,
            show_events: false,
            show_npc_bars: false,
            pheromone_overlay: None,
            pheromone_map: None,
            pending_command: PendingCommand::None,
            command_input: None,
            command_feedback: None,
            command_history: Vec::new(),
            command_history_index: None,
            discovered_servers: Vec::new(),
            selected_server_index: 0,
            selected_server_addr: None,
            last_error: None,
            last_info: None,
            max_history,
            action_animation: None,
            queen_idle_states: HashMap::new(),
            sync_state: SyncState::Ready,
        }
    }

    pub(crate) fn player(&self) -> Option<&Player> {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == self.player_id)
    }

    pub(crate) fn is_replay(&self) -> bool {
        self.mode == AppMode::Replay
    }

    pub(crate) fn is_selecting_server(&self) -> bool {
        matches!(self.sync_state, SyncState::SelectingServer)
    }

    pub(crate) fn focus_position(&self) -> Position {
        self.player()
            .map(|player| player.pos)
            .unwrap_or(self.camera_center)
    }

    pub(crate) fn preferred_hive_id(&self) -> Option<u16> {
        self.player()
            .and_then(|player| player.hive_id)
            .or_else(|| self.snapshot.npcs.iter().find_map(|npc| npc.hive_id))
            .or_else(|| self.snapshot.placed_art.iter().find_map(|placed| placed.hive_id))
    }

    pub(crate) fn pan_camera(&mut self, dir: MoveDir) {
        let (dx, dy) = dir.delta();
        let max_x = self.snapshot.world.width().saturating_sub(1);
        let max_y = self.snapshot.world.height().saturating_sub(1);
        self.camera_center = Position {
            x: (self.camera_center.x + dx).clamp(0, max_x),
            y: (self.camera_center.y + dy).clamp(0, max_y),
        };
    }

    pub(crate) fn toggle_local_pause(&mut self) {
        self.snapshot.simulation_paused = !self.snapshot.simulation_paused;
        if self.snapshot.simulation_paused {
            self.set_info("Replay paused");
        } else {
            self.set_info("Replay running");
        }
    }

    pub(crate) fn tick_animation(&mut self) {
        let now = Instant::now();
        if self.action_animation.is_some_and(|animation| now >= animation.until) {
            self.action_animation = None;
        }
        self.tick_queen_idle_animation(now);
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
        self.queen_idle_states.clear();
        self.pheromone_map = None;
        self.set_error(message);
    }

    pub(crate) fn begin_syncing(&mut self) {
        self.sync_state = SyncState::Syncing {
            received_rows: 0,
            total_rows: 0,
        };
        self.clear_status();
    }

    pub(crate) fn begin_server_selection(&mut self) {
        self.sync_state = SyncState::SelectingServer;
        self.clear_status();
    }

    pub(crate) fn start_full_sync(&mut self, start: &FullSyncStart) {
        self.player_id = start.player_id;
        self.snapshot.tick = start.tick;
        self.snapshot.world = World::empty(start.world_width, start.world_height);
        self.snapshot.players.clear();
        self.snapshot.npcs.clear();
        self.snapshot.placed_art.clear();
        self.snapshot.event_log.clear();
        self.snapshot.config = default_server_config();
        self.snapshot.simulation_paused = false;
        self.queen_idle_states.clear();
        self.pheromone_map = None;
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
        self.snapshot.placed_art = complete.placed_art;
        self.snapshot.event_log = complete.event_log;
        self.snapshot.config = complete.config;
        self.snapshot.simulation_paused = complete.simulation_paused;
        self.sync_queen_idle_states();
        self.sync_state = SyncState::Ready;
        if self.pending_startup_help {
            self.show_help = true;
            self.pending_startup_help = false;
        }
        self.clear_status();
        self.sync_status_from_latest_event();
    }

    pub(crate) fn open_params(&mut self) {
        self.show_params = true;
        self.params_scroll = 0;
    }

    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.sync_state, SyncState::Ready)
    }

    pub(crate) fn selected_server(&self) -> Option<&DiscoveredServer> {
        self.discovered_servers.get(self.selected_server_index)
    }

    pub(crate) fn move_server_selection(&mut self, delta: i32) {
        if self.discovered_servers.is_empty() {
            self.selected_server_index = 0;
            return;
        }
        let max_index = self.discovered_servers.len().saturating_sub(1) as i32;
        let next = (self.selected_server_index as i32 + delta).clamp(0, max_index) as usize;
        self.selected_server_index = next;
    }

    pub(crate) fn upsert_discovered_server(&mut self, server: DiscoveredServer) {
        if let Some(existing_index) = self
            .discovered_servers
            .iter()
            .position(|existing| existing.id == server.id)
        {
            self.discovered_servers[existing_index] = server;
            return;
        }
        self.discovered_servers.push(server);
        self.discovered_servers.sort_by(|left, right| {
            use crate::discovery::DiscoverySource;
            match (&left.source, &right.source) {
                (DiscoverySource::Localhost, DiscoverySource::Mdns) => std::cmp::Ordering::Less,
                (DiscoverySource::Mdns, DiscoverySource::Localhost) => std::cmp::Ordering::Greater,
                _ => left.label.cmp(&right.label),
            }
        });
        if self.selected_server_index >= self.discovered_servers.len() {
            self.selected_server_index = self.discovered_servers.len().saturating_sub(1);
        }
    }

    pub(crate) fn remove_discovered_server(&mut self, id: &str) {
        self.discovered_servers.retain(|server| server.id != id);
        if self.discovered_servers.is_empty() {
            self.selected_server_index = 0;
        } else if self.selected_server_index >= self.discovered_servers.len() {
            self.selected_server_index = self.discovered_servers.len().saturating_sub(1);
        }
    }

    pub(crate) fn choose_selected_server(&mut self) -> bool {
        let Some(server) = self.selected_server().cloned() else {
            return false;
        };
        self.selected_server_addr = Some(server.addr.clone());
        self.sync_state = SyncState::Connecting;
        self.set_info(format!("Connecting to {}", server.label));
        true
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

    fn sync_status_from_latest_event(&mut self) {
        let Some(latest_event) = self.snapshot.event_log.last().cloned() else {
            return;
        };

        let lowered = latest_event.to_ascii_lowercase();
        let looks_like_error = [
            "could not",
            "has no",
            "must place",
            "hit bedrock",
            "missing",
            "no players matched",
            "unknown resource",
        ]
        .iter()
        .any(|needle| lowered.contains(needle));

        if looks_like_error {
            self.set_error(latest_event);
        } else {
            self.set_info(latest_event);
        }
    }

    pub(crate) fn queen_rows_for_asset(
        &self,
        asset: &'static AsciiArtAsset,
        origin: Position,
    ) -> &'static [&'static str] {
        let Some(state) = self.queen_idle_states.get(&origin) else {
            return asset.rows;
        };
        let Some(animation_index) = state.active_animation else {
            return asset.rows;
        };
        let Some(animation) = asset.idle_animations.get(animation_index) else {
            return asset.rows;
        };
        animation
            .frames
            .get(state.frame_index)
            .map(|frame| frame.rows)
            .unwrap_or(asset.rows)
    }

    fn sync_queen_idle_states(&mut self) {
        let now = Instant::now();
        let mut next_states = HashMap::new();
        for placed in &self.snapshot.placed_art {
            let Some(asset) = find_ascii_art_asset(&placed.asset_id) else {
                continue;
            };
            if asset.id != "queen_ant" || asset.idle_animations.is_empty() {
                continue;
            }
            let state = self
                .queen_idle_states
                .get(&placed.pos)
                .copied()
                .unwrap_or_else(|| QueenIdleState {
                    active_animation: None,
                    frame_index: 0,
                    frame_until: None,
                    next_trigger_at: now
                        + Duration::from_millis(schedule_idle_interval_ms(
                            asset.idle_animations[0].average_interval_ms,
                            placed.pos,
                        )),
                });
            next_states.insert(placed.pos, state);
        }
        self.queen_idle_states = next_states;
    }

    fn tick_queen_idle_animation(&mut self, now: Instant) {
        self.sync_queen_idle_states();
        for placed in &self.snapshot.placed_art {
            let Some(asset) = find_ascii_art_asset(&placed.asset_id) else {
                continue;
            };
            if asset.id != "queen_ant" || asset.idle_animations.is_empty() {
                continue;
            }
            let Some(state) = self.queen_idle_states.get_mut(&placed.pos) else {
                continue;
            };
            match state.active_animation {
                Some(animation_index) => {
                    let Some(frame_until) = state.frame_until else {
                        state.active_animation = None;
                        state.frame_index = 0;
                        state.next_trigger_at = now
                            + Duration::from_millis(schedule_idle_interval_ms(
                                asset.idle_animations[animation_index].average_interval_ms,
                                placed.pos,
                            ));
                        continue;
                    };
                    if now < frame_until {
                        continue;
                    }
                    let animation = &asset.idle_animations[animation_index];
                    if state.frame_index + 1 < animation.frames.len() {
                        state.frame_index += 1;
                        state.frame_until = Some(
                            now + Duration::from_millis(animation.frames[state.frame_index].duration_ms),
                        );
                    } else {
                        state.active_animation = None;
                        state.frame_index = 0;
                        state.frame_until = None;
                        state.next_trigger_at = now
                            + Duration::from_millis(schedule_idle_interval_ms(
                                animation.average_interval_ms,
                                placed.pos,
                            ));
                    }
                }
                None => {
                    if now < state.next_trigger_at {
                        continue;
                    }
                    let animation_index = choose_idle_animation(asset, placed.pos);
                    let animation = &asset.idle_animations[animation_index];
                    state.active_animation = Some(animation_index);
                    state.frame_index = 0;
                    state.frame_until =
                        Some(now + Duration::from_millis(animation.frames[0].duration_ms));
                }
            }
        }
    }
}

fn preferred_camera_center(snapshot: &Snapshot) -> Position {
    snapshot
        .npcs
        .iter()
        .find(|npc| matches!(npc.kind, antfarm_core::NpcKind::Queen))
        .map(|npc| npc.pos)
        .or_else(|| snapshot.players.first().map(|player| player.pos))
        .unwrap_or_else(|| default_camera_center(&snapshot.world))
}

fn default_camera_center(world: &World) -> Position {
    Position {
        x: world.width() / 2,
        y: world.height() / 2,
    }
}

pub(crate) fn handle_server_message(app: &mut App, message: ServerMessage) {
    match message {
        ServerMessage::FullSyncStart(start) => app.start_full_sync(&start),
        ServerMessage::FullSyncChunk(chunk) => app.apply_full_sync_chunk(&chunk),
        ServerMessage::FullSyncComplete(complete) => app.finish_full_sync(complete),
        ServerMessage::PheromoneMap(map) => app.pheromone_map = Some(map),
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
    if let Some(placed_art) = patch.placed_art {
        app.snapshot.placed_art = placed_art;
    }
    if let Some(event_log) = patch.event_log {
        app.snapshot.event_log = event_log;
        app.sync_status_from_latest_event();
    }
    if let Some(config) = patch.config {
        app.snapshot.config = config;
    }
    if let Some(simulation_paused) = patch.simulation_paused {
        app.snapshot.simulation_paused = simulation_paused;
    }
    app.sync_queen_idle_states();
}

fn choose_idle_animation(asset: &'static AsciiArtAsset, pos: Position) -> usize {
    if asset.idle_animations.len() <= 1 {
        return 0;
    }
    (random_u64(pos) as usize) % asset.idle_animations.len()
}

fn schedule_idle_interval_ms(average_interval_ms: u64, pos: Position) -> u64 {
    let half = average_interval_ms / 2;
    let spread = average_interval_ms.max(1);
    half + (random_u64(pos) % (spread + 1))
}

fn random_u64(pos: Position) -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ ((pos.x as u64) << 32) ^ (pos.y as u64).wrapping_mul(0x9E37_79B9)
}
