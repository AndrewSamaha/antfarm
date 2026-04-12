use antfarm_core::{
    Action, ClientMessage, FullSyncChunk, FullSyncComplete, FullSyncStart, MoveDir, PlaceMaterial,
    PatchFrame, Player, Position, SURFACE_Y, ServerMessage, Snapshot, Tile, Viewport, World,
    default_server_config,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use rand::{Rng, rng};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
    time::{self, timeout},
};

const RECONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(900);

#[derive(Debug, Serialize, Deserialize)]
struct ClientConfig {
    token: String,
}

#[derive(Debug)]
struct App {
    player_name: String,
    player_id: u8,
    snapshot: Snapshot,
    show_help: bool,
    show_params: bool,
    params_scroll: u16,
    show_events: bool,
    pending_command: PendingCommand,
    command_input: Option<String>,
    command_feedback: Option<String>,
    command_history: Vec<String>,
    command_history_index: Option<usize>,
    last_error: Option<String>,
    action_animation: Option<ActionAnimation>,
    sync_state: SyncState,
}

#[derive(Debug, Clone)]
enum SyncState {
    Connecting,
    Syncing { received_rows: i32, total_rows: i32 },
    Ready,
    Reconnecting,
}

#[derive(Debug, Clone, Copy)]
enum PendingCommand {
    None,
    PlaceMaterial,
    PlaceDirection(PlaceMaterial),
}

#[derive(Debug, Clone, Copy)]
struct ActionAnimation {
    dir: MoveDir,
    until: Instant,
}

impl App {
    fn new(player_name: String, player_id: u8, snapshot: Snapshot) -> Self {
        Self {
            player_name,
            player_id,
            snapshot,
            show_help: true,
            show_params: false,
            params_scroll: 0,
            show_events: false,
            pending_command: PendingCommand::None,
            command_input: None,
            command_feedback: None,
            command_history: Vec::new(),
            command_history_index: None,
            last_error: None,
            action_animation: None,
            sync_state: SyncState::Connecting,
        }
    }

    fn player(&self) -> Option<&Player> {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == self.player_id)
    }

    fn tick_animation(&mut self) {
        if self
            .action_animation
            .is_some_and(|animation| Instant::now() >= animation.until)
        {
            self.action_animation = None;
        }
    }

    fn enter_reconnecting(&mut self, message: String) {
        self.sync_state = SyncState::Reconnecting;
        self.pending_command = PendingCommand::None;
        self.command_input = None;
        self.command_feedback = None;
        self.command_history_index = None;
        self.show_params = false;
        self.params_scroll = 0;
        self.action_animation = None;
        self.last_error = Some(message);
    }

    fn begin_syncing(&mut self) {
        self.sync_state = SyncState::Syncing {
            received_rows: 0,
            total_rows: 0,
        };
        self.last_error = None;
    }

    fn start_full_sync(&mut self, start: &FullSyncStart) {
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

    fn apply_full_sync_chunk(&mut self, chunk: &FullSyncChunk) {
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

    fn finish_full_sync(&mut self, complete: FullSyncComplete) {
        self.snapshot.players = complete.players;
        self.snapshot.npcs = complete.npcs;
        self.snapshot.event_log = complete.event_log;
        self.snapshot.config = complete.config;
        self.sync_state = SyncState::Ready;
        self.last_error = None;
    }

    fn open_params(&mut self) {
        self.show_params = true;
        self.params_scroll = 0;
    }

    fn is_ready(&self) -> bool {
        matches!(self.sync_state, SyncState::Ready)
    }
}

struct Connection {
    writer: tokio::net::tcp::OwnedWriteHalf,
    network_rx: mpsc::UnboundedReceiver<ServerMessage>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let name = env::args()
        .nth(1)
        .unwrap_or_else(|| "worker-ant".to_string());
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let terminal = ratatui::init();

    let result = run_app(terminal, name).await;

    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

async fn run_app(mut terminal: DefaultTerminal, player_name: String) -> Result<()> {
    let client_token = load_or_create_client_token(&player_name)?;
    let mut app = App::new(player_name.clone(), 0, offline_snapshot());
    app.sync_state = SyncState::Connecting;
    let mut events = EventStream::new();
    let mut redraw = time::interval(Duration::from_millis(33));
    let mut reconnect = time::interval(Duration::from_millis(1000));
    reconnect.tick().await;
    let mut connection: Option<Connection> = None;

    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = redraw.tick() => app.tick_animation(),
            _ = reconnect.tick(), if connection.is_none() => {
                match timeout(RECONNECT_ATTEMPT_TIMEOUT, connect_session(&app.player_name, &client_token)).await {
                    Ok(Ok(new_connection)) => {
                        app.begin_syncing();
                        connection = Some(new_connection);
                    }
                    Ok(Err(_)) | Err(_) => {
                        app.enter_reconnecting("attempting to reconnect".to_string());
                    }
                }
            }
            maybe_message = recv_server_message(&mut connection), if connection.is_some() => {
                match maybe_message {
                    Some(ServerMessage::FullSyncStart(start)) => app.start_full_sync(&start),
                    Some(ServerMessage::FullSyncChunk(chunk)) => app.apply_full_sync_chunk(&chunk),
                    Some(ServerMessage::FullSyncComplete(complete)) => app.finish_full_sync(complete),
                    Some(ServerMessage::Patch(patch)) => apply_patch_frame(&mut app, patch),
                    Some(ServerMessage::Error { message }) => app.last_error = Some(message),
                    None => {
                        connection = None;
                        app.enter_reconnecting("attempting to reconnect".to_string());
                    }
                }
            }
            maybe_event = tokio_stream_event(&mut events) => {
                if let Some(event) = maybe_event? {
                    let writer = connection.as_mut().map(|connection| &mut connection.writer);
                    if handle_event(event, &mut app, writer).await? {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn connect_session(player_name: &str, client_token: &str) -> Result<Connection> {
    let stream = timeout(
        RECONNECT_ATTEMPT_TIMEOUT,
        TcpStream::connect("127.0.0.1:7000"),
    )
    .await
    .context("timed out connecting to antfarm-server")?
    .context("connect to antfarm-server on 127.0.0.1:7000")?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    let join = serde_json::to_string(&ClientMessage::Join {
        name: player_name.to_string(),
        token: client_token.to_string(),
    })?;
    writer.write_all(join.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let (network_tx, network_rx) = mpsc::unbounded_channel::<ServerMessage>();
    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            match serde_json::from_str::<ServerMessage>(&line) {
                Ok(message) => {
                    let _ = network_tx.send(message);
                }
                Err(error) => {
                    let _ = network_tx.send(ServerMessage::Error {
                        message: error.to_string(),
                    });
                    break;
                }
            }
        }
    });

    Ok(Connection { writer, network_rx })
}

fn load_or_create_client_token(player_name: &str) -> Result<String> {
    let path = client_config_path(player_name);
    if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read client config at {}", path.display()))?;
        let config: ClientConfig =
            toml::from_str(&content).context("parse client config TOML")?;
        if !config.token.trim().is_empty() {
            return Ok(config.token);
        }
    }

    let token = generate_client_token();
    let config = ClientConfig {
        token: token.clone(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create client config dir {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(&config).context("serialize client config TOML")?;
    fs::write(&path, content).with_context(|| format!("write client config {}", path.display()))?;
    Ok(token)
}

fn client_config_path(player_name: &str) -> PathBuf {
    let slug = sanitize_player_name(player_name);
    Path::new("data")
        .join("clients")
        .join(format!("{slug}.toml"))
}

fn sanitize_player_name(player_name: &str) -> String {
    let mut slug = String::new();
    for ch in player_name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            slug.push(ch);
        } else {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_');
    if slug.is_empty() {
        "worker-ant".to_string()
    } else {
        slug.to_string()
    }
}

fn generate_client_token() -> String {
    let token: u128 = rng().random();
    format!("{token:032x}")
}

async fn recv_server_message(connection: &mut Option<Connection>) -> Option<ServerMessage> {
    let connection = connection.as_mut()?;
    connection.network_rx.recv().await
}

fn offline_snapshot() -> Snapshot {
    Snapshot {
        tick: 0,
        world: World::empty(1, 1),
        players: Vec::new(),
        npcs: Vec::new(),
        event_log: Vec::new(),
        config: default_server_config(),
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

async fn tokio_stream_event(events: &mut EventStream) -> Result<Option<Event>> {
    use futures_util::StreamExt;

    Ok(events.next().await.transpose()?)
}

async fn handle_event(
    event: Event,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<bool> {
    let Event::Key(key) = event else {
        return Ok(false);
    };

    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    if app.show_params {
        match key.code {
            KeyCode::Esc => {
                app.show_params = false;
                app.params_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                app.params_scroll = app.params_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.params_scroll = app.params_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                app.params_scroll = app.params_scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                app.params_scroll = app.params_scroll.saturating_sub(10);
            }
            KeyCode::Char('q') => return Ok(true),
            _ => {}
        }
        return Ok(false);
    }

    if app.command_input.is_some() {
        return handle_command_input(key.code, app, writer).await;
    }

    if !app.is_ready() {
        if matches!(key.code, KeyCode::Char('q')) {
            return Ok(true);
        }
        if matches!(key.code, KeyCode::Char('?')) {
            app.show_help = !app.show_help;
        }
        return Ok(false);
    }

    let direction = match key.code {
        KeyCode::Char('k') => Some(MoveDir::Up),
        KeyCode::Char('j') => Some(MoveDir::Down),
        KeyCode::Char('h') => Some(MoveDir::Left),
        KeyCode::Char('l') => Some(MoveDir::Right),
        _ => None,
    };

    if let Some(dir) = direction {
        let action = match app.pending_command {
            PendingCommand::None => default_action(app, dir),
            PendingCommand::PlaceMaterial => {
                app.pending_command = PendingCommand::None;
                default_action(app, dir)
            }
            PendingCommand::PlaceDirection(material) => {
                app.pending_command = PendingCommand::None;
                Action::Place { dir, material }
            }
        };
        if matches!(action, Action::Dig(_) | Action::Place { .. }) {
            app.action_animation = Some(ActionAnimation {
                dir,
                until: Instant::now() + Duration::from_millis(110),
            });
        }
        if let Some(writer) = writer {
            send_action(writer, action).await?;
        }
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Esc => {
            app.pending_command = PendingCommand::None;
            app.command_input = None;
            app.command_feedback = None;
            app.command_history_index = None;
            app.show_params = false;
            app.params_scroll = 0;
        }
        KeyCode::Char('?') => app.show_help = !app.show_help,
        KeyCode::Char('e') => app.show_events = !app.show_events,
        KeyCode::Char('/') => {
            app.command_input = Some("/".to_string());
            app.command_feedback = command_suggestion("/");
            app.command_history_index = None;
            app.pending_command = PendingCommand::None;
            app.last_error = None;
        }
        KeyCode::Char(' ') => app.pending_command = PendingCommand::PlaceMaterial,
        KeyCode::Char('d') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Dirt);
            app.last_error = None;
        }
        KeyCode::Char('s') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Stone);
            app.last_error = None;
        }
        _ => {}
    }

    Ok(false)
}

async fn send_action(writer: &mut tokio::net::tcp::OwnedWriteHalf, action: Action) -> Result<()> {
    let payload = serde_json::to_string(&ClientMessage::Action(action))?;
    writer.write_all(payload.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

async fn send_message(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    message: ClientMessage,
) -> Result<()> {
    let payload = serde_json::to_string(&message)?;
    writer.write_all(payload.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

async fn handle_command_input(
    code: KeyCode,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<bool> {
    let Some(input) = app.command_input.as_mut() else {
        return Ok(false);
    };

    match code {
        KeyCode::Esc => {
            app.command_input = None;
            app.command_feedback = None;
            app.command_history_index = None;
        }
        KeyCode::Backspace => {
            app.command_history_index = None;
            input.pop();
            if input.is_empty() {
                app.command_input = None;
                app.command_feedback = None;
            }
        }
        KeyCode::Enter => {
            let command = input.clone();
            app.command_input = None;
            app.command_feedback = None;
            app.command_history_index = None;
            submit_command(command, app, writer).await?;
        }
        KeyCode::Tab => {
            autocomplete_command(input);
            app.command_feedback = command_suggestion(input);
        }
        KeyCode::Up => history_up(app),
        KeyCode::Down => history_down(app),
        KeyCode::Char(ch) => {
            app.command_history_index = None;
            input.push(ch);
        }
        _ => {}
    }

    if let Some(input) = &app.command_input {
        app.command_feedback = command_suggestion(input);
    }

    Ok(false)
}

async fn submit_command(
    command: String,
    app: &mut App,
    writer: Option<&mut tokio::net::tcp::OwnedWriteHalf>,
) -> Result<()> {
    let trimmed = command.trim();
    if !trimmed.is_empty() {
        push_command_history(app, trimmed);
    }
    let mut parts = trimmed.splitn(4, ' ');
    let head = parts.next().unwrap_or_default();
    let verb = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let raw_value = parts.next().unwrap_or_default();

    let Some(writer) = writer else {
        app.last_error = Some("server unavailable while reconnecting".to_string());
        return Ok(());
    };

    if trimmed == "/help" {
        app.show_help = true;
        app.last_error = None;
        return Ok(());
    }

    if trimmed == "/sc show_params" {
        app.open_params();
        app.last_error = None;
        return Ok(());
    }

    if head == "/sc" && verb == "world_reset" {
        let seed = if path.is_empty() {
            None
        } else {
            Some(
                path.parse::<u64>()
                    .map_err(|_| anyhow::anyhow!("world_reset seed must be an unsigned integer"))?,
            )
        };
        send_message(writer, ClientMessage::WorldReset { seed }).await?;
        app.last_error = None;
        return Ok(());
    }

    if head != "/sc" || verb != "set" || path.is_empty() || raw_value.is_empty() {
        app.last_error = Some("expected: /help, /sc show_params, /sc world_reset [seed], or /sc set <path> <value>".to_string());
        return Ok(());
    }

    let value = parse_config_value(raw_value)?;
    send_message(
        writer,
        ClientMessage::ConfigSet {
            path: path.to_string(),
            value,
        },
    )
    .await?;
    app.last_error = None;
    Ok(())
}

fn push_command_history(app: &mut App, command: &str) {
    if app.command_history.last().is_some_and(|last| last == command) {
        return;
    }
    app.command_history.push(command.to_string());
    if app.command_history.len() > 50 {
        let extra = app.command_history.len() - 50;
        app.command_history.drain(0..extra);
    }
}

fn history_up(app: &mut App) {
    if app.command_history.is_empty() {
        return;
    }

    let next_index = match app.command_history_index {
        None => app.command_history.len().saturating_sub(1),
        Some(0) => 0,
        Some(index) => index.saturating_sub(1),
    };
    app.command_history_index = Some(next_index);
    if let Some(command) = app.command_history.get(next_index) {
        app.command_input = Some(command.clone());
    }
}

fn history_down(app: &mut App) {
    let Some(index) = app.command_history_index else {
        return;
    };

    if index + 1 >= app.command_history.len() {
        app.command_history_index = None;
        app.command_input = Some("/".to_string());
        return;
    }

    let next_index = index + 1;
    app.command_history_index = Some(next_index);
    if let Some(command) = app.command_history.get(next_index) {
        app.command_input = Some(command.clone());
    }
}

fn parse_config_value(raw: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Ok(value);
    }

    if let Ok(number) = raw.parse::<f64>() {
        return Ok(Value::from(number));
    }

    match raw {
        "true" => Ok(Value::from(true)),
        "false" => Ok(Value::from(false)),
        "null" => Ok(Value::Null),
        _ => Ok(Value::from(raw)),
    }
}

fn command_suggestion(input: &str) -> Option<String> {
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }

    let suggestions = [
        "/help",
        "/sc show_params",
        "/sc world_reset",
        "/sc world_reset 42",
        "/sc set soil.settle_frequency 0.01",
        "/sc set world.gen_params.soil.dirt_depth 8",
        "/sc set world.gen_params.ore.cluster_max 18",
        "/sc set world.gen_params.food.max_depth 50",
        "/sc set world.gen_params.stone_pockets.cluster_max 12",
        "/sc set world.snapshot_interval 5.0",
    ];

    let matches: Vec<_> = suggestions
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect();

    if matches.is_empty() {
        None
    } else {
        Some(matches.join("   "))
    }
}

fn autocomplete_command(input: &mut String) {
    let trimmed = input.trim_start();
    let suggestions = [
        "/help",
        "/sc show_params",
        "/sc world_reset",
        "/sc world_reset 42",
        "/sc set soil.settle_frequency 0.01",
        "/sc set world.gen_params.soil.dirt_depth 8",
        "/sc set world.gen_params.ore.cluster_max 18",
        "/sc set world.gen_params.food.max_depth 50",
        "/sc set world.gen_params.stone_pockets.cluster_max 12",
        "/sc set world.snapshot_interval 5.0",
    ];

    let matches: Vec<_> = suggestions
        .into_iter()
        .filter(|candidate| candidate.starts_with(trimmed))
        .collect();

    if matches.len() == 1 {
        *input = matches[0].to_string();
    }
}

fn default_action(app: &App, dir: MoveDir) -> Action {
    let Some(player) = app.player() else {
        return Action::Move(dir);
    };

    let (dx, dy) = dir.delta();
    let target = Position {
        x: player.pos.x + dx,
        y: player.pos.y + dy,
    };

    match app.snapshot.world.tile(target) {
        Some(Tile::Empty) => Action::Move(dir),
        Some(_) => Action::Dig(dir),
        None => Action::Move(dir),
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let constraints = if app.show_events {
        vec![
            Constraint::Length(4),
            Constraint::Min(8),
            Constraint::Length(7),
        ]
    } else {
        vec![Constraint::Length(4), Constraint::Min(8)]
    };
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    draw_status(frame, vertical[0], app);
    draw_world(frame, vertical[1], app);
    if app.show_events {
        draw_log(frame, vertical[2], app);
    }

    if !app.is_ready() {
        draw_sync_modal(frame, centered_rect(52, 28, area), app);
    }

    if app.show_params {
        draw_params_modal(frame, centered_rect(62, 62, area), app);
    }

    if app.show_help {
        draw_help_modal(frame, centered_rect(60, 55, area), app);
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let mut top = vec![Span::styled(
        "Antfarm vertical slice",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];

    if let Some(player) = app.player() {
        let dirt = player.inventory.get("dirt").copied().unwrap_or(0);
        let ore = player.inventory.get("ore").copied().unwrap_or(0);
        let stone = player.inventory.get("stone").copied().unwrap_or(0);
        let food = player.inventory.get("food").copied().unwrap_or(0);
        top.push(Span::raw(format!(
            "  ant={} dirt={} ore={} stone={} food={} pos=({}, {})",
            player.id, dirt, ore, stone, food, player.pos.x, player.pos.y
        )));
    }

    let mode = match app.pending_command {
        PendingCommand::None => None,
        PendingCommand::PlaceMaterial => Some("PLACE material"),
        PendingCommand::PlaceDirection(PlaceMaterial::Dirt) => Some("PLACE dirt"),
        PendingCommand::PlaceDirection(PlaceMaterial::Stone) => Some("PLACE stone"),
    };
    if let Some(label) = mode {
        top.push(Span::styled(
            format!("  mode={label}"),
            Style::default().fg(Color::LightCyan),
        ));
    }

    let settle = app
        .snapshot
        .config
        .get("soil")
        .and_then(|soil| soil.get("settle_frequency"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    top.push(Span::styled(
        format!("  settle={settle:.3}"),
        Style::default().fg(Color::LightYellow),
    ));

    let command_line = if let Some(input) = &app.command_input {
        let mut spans = vec![
            Span::styled("cmd ", Style::default().fg(Color::LightGreen)),
            Span::raw(input.clone()),
        ];
        if let Some(feedback) = &app.command_feedback {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                feedback.clone(),
                Style::default().fg(Color::LightBlue),
            ));
        }
        Line::from(spans)
    } else if let Some(feedback) = &app.command_feedback {
        Line::from(vec![Span::styled(
            feedback.clone(),
            Style::default().fg(Color::LightBlue),
        )])
    } else if let Some(error) = &app.last_error {
        Line::from(vec![Span::styled(
            format!("error {error}"),
            Style::default().fg(Color::Red),
        )])
    } else {
        Line::from("type / for server config commands")
    };

    let paragraph = Paragraph::new(vec![Line::from(top), command_line])
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(paragraph, area);
}

fn draw_world(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title("Colony");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(player) = app.player() else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let world_width = inner.width / 2;
    if world_width == 0 {
        return;
    }

    let viewport = Viewport::follow(player.pos, world_width, inner.height, &app.snapshot.world);
    let mut lines = Vec::with_capacity(inner.height as usize);

    for screen_y in 0..inner.height {
        let world_y = viewport.top + i32::from(screen_y);
        let mut spans = Vec::with_capacity(world_width as usize);

        for screen_x in 0..world_width {
            let world_x = viewport.left + i32::from(screen_x);
            let pos = Position {
                x: world_x,
                y: world_y,
            };
            spans.push(render_cell(app, pos));
        }

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_cell(app: &App, pos: Position) -> Span<'static> {
    if let Some(player) = app.snapshot.players.iter().find(|player| player.pos == pos) {
        let color = if player.id == app.player_id {
            Color::Green
        } else {
            Color::Cyan
        };
        let glyph = if player.id == app.player_id {
            animated_player_glyph(app)
        } else {
            "@@"
        };
        return Span::styled(
            glyph,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
    }

    if app.snapshot.npcs.iter().any(|npc| npc.pos == pos) {
        return Span::styled(
            "xx",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
    }

    let Some(tile) = app.snapshot.world.tile(pos) else {
        return Span::raw("  ");
    };
    match tile {
        Tile::Empty if pos.y == SURFACE_Y - 1 => {
            Span::styled("  ", Style::default().bg(Color::Rgb(20, 45, 20)))
        }
        Tile::Empty => Span::raw("  "),
        Tile::Dirt => Span::styled("▓▓", Style::default().fg(Color::Gray)),
        Tile::Stone => Span::styled("██", Style::default().fg(Color::White)),
        Tile::Resource => Span::styled("▒▒", Style::default().fg(Color::LightCyan)),
        Tile::Food => Span::styled("&&", Style::default().fg(Color::Green)),
        Tile::Bedrock => Span::styled("██", Style::default().fg(Color::DarkGray)),
    }
}

fn animated_player_glyph(app: &App) -> &'static str {
    match app.action_animation.map(|animation| animation.dir) {
        Some(MoveDir::Left) => "@ ",
        Some(MoveDir::Right) => " @",
        Some(MoveDir::Up) => "/\\",
        Some(MoveDir::Down) => "\\/",
        None => "@@",
    }
}

fn draw_log(frame: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<_> = app
        .snapshot
        .event_log
        .iter()
        .rev()
        .map(|entry| Line::from(entry.as_str()))
        .collect();

    let log = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Events"))
        .wrap(Wrap { trim: true });
    frame.render_widget(log, area);
}

fn draw_help_modal(frame: &mut Frame, area: Rect, app: &App) {
    let mode_line = match app.pending_command {
        PendingCommand::None => "Move with hjkl. Filled tiles auto-dig.",
        PendingCommand::PlaceMaterial => "Pending place: choose d for dirt or s for stone.",
        PendingCommand::PlaceDirection(PlaceMaterial::Dirt) => {
            "Pending place dirt: press h, j, k, or l."
        }
        PendingCommand::PlaceDirection(PlaceMaterial::Stone) => {
            "Pending place stone: press h, j, k, or l."
        }
    };

    let lines = vec![
        Line::from("hjkl: move or auto-dig"),
        Line::from("Space d h/j/k/l: place dirt"),
        Line::from("Space s h/j/k/l: place stone"),
        Line::from("/help"),
        Line::from("/sc show_params"),
        Line::from("/sc set soil.settle_frequency 0.01"),
        Line::from("/sc set world.max_depth -255"),
        Line::from("/sc set world.gen_params.soil.dirt_depth 8"),
        Line::from("/sc set world.gen_params.ore.cluster_max 18"),
        Line::from("/sc set world.gen_params.food.max_depth 50"),
        Line::from("/sc set world.gen_params.stone_pockets.cluster_max 12"),
        Line::from("/sc set world.snapshot_interval 5.0"),
        Line::from("/sc world_reset"),
        Line::from("/sc world_reset 42"),
        Line::from("Tab: autocomplete slash command"),
        Line::from("e: toggle event pane"),
        Line::from("? : toggle help"),
        Line::from("Esc: cancel place/slash/params modal"),
        Line::from("q: quit"),
        Line::from(""),
        Line::from(mode_line),
    ];

    frame.render_widget(Clear, area);
    let modal = Paragraph::new(lines)
        .block(Block::default().title("Controls").borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(modal, area);
}

fn draw_sync_modal(frame: &mut Frame, area: Rect, app: &App) {
    let (title, lines) = match app.sync_state {
        SyncState::Connecting => (
            "Connecting",
            vec![
                Line::from("Connecting to server..."),
                Line::from(""),
                Line::from("Waiting for TCP session and join handshake."),
                Line::from(""),
                Line::from("Press q to quit."),
            ],
        ),
        SyncState::Syncing {
            received_rows,
            total_rows,
        } => {
            let percent = if total_rows <= 0 {
                0.0
            } else {
                (received_rows as f64 / total_rows as f64 * 100.0).clamp(0.0, 100.0)
            };
            (
                "Syncing World",
                vec![
                    Line::from("Downloading world state..."),
                    Line::from(""),
                    Line::from(format!(
                        "received rows: {received_rows}/{total_rows} ({percent:.0}%)"
                    )),
                    Line::from("Applying chunked full sync before live patches."),
                    Line::from(""),
                    Line::from("Press q to quit."),
                ],
            )
        }
        SyncState::Reconnecting => (
            "Reconnecting",
            vec![
                Line::from("Server disconnected"),
                Line::from(""),
                Line::from(
                    app.last_error
                        .as_deref()
                        .unwrap_or("attempting to reconnect"),
                ),
                Line::from("Retrying automatically once per second."),
                Line::from(""),
                Line::from("Press q to quit."),
            ],
        ),
        SyncState::Ready => return,
    };

    frame.render_widget(Clear, area);
    let modal = Paragraph::new(lines)
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    frame.render_widget(modal, area);
}

fn draw_params_modal(frame: &mut Frame, area: Rect, app: &App) {
    let pretty = serde_json::to_string_pretty(&app.snapshot.config)
        .unwrap_or_else(|_| "{\"error\":\"failed to render config\"}".to_string());
    let mut lines: Vec<Line> = pretty.lines().map(Line::from).collect();
    lines.push(Line::from(""));
    lines.push(Line::from("j/k or arrows: scroll"));
    lines.push(Line::from("PgUp/PgDn: faster scroll"));
    lines.push(Line::from("Esc: close"));

    frame.render_widget(Clear, area);
    let modal = Paragraph::new(lines)
        .block(Block::default().title("Server Params").borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .scroll((app.params_scroll, 0));
    frame.render_widget(modal, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
