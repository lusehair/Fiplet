//! Poller thread: queries Radio France's `live` GraphQL API for the currently
//! playing station only. Polling is aligned on `song.end` (≈ 1 request per track).

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config;
use crate::model::{AppEvent, NowPlaying};
use crate::stations::STATIONS;

const FALLBACK_POLL: Duration = Duration::from_secs(25);
const END_MARGIN: Duration = Duration::from_secs(2);

/// Resolved track + track end time (epoch seconds), or a network error.
type LiveResult = Result<Option<(NowPlaying, Option<i64>)>, Box<dyn std::error::Error>>;

/// Commands to the poller.
pub enum MetaCmd {
    /// Change the followed station and (re)start polling.
    Switch(usize),
    /// Suspend polling (playback paused).
    Pause,
}

/// Starts the metadata thread following the `initial` station.
pub fn spawn(events: Sender<AppEvent>, initial: usize) -> Sender<MetaCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("metadata".into())
        .spawn(move || run(events, initial, rx))
        .expect("démarrage du thread métadonnées");
    tx
}

fn run(events: Sender<AppEvent>, initial: usize, rx: mpsc::Receiver<MetaCmd>) {
    let mut station = initial;
    let mut last_key: Option<(String, String)> = None;
    // Starts paused: track display is only enabled by the UI (via Switch) once
    // the user has entered the API key.
    let mut paused = true;

    loop {
        if paused {
            match rx.recv() {
                Ok(MetaCmd::Switch(s)) => {
                    station = s;
                    last_key = None;
                    paused = false;
                }
                Ok(MetaCmd::Pause) => {}
                Err(_) => return,
            }
            continue;
        }

        let wait = match fetch_live(station) {
            Ok(Some((np, end))) => {
                let key = np.dedup_key();
                let is_new = last_key.as_ref() != Some(&key);
                if is_new {
                    last_key = Some(key);
                    let events2 = events.clone();
                    let title = np.title.clone();
                    let artist = np.artist.clone();
                    let si = np.station_index;
                    std::thread::spawn(move || {
                        if let Some(jpeg) = fetch_cover(&artist, &title) {
                            let _ = events2.send(AppEvent::Cover { station_index: si, jpeg });
                        }
                    });
                }
                let _ = events.send(AppEvent::NowPlaying(np));
                wait_until_end(end)
            }
            Ok(None) => {
                last_key = None;
                let _ = events.send(AppEvent::MetadataCleared {
                    station_index: station,
                });
                FALLBACK_POLL
            }
            Err(e) => {
                eprintln!("metadata: {e}");
                FALLBACK_POLL
            }
        };

        match rx.recv_timeout(wait) {
            Ok(MetaCmd::Switch(s)) => {
                station = s;
                last_key = None;
            }
            Ok(MetaCmd::Pause) => paused = true,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn wait_until_end(end: Option<i64>) -> Duration {
    match end {
        Some(end) => {
            let now = now_secs();
            if end > now {
                Duration::from_secs((end - now) as u64) + END_MARGIN
            } else {
                FALLBACK_POLL
            }
        }
        None => FALLBACK_POLL,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct GqlResp {
    data: Option<GqlData>,
}
#[derive(Deserialize)]
struct GqlData {
    live: Option<Live>,
}
#[derive(Deserialize)]
struct Live {
    song: Option<SongStep>,
}
#[derive(Deserialize)]
struct SongStep {
    end: Option<i64>,
    track: Option<Track>,
}
#[derive(Deserialize)]
struct Track {
    title: String,
    #[serde(rename = "mainArtists")]
    main_artists: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ItunesResp {
    results: Vec<ItunesResult>,
}

#[derive(Deserialize)]
struct ItunesResult {
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
}

/// HTTP agent for the metadata/cover requests, with sane timeouts so a stalled
/// connection can never freeze the poller. NOT used for the audio stream (which
/// must stay open indefinitely — see `audio.rs`).
fn http() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(8)))
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .new_agent()
    })
}

/// Strips qualifiers that hurt catalog matching — bracketed/parenthesized parts
/// (`(Remaster)`, `[Live]`) and "feat." credits — so the iTunes search hits the
/// canonical track more often.
fn clean_term(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth: i32 = 0;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // `to_ascii_lowercase` keeps byte length, so positions map back onto `out`.
    let lower = out.to_ascii_lowercase();
    for marker in [" feat.", " feat ", " ft.", " ft ", " featuring "] {
        if let Some(pos) = lower.find(marker) {
            out.truncate(pos);
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fetch_cover(artist: &str, title: &str) -> Option<Vec<u8>> {
    let term = format!("{} {}", clean_term(artist), clean_term(title));
    let mut resp = http()
        .get("https://itunes.apple.com/search")
        .query("term", &term)
        .query("media", "music")
        .query("entity", "song")
        .query("limit", "1")
        .call()
        .ok()?;
    let parsed: ItunesResp = resp.body_mut().read_json().ok()?;
    let url = parsed
        .results
        .into_iter()
        .next()?
        .artwork_url_100?
        .replace("100x100bb", "600x600bb");
    let mut img = http().get(&url).call().ok()?;
    img.body_mut().read_to_vec().ok()
}

fn fetch_live(station: usize) -> LiveResult {
    if config::api_token().trim().is_empty() {
        return Ok(None);
    }
    let api = STATIONS[station].api_station;
    let query = format!(
        "{{ live(station: {api}) {{ song {{ end track {{ title mainArtists }} }} }} }}"
    );

    let mut resp = http()
        .post(config::graphql_endpoint())
        .send_json(serde_json::json!({ "query": query }))?;
    let parsed: GqlResp = resp.body_mut().read_json()?;

    let Some(song) = parsed.data.and_then(|d| d.live).and_then(|l| l.song) else {
        return Ok(None);
    };
    let Some(track) = song.track else {
        return Ok(None);
    };

    let np = NowPlaying {
        title: track.title,
        artist: track.main_artists.unwrap_or_default().join(", "),
        station_index: station,
    };
    Ok(Some((np, song.end)))
}
