//! Configuration for the light version: only the Radio France open API token,
//! persisted in `settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Settings persisted in `settings.json`.
#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub token: String,
    /// Index (in `stations::STATIONS`) of the last station the user selected,
    /// restored on the next launch. Defaults to 0 (FIP).
    #[serde(default)]
    pub last_station: usize,
}

/// Data directory of the light app (independent from the full version).
pub fn data_dir() -> PathBuf {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fip-menubar-light");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

/// Path to the JSON file of liked tracks.
pub fn likes_json_path() -> PathBuf {
    data_dir().join("likes.json")
}

/// True on the very first launch (as long as `settings.json` does not exist).
/// Used to open the Settings window (API key entry) only once: as soon as the
/// user saves, the file exists and the window no longer reopens.
pub fn needs_onboarding() -> bool {
    !settings_path().exists()
}

pub fn load_settings() -> Settings {
    std::fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &Settings) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(settings)?;
    std::fs::write(settings_path(), json)
}

/// Index of the last station the user selected (0/FIP if unset).
pub fn last_station() -> usize {
    load_settings().last_station
}

/// Persists the last selected station, preserving the rest of the settings
/// (notably the API token). No-op if unchanged.
pub fn save_last_station(index: usize) {
    let mut settings = load_settings();
    if settings.last_station != index {
        settings.last_station = index;
        if let Err(e) = save_settings(&settings) {
            eprintln!("paramètres: échec d'écriture de la station : {e}");
        }
    }
}

/// True if the user has entered their own API key in the settings.
/// Until then, the light version does not fetch track info (it does not use the
/// built-in default token).
pub fn has_user_token() -> bool {
    !load_settings().token.trim().is_empty()
}

/// API token: settings > environment variable > default.
pub fn api_token() -> String {
    let from_settings = load_settings().token;
    if !from_settings.trim().is_empty() {
        return from_settings;
    }
    std::env::var("RADIOFRANCE_TOKEN").unwrap_or_default()
}

/// GraphQL endpoint URL, token included.
pub fn graphql_endpoint() -> String {
    format!(
        "https://openapi.radiofrance.fr/v1/graphql?x-token={}",
        api_token()
    )
}
