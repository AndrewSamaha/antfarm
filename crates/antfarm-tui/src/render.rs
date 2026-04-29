use crate::{
    app::{App, PendingCommand},
    modals::{centered_rect, draw_help_modal, draw_params_modal, draw_sync_modal},
};
use antfarm_core::{
    DAY_TICKS, MoveDir, NpcKind, PheromoneChannel, PlaceMaterial, Position, SURFACE_Y, Tile,
    Viewport,
    find_ascii_art_asset,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
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
        let queen = player.inventory.get("queen").copied().unwrap_or(0);
        let hive = player
            .hive_id
            .map(|hive_id| hive_id.to_string())
            .unwrap_or_else(|| "-".to_string());
        top.push(Span::raw(format!(
            "  ant={} hive={} dirt={} ore={} stone={} food={} queen={} pos=({}, {})",
            player.id, hive, dirt, ore, stone, food, queen, player.pos.x, player.pos.y
        )));
    }

    top.push(Span::styled(
        if app.snapshot.simulation_paused {
            "  sim=paused"
        } else {
            "  sim=running"
        },
        Style::default().fg(if app.snapshot.simulation_paused {
            Color::LightRed
        } else {
            Color::LightGreen
        }),
    ));
    top.push(Span::styled(
        format!(
            "  day={} tick={}",
            app.snapshot.tick / DAY_TICKS + 1,
            app.snapshot.tick
        ),
        Style::default().fg(Color::LightYellow),
    ));

    if let Some(channel) = app.pheromone_overlay {
        let label = match channel {
            PheromoneChannel::Home => "home",
            PheromoneChannel::Food => "food",
            PheromoneChannel::Threat => "threat",
            PheromoneChannel::Defense => "defense",
        };
        top.push(Span::styled(
            format!("  pheromone={label}"),
            Style::default().fg(Color::LightMagenta),
        ));
    }

    let mode = match app.pending_command {
        PendingCommand::None => None,
        PendingCommand::PlaceMaterial => Some("PLACE material"),
        PendingCommand::PlaceDirection(PlaceMaterial::Dirt) => Some("PLACE dirt"),
        PendingCommand::PlaceDirection(PlaceMaterial::Stone) => Some("PLACE stone"),
        PendingCommand::PlaceDirection(PlaceMaterial::Food) => Some("PLACE food"),
        PendingCommand::PlaceDirection(PlaceMaterial::Queen) => Some("PLACE queen"),
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

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let world_width = inner.width / 2;
    if world_width == 0 {
        return;
    }

    let viewport = Viewport::follow(
        app.focus_position(),
        world_width,
        inner.height,
        &app.snapshot.world,
    );
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
    if let Some(span) = render_npc_overlay_bar(app, pos) {
        return span;
    }

    if let Some(player) = app.snapshot.players.iter().find(|player| player.pos == pos) {
        let color = if player.id == app.player_id {
            Color::Green
        } else {
            Color::LightBlue
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

    if let Some(npc) = app.snapshot.npcs.iter().find(|npc| npc.pos == pos) {
        let color = npc_color(app, npc.hive_id, npc.kind);
        return match npc.kind {
            NpcKind::Worker => Span::styled(
                "xx",
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            NpcKind::Egg => Span::styled(
                "oo",
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            NpcKind::Queen => Span::raw("  "),
        };
    }

    if let Some(span) = render_preview_art_cell(app, pos) {
        return span;
    }

    if let Some(span) = render_placed_art_cell(app, pos) {
        return span;
    }

    let Some(tile) = app.snapshot.world.tile(pos) else {
        return Span::raw("  ");
    };
    let overlay_bg = pheromone_overlay_bg(app, pos);
    match tile {
        Tile::Empty if pos.y == SURFACE_Y - 1 => {
            Span::styled("  ", Style::default().bg(overlay_bg.unwrap_or(Color::Rgb(20, 45, 20))))
        }
        Tile::Empty => Span::styled("  ", Style::default().bg(overlay_bg.unwrap_or(Color::Reset))),
        Tile::Dirt => Span::styled("▓▓", base_tile_style(Color::Gray, overlay_bg)),
        Tile::Stone => Span::styled("██", base_tile_style(Color::White, overlay_bg)),
        Tile::Resource => Span::styled("▒▒", base_tile_style(Color::LightCyan, overlay_bg)),
        Tile::Food => Span::styled(
            "&&",
            base_tile_style(Color::Rgb(0, 128, 0), overlay_bg),
        ),
        Tile::Bedrock => Span::styled("██", base_tile_style(Color::DarkGray, overlay_bg)),
    }
}

fn base_tile_style(fg: Color, bg: Option<Color>) -> Style {
    let style = Style::default().fg(fg);
    if let Some(bg) = bg {
        style.bg(bg)
    } else {
        style
    }
}

fn render_npc_overlay_bar(app: &App, pos: Position) -> Option<Span<'static>> {
    if !app.show_npc_bars {
        return None;
    }

    let npc = app
        .snapshot
        .npcs
        .iter()
        .find(|npc| npc.pos.x == pos.x && npc.pos.y - 1 == pos.y)?;

    let health_char = bar_glyph(npc.health, npc.kind.max_health());
    let food_char = bar_glyph(npc.food, npc.kind.max_food());
    Some(Span::styled(
        format!("{health_char}{food_char}"),
        Style::default().fg(npc_color(app, npc.hive_id, npc.kind)),
    ))
}

fn bar_glyph(current: u16, max: u16) -> char {
    if max == 0 || current == 0 {
        return '.';
    }
    let ratio = current as f32 / max as f32;
    if ratio >= 0.80 {
        '█'
    } else if ratio >= 0.55 {
        '▓'
    } else if ratio >= 0.30 {
        '▒'
    } else {
        '░'
    }
}

fn render_placed_art_cell(app: &App, pos: Position) -> Option<Span<'static>> {
    for placed in &app.snapshot.placed_art {
        let Some(asset) = find_ascii_art_asset(&placed.asset_id) else {
            continue;
        };
        let local_x = pos.x - placed.pos.x;
        let local_y = pos.y - placed.pos.y;
        let rows = if asset.id == "queen_ant" {
            app.queen_rows_for_asset(asset, placed.pos)
        } else {
            asset.rows
        };
        let Some((left, right)) = asset.glyph_pair_at_world_in_rows(rows, local_x, local_y) else {
            continue;
        };
        return Some(Span::styled(
            format!("{left}{right}"),
            Style::default()
                .fg(npc_color(app, placed.hive_id, NpcKind::Queen))
                .add_modifier(Modifier::BOLD),
        ));
    }
    None
}

fn render_preview_art_cell(app: &App, pos: Position) -> Option<Span<'static>> {
    if !matches!(
        app.pending_command,
        PendingCommand::PlaceDirection(PlaceMaterial::Queen)
    ) {
        return None;
    }

    let player = app.player()?;
    let asset = find_ascii_art_asset("queen_ant")?;
    let origin = Position {
        x: player.pos.x - asset.world_anchor_x(),
        y: player.pos.y - asset.anchor_y,
    };
    let local_x = pos.x - origin.x;
    let local_y = pos.y - origin.y;
    let (left, right) = asset.glyph_pair_at_world(local_x, local_y)?;

    Some(Span::styled(
        format!("{left}{right}"),
        Style::default().fg(Color::DarkGray),
    ))
}

fn npc_color(app: &App, npc_hive_id: Option<u16>, kind: NpcKind) -> Color {
    if app
        .player()
        .and_then(|player| player.hive_id)
        .zip(npc_hive_id)
        .is_some_and(|(player_hive, npc_hive)| player_hive == npc_hive)
    {
        return Color::White;
    }

    match kind {
        NpcKind::Worker => Color::Red,
        NpcKind::Queen => Color::LightYellow,
        NpcKind::Egg => Color::LightMagenta,
    }
}

fn pheromone_overlay_bg(app: &App, pos: Position) -> Option<Color> {
    let map = app.pheromone_map.as_ref()?;
    if pos.x < 0 || pos.x >= map.width || pos.y < 0 || pos.y >= map.height {
        return None;
    }
    let value = map.values[(pos.y * map.width + pos.x) as usize];
    if value == 0 {
        return None;
    }
    let intensity = value.min(24);
    Some(match map.channel {
        PheromoneChannel::Home => Color::Rgb(18, 20 + intensity * 3, 48 + intensity * 5),
        PheromoneChannel::Food => Color::Rgb(20 + intensity * 4, 32 + intensity * 3, 20),
        PheromoneChannel::Threat => Color::Rgb(48 + intensity * 5, 20, 20),
        PheromoneChannel::Defense => Color::Rgb(22, 28, 42 + intensity * 5),
    })
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
