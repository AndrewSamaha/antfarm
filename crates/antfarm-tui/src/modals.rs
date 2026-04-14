use crate::app::{App, PendingCommand, SyncState};
use antfarm_core::PlaceMaterial;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

pub(crate) fn draw_help_modal(frame: &mut Frame, area: Rect, app: &App) {
    let mode_line = match app.pending_command {
        PendingCommand::None => "Move with hjkl. Filled tiles auto-dig.",
        PendingCommand::PlaceMaterial => {
            "Pending place: choose d for dirt, s for stone, f for food, or q for queen."
        }
        PendingCommand::PlaceDirection(PlaceMaterial::Dirt) => {
            "Pending place dirt: press h, j, k, or l."
        }
        PendingCommand::PlaceDirection(PlaceMaterial::Stone) => {
            "Pending place stone: press h, j, k, or l."
        }
        PendingCommand::PlaceDirection(PlaceMaterial::Food) => {
            "Pending place food: press h, j, k, or l."
        }
        PendingCommand::PlaceDirection(PlaceMaterial::Queen) => {
            "Queen preview active: h, j, k, or l attempts placement."
        }
    };

    let lines = vec![
        Line::from("hjkl: move or auto-dig"),
        Line::from("Space d h/j/k/l: place dirt"),
        Line::from("Space s h/j/k/l: place stone"),
        Line::from("Space f h/j/k/l: place food"),
        Line::from("Space q, then h/j/k/l: preview and place queen"),
        Line::from("Tab: toggle NPC health/food bars"),
        Line::from("/help"),
        Line::from("/cc set show_help_at_startup false"),
        Line::from("/cc set max_history 100"),
        Line::from("/sc show_params"),
        Line::from("/sc give all q 1"),
        Line::from("/sc set soil.settle_frequency 0.01"),
        Line::from("/sc set world.max_depth -255"),
        Line::from("/sc set world.gen_params.soil.dirt_depth 8"),
        Line::from("/sc set world.gen_params.ore.cluster_max 18"),
        Line::from("/sc set world.gen_params.food.max_depth 50"),
        Line::from("/sc set world.gen_params.stone_pockets.cluster_max 12"),
        Line::from("/sc set world.snapshot_interval 5.0"),
        Line::from("/sc world_reset"),
        Line::from("/sc world_reset 42"),
        Line::from("Tab: autocomplete slash command when in / command mode"),
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

pub(crate) fn draw_sync_modal(frame: &mut Frame, area: Rect, app: &App) {
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

pub(crate) fn draw_params_modal(frame: &mut Frame, area: Rect, app: &App) {
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

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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
