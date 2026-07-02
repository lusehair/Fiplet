//! "Like" system (light version): stores liked tracks in a JSON file.
//! Unlike the full version, there are no downloads: the list is used only for
//! display (the "Favorites" submenu).

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config;
use crate::model::NowPlaying;
use crate::stations::STATIONS;

/// A liked entry, as serialized in `likes.json`.
#[derive(Serialize, Deserialize, Clone)]
pub struct Like {
    pub title: String,
    pub artist: String,
    pub station: String,
    /// Like date (Unix timestamp, in seconds).
    pub liked_at_unix: i64,
}

impl Like {
    /// Case-insensitive deduplication key (artist, title).
    pub fn key(&self) -> (String, String) {
        (self.artist.to_lowercase(), self.title.to_lowercase())
    }
}

/// Loads all likes (empty list if missing or unreadable).
pub fn all() -> Vec<Like> {
    std::fs::read(config::likes_json_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// True if this track is already liked (deduplication on (artist, title)).
pub fn contains(np: &NowPlaying) -> bool {
    let key = np.dedup_key();
    all().iter().any(|l| l.key() == key)
}

/// Adds the current track to the likes. Returns the `Like` created if it was
/// added, `None` if it was already present or if the title is empty.
pub fn add(np: &NowPlaying) -> std::io::Result<Option<Like>> {
    if np.title.is_empty() {
        return Ok(None);
    }
    let mut likes = all();
    let key = np.dedup_key();
    if likes.iter().any(|l| l.key() == key) {
        return Ok(None);
    }

    let like = Like {
        title: np.title.clone(),
        artist: np.artist.clone(),
        station: STATIONS
            .get(np.station_index)
            .map(|s| s.name.to_string())
            .unwrap_or_default(),
        liked_at_unix: now_secs(),
    };
    likes.push(like.clone());
    save(&likes)?;
    Ok(Some(like))
}

/// Removes the liked track matching `key` (case-insensitive artist/title).
/// Returns true if something was removed.
pub fn remove_by_key(key: &(String, String)) -> bool {
    let mut likes = all();
    let before = likes.len();
    likes.retain(|l| &l.key() != key);
    if likes.len() != before {
        if let Err(e) = save(&likes) {
            eprintln!("like: échec de suppression : {e}");
            return false;
        }
        true
    } else {
        false
    }
}

fn save(likes: &[Like]) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(likes)?;
    std::fs::write(config::likes_json_path(), json)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
