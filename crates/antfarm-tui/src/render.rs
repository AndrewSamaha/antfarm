use crate::app::{App, PendingCommand, SyncState};
use antfarm_core::{MoveDir, PlaceMaterial, Position, SURFACE_Y, Tile, Viewport};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde_json::Value;

pub(crate) fn draw(frame: &mut Frame, app: &App) {
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
    } else if let Some(info) = &app.last_info {
        Line::from(vec![Span::styled(
            info.clone(),
            Style::default().fg(Color::LightGreen),
        )])
    } else {
        Line::from("type / for client or server config commands")
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
        Line::from("/cc set show_help_at_startup false"),
        Line::from("/cc set max_history 100"),
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
