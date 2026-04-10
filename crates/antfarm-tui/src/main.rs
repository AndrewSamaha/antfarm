use antfarm_core::{
    Action, ClientMessage, MoveDir, Player, Position, SURFACE_Y, ServerMessage, Snapshot, Tile,
    Viewport,
};
use anyhow::{Context, Result};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::{env, io, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc,
    time,
};

#[derive(Debug)]
struct App {
    player_id: u8,
    snapshot: Snapshot,
    show_help: bool,
    pending_action: Option<PendingAction>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum PendingAction {
    Dig,
    Place,
}

impl App {
    fn new(player_id: u8, snapshot: Snapshot) -> Self {
        Self {
            player_id,
            snapshot,
            show_help: true,
            pending_action: None,
            last_error: None,
        }
    }

    fn player(&self) -> Option<&Player> {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == self.player_id)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let name = env::args()
        .nth(1)
        .unwrap_or_else(|| "worker-ant".to_string());
    let stream = TcpStream::connect("127.0.0.1:7000")
        .await
        .context("connect to antfarm-server on 127.0.0.1:7000")?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    let join = serde_json::to_string(&ClientMessage::Join { name })?;
    writer.write_all(join.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    let joined = loop {
        let Some(line) = lines.next_line().await? else {
            anyhow::bail!("server closed before join completed");
        };
        match serde_json::from_str::<ServerMessage>(&line)? {
            ServerMessage::Joined(joined) => break joined,
            ServerMessage::Error { message } => anyhow::bail!(message),
            ServerMessage::Snapshot(_) => {}
        }
    };

    let (network_tx, mut network_rx) = mpsc::unbounded_channel::<ServerMessage>();
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
                }
            }
        }
    });

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let terminal = ratatui::init();

    let result = run_app(
        terminal,
        &mut writer,
        joined.player_id,
        joined.snapshot,
        &mut network_rx,
    )
    .await;

    ratatui::restore();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

async fn run_app(
    mut terminal: DefaultTerminal,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    player_id: u8,
    snapshot: Snapshot,
    network_rx: &mut mpsc::UnboundedReceiver<ServerMessage>,
) -> Result<()> {
    let mut app = App::new(player_id, snapshot);
    let mut events = EventStream::new();
    let mut redraw = time::interval(Duration::from_millis(33));

    loop {
        terminal.draw(|frame| draw(frame, &app))?;

        tokio::select! {
            _ = redraw.tick() => {}
            maybe_message = network_rx.recv() => {
                match maybe_message {
                    Some(ServerMessage::Snapshot(snapshot)) => app.snapshot = snapshot,
                    Some(ServerMessage::Error { message }) => app.last_error = Some(message),
                    Some(ServerMessage::Joined(_)) => {}
                    None => anyhow::bail!("lost connection to server"),
                }
            }
            maybe_event = tokio_stream_event(&mut events) => {
                if let Some(event) = maybe_event? {
                    if handle_event(event, &mut app, writer).await? {
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn tokio_stream_event(events: &mut EventStream) -> Result<Option<Event>> {
    use futures_util::StreamExt;

    Ok(events.next().await.transpose()?)
}

async fn handle_event(
    event: Event,
    app: &mut App,
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
) -> Result<bool> {
    let Event::Key(key) = event else {
        return Ok(false);
    };

    if key.kind != KeyEventKind::Press {
        return Ok(false);
    }

    let direction = match key.code {
        KeyCode::Up | KeyCode::Char('k') => Some(MoveDir::Up),
        KeyCode::Down | KeyCode::Char('j') => Some(MoveDir::Down),
        KeyCode::Left | KeyCode::Char('h') => Some(MoveDir::Left),
        KeyCode::Right | KeyCode::Char('l') => Some(MoveDir::Right),
        _ => None,
    };

    if let Some(dir) = direction {
        let action = match app.pending_action.take() {
            Some(PendingAction::Dig) => Action::Dig(dir),
            Some(PendingAction::Place) => Action::Place(dir),
            None => default_action(app, dir),
        };
        send_action(writer, action).await?;
        return Ok(false);
    }

    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Esc => app.pending_action = None,
        KeyCode::Char('?') => app.show_help = !app.show_help,
        KeyCode::Char('d') => app.pending_action = Some(PendingAction::Dig),
        KeyCode::Char('p') => app.pending_action = Some(PendingAction::Place),
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
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(7),
        ])
        .split(area);

    draw_status(frame, vertical[0], app);
    draw_world(frame, vertical[1], app);
    draw_log(frame, vertical[2], app);

    if app.show_help {
        draw_help_modal(frame, centered_rect(60, 55, area), app);
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(
        "Antfarm vertical slice",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];

    if let Some(player) = app.player() {
        spans.push(Span::raw(format!(
            "  ant={} dirt={} ore={} pos=({}, {})",
            player.id, player.dirt, player.resources, player.pos.x, player.pos.y
        )));
    }

    if let Some(mode) = app.pending_action {
        let label = match mode {
            PendingAction::Dig => "DIG",
            PendingAction::Place => "PLACE",
        };
        spans.push(Span::styled(
            format!("  mode={label}"),
            Style::default().fg(Color::LightCyan),
        ));
    }

    if let Some(error) = &app.last_error {
        spans.push(Span::styled(
            format!("  error={error}"),
            Style::default().fg(Color::Red),
        ));
    }

    let paragraph = Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
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
        return Span::styled(
            "@@",
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
    let mode_line = match app.pending_action {
        Some(PendingAction::Dig) => "Pending dig: press a direction.",
        Some(PendingAction::Place) => "Pending place: press a direction.",
        None => "Move with arrows or hjkl.",
    };

    let lines = vec![
        Line::from("Arrows / hjkl: move or auto-dig"),
        Line::from("d then direction: force a dig action"),
        Line::from("p then direction: place dirt"),
        Line::from("? : toggle help"),
        Line::from("Esc: cancel dig/place"),
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
