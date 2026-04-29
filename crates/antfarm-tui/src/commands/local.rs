use crate::{
    app::App,
    client_files::{load_or_create_client_config, save_client_config, save_command_history},
};
use anyhow::Result;

pub(super) async fn submit_local_command(trimmed: &str, app: &mut App) -> Result<()> {
    let mut parts = trimmed.splitn(4, ' ');
    let head = parts.next().unwrap_or_default();
    let verb = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let raw_value = parts.next().unwrap_or_default();

    if head != "/cc" || verb != "set" || path.is_empty() || raw_value.is_empty() {
        app.set_error(
            "expected: /cc set show_help_at_startup true|false or /cc set max_history <n>",
        );
        return Ok(());
    }

    if !app.persist_client_files {
        match path {
            "show_help_at_startup" => {
                let show_help_at_startup = match raw_value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        app.set_error("expected: /cc set show_help_at_startup true|false");
                        return Ok(());
                    }
                };
                app.set_info(format!(
                    "dev mode client config updated in memory: show_help_at_startup={show_help_at_startup}"
                ));
            }
            "max_history" => {
                let max_history = raw_value
                    .parse::<usize>()
                    .map_err(|_| anyhow::anyhow!("max_history must be a positive integer"))?;
                if max_history == 0 {
                    app.set_error("max_history must be at least 1");
                    return Ok(());
                }
                app.max_history = max_history;
                if app.command_history.len() > app.max_history {
                    let extra = app.command_history.len() - app.max_history;
                    app.command_history.drain(0..extra);
                }
                app.set_info(format!(
                    "dev mode client config updated in memory: max_history={max_history}"
                ));
            }
            _ => {
                app.set_error(
                    "expected: /cc set show_help_at_startup true|false or /cc set max_history <n>",
                );
            }
        }
        return Ok(());
    }

    let mut client_config = load_or_create_client_config(&app.player_name)?;
    match path {
        "show_help_at_startup" => {
            let show_help_at_startup = match raw_value {
                "true" => true,
                "false" => false,
                _ => {
                    app.set_error("expected: /cc set show_help_at_startup true|false");
                    return Ok(());
                }
            };
            client_config.show_help_at_startup = show_help_at_startup;
            save_client_config(&app.player_name, &client_config)?;
            app.set_info(format!(
                "client config updated: show_help_at_startup={show_help_at_startup}"
            ));
        }
        "max_history" => {
            let max_history = raw_value
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("max_history must be a positive integer"))?;
            if max_history == 0 {
                app.set_error("max_history must be at least 1");
                return Ok(());
            }
            client_config.max_history = max_history;
            save_client_config(&app.player_name, &client_config)?;
            app.max_history = max_history;
            if app.command_history.len() > app.max_history {
                let extra = app.command_history.len() - app.max_history;
                app.command_history.drain(0..extra);
            }
            save_command_history(&app.player_name, &app.command_history, app.max_history)?;
            app.set_info(format!("client config updated: max_history={max_history}"));
        }
        _ => {
            app.set_error(
                "expected: /cc set show_help_at_startup true|false or /cc set max_history <n>",
            );
        }
    }

    Ok(())
}
