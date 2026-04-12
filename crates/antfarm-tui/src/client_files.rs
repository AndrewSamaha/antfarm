use anyhow::{Context, Result};
use rand::{Rng, rng};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClientConfig {
    pub(crate) token: String,
    #[serde(default = "default_show_help_at_startup")]
    pub(crate) show_help_at_startup: bool,
    #[serde(default = "default_max_history")]
    pub(crate) max_history: usize,
}

fn default_show_help_at_startup() -> bool {
    true
}

fn default_max_history() -> usize {
    100
}

pub(crate) fn load_or_create_client_config(player_name: &str) -> Result<ClientConfig> {
    let path = client_config_path(player_name);
    if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("read client config at {}", path.display()))?;
        let mut config: ClientConfig =
            toml::from_str(&content).context("parse client config TOML")?;
        if config.token.trim().is_empty() {
            config.token = generate_client_token();
            save_client_config(player_name, &config)?;
        }
        return Ok(config);
    }

    let config = ClientConfig {
        token: generate_client_token(),
        show_help_at_startup: true,
        max_history: default_max_history(),
    };
    save_client_config(player_name, &config)?;
    Ok(config)
}

pub(crate) fn save_client_config(player_name: &str, config: &ClientConfig) -> Result<()> {
    let path = client_config_path(player_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create client config dir {}", parent.display()))?;
    }
    let content = toml::to_string_pretty(config).context("serialize client config TOML")?;
    fs::write(&path, content).with_context(|| format!("write client config {}", path.display()))?;
    Ok(())
}

pub(crate) fn load_command_history(player_name: &str, max_history: usize) -> Result<Vec<String>> {
    let path = client_history_path(player_name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("read client history at {}", path.display()))?;
    let mut entries: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if entries.len() > max_history {
        let extra = entries.len() - max_history;
        entries.drain(0..extra);
    }
    Ok(entries)
}

pub(crate) fn save_command_history(
    player_name: &str,
    history: &[String],
    max_history: usize,
) -> Result<()> {
    let path = client_history_path(player_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create client history dir {}", parent.display()))?;
    }
    let start = history.len().saturating_sub(max_history);
    let content = history[start..].join("\n");
    fs::write(&path, content).with_context(|| format!("write client history {}", path.display()))?;
    Ok(())
}

fn client_config_path(player_name: &str) -> PathBuf {
    let slug = sanitize_player_name(player_name);
    Path::new("data")
        .join("clients")
        .join(format!("{slug}.toml"))
}

fn client_history_path(player_name: &str) -> PathBuf {
    let slug = sanitize_player_name(player_name);
    Path::new("data")
        .join("clients")
        .join(format!("{slug}.history"))
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
