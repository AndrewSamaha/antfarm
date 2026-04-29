use crate::{
    app::{ActionAnimation, App, PendingCommand},
    commands::{command_suggestion, handle_command_input},
    network::{send_action, send_message},
};
use anyhow::Result;
use antfarm_core::{Action, ClientMessage, MoveDir, PheromoneChannel, PlaceMaterial, Position, Tile};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use std::time::{Duration, Instant};

pub(crate) async fn handle_event(
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
        if app.is_selecting_server() {
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('?') => app.show_help = !app.show_help,
                KeyCode::Char('j') | KeyCode::Down => app.move_server_selection(1),
                KeyCode::Char('k') | KeyCode::Up => app.move_server_selection(-1),
                KeyCode::Enter => {
                    if !app.choose_selected_server() {
                        app.set_error("no server selected");
                    }
                }
                _ => {}
            }
            return Ok(false);
        }
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
        if writer.is_none() && app.is_replay() && app.player().is_none() {
            app.pan_camera(dir);
            return Ok(false);
        }
        let action = match app.pending_command {
            PendingCommand::None => default_action(app, dir),
            PendingCommand::PlaceMaterial => {
                app.pending_command = PendingCommand::None;
                default_action(app, dir)
            }
            PendingCommand::PlaceDirection(material) => {
                app.pending_command = PendingCommand::None;
                match material {
                    PlaceMaterial::Queen => Action::PlaceQueen,
                    _ => Action::Place { dir, material },
                }
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
        KeyCode::Char('q') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Queen);
            app.clear_status();
        }
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
        KeyCode::Tab => app.show_npc_bars = !app.show_npc_bars,
        KeyCode::Char('p') => {
            if app.is_replay() && writer.is_none() {
                app.toggle_local_pause();
            } else if let Some(writer) = writer {
                send_message(
                    writer,
                    ClientMessage::SetSimulationPaused {
                        paused: !app.snapshot.simulation_paused,
                    },
                )
                .await?;
            }
        }
        KeyCode::Char('o') => {
            app.pheromone_overlay = match app.pheromone_overlay {
                None => Some(PheromoneChannel::Home),
                Some(PheromoneChannel::Home) => Some(PheromoneChannel::Food),
                Some(PheromoneChannel::Food) => None,
                Some(PheromoneChannel::Threat | PheromoneChannel::Defense) => None,
            };
            if app.pheromone_overlay.is_none() {
                app.pheromone_map = None;
            }
        }
        KeyCode::Char('/') => {
            app.command_input = Some("/".to_string());
            app.command_feedback = command_suggestion("/");
            app.command_history_index = None;
            app.pending_command = PendingCommand::None;
            app.clear_status();
        }
        KeyCode::Char(' ') => app.pending_command = PendingCommand::PlaceMaterial,
        KeyCode::Char('d') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Dirt);
            app.clear_status();
        }
        KeyCode::Char('s') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Stone);
            app.clear_status();
        }
        KeyCode::Char('f') if matches!(app.pending_command, PendingCommand::PlaceMaterial) => {
            app.pending_command = PendingCommand::PlaceDirection(PlaceMaterial::Food);
            app.clear_status();
        }
        _ => {}
    }

    Ok(false)
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
