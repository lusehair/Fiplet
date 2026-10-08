//! Configuration for the light version: the Radio France open API token and
//! the last station, persisted in `settings.json`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// GraphQL endpoint of the Radio France open API. The token is sent in the
/// `x-token` header (never in the URL, which ends up in proxy/server logs).
pub const GRAPHQL_ENDPOINT: &str = "https://openapi.radiofrance.fr/v1/graphql";

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

/// Data directory of the light app (independent from the full version):
/// `~/Library/Application Support/fip-menubar-light`. Never falls back to the
/// current directory (which is `/` for an app bundle).
pub fn data_dir() -> PathBuf {
    let base = dirs::data_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Library/Application Support")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("fip-menubar-light");
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
    write_atomic(&settings_path(), &json)
}

/// Writes `bytes` to `path` atomically (temp file + rename, so a crash never
/// leaves a truncated file) and readable by the owner only (0600: the settings
/// hold the API token).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // Tighten an existing file created before this permission policy.
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    std::fs::rename(&tmp, path)
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
            eprintln!("settings: failed to save the station: {e}");
        }
    }
}

/// API token: settings > environment variable > none (empty string).
///
/// The key is optional: track info comes from the unauthenticated `livemeta`
/// API by default. The key only serves FIP Cultes (not on `livemeta`) and as
/// a fallback when `livemeta` fails.
pub fn api_token() -> String {
    let from_settings = load_settings().token;
    if !from_settings.trim().is_empty() {
        return from_settings.trim().to_string();
    }
    std::env::var("RADIOFRANCE_TOKEN")
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn settings_tolerate_missing_fields() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.token, "");
        assert_eq!(s.last_station, 0);
        let s: Settings = serde_json::from_str(r#"{"token":"abc"}"#).unwrap();
        assert_eq!(s.token, "abc");
    }

    #[test]
    fn write_atomic_is_private_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("fiplet-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        write_atomic(&path, b"{\"token\":\"x\"}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"token\":\"x\"}");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(!path.with_extension("tmp").exists());
        // Overwrite keeps the file private and replaces the content wholesale.
        write_atomic(&path, b"{}").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"{}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
