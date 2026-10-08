//! Poller thread: resolves the track currently playing on the followed station.
//! Polling is aligned on the announced track end (≈ 1 request per track).
//!
//! Two sources, in order:
//! 1. Radio France's unauthenticated `livemeta` API — the one the radiofrance.fr
//!    player uses (`PUBLIC_LIVE_API_URL` in the site's config). No key needed;
//!    also provides the cover art (resolved through the site's image service).
//! 2. The Radio France open API (GraphQL, user key): used for FIP Cultes, which
//!    `livemeta` does not serve, and as a fallback whenever `livemeta` fails.

use std::fmt;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config;
use crate::model::{AppEvent, NowPlaying};
use crate::stations::STATIONS;

/// Poll interval when the API gives no usable `end` (spoken program, error).
const FALLBACK_POLL: Duration = Duration::from_secs(25);
/// Poll interval after the API rejected the request (bad key): no point
/// hammering it, but keep checking in case the key gets fixed server-side.
const API_ERROR_POLL: Duration = Duration::from_secs(60);
/// Margin added after the announced track end before re-polling.
const END_MARGIN: Duration = Duration::from_secs(2);
/// Upper bound on the wait between two polls: `song.end` can be far off (long
/// show, clock skew, bad data) and FIP regularly cuts tracks before their
/// announced end, so never sleep for hours trusting it.
const MAX_WAIT: Duration = Duration::from_secs(5 * 60);

/// Resolved track + track end time (epoch seconds), or an error.
type LiveResult = Result<Option<(NowPlaying, Option<i64>)>, LiveError>;

/// Unauthenticated now-playing API used by the radiofrance.fr player.
const LIVEMETA_URL: &str = "https://api.radiofrance.fr/livemeta/live";
/// Image service of radiofrance.fr: `/<uuid>/<WxH>` serves a JPEG.
const PIKAPI_IMAGES_URL: &str = "https://www.radiofrance.fr/pikapi/images";
const COVER_SIZE: &str = "600x600";

/// Why a `live` query failed.
#[derive(Debug)]
enum LiveError {
    /// The API rejected the key (HTTP 401/403).
    Unauthorized(u16),
    /// The API answered with GraphQL `errors`.
    Graphql(String),
    /// Network / parsing failure.
    Other(Box<dyn std::error::Error>),
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveError::Unauthorized(code) => write!(f, "API key rejected (HTTP {code})"),
            LiveError::Graphql(m) => write!(f, "erreur GraphQL : {m}"),
            LiveError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<ureq::Error> for LiveError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(c @ (401 | 403)) => LiveError::Unauthorized(c),
            other => LiveError::Other(Box::new(other)),
        }
    }
}

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
        .expect("failed to spawn the metadata thread");
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
                    last_key = Some(key.clone());
                    let events2 = events.clone();
                    let title = np.title.clone();
                    let artist = np.artist.clone();
                    let cover_url = np.cover_url.clone();
                    let si = np.station_index;
                    std::thread::spawn(move || {
                        let jpeg = cover_url
                            .as_deref()
                            .and_then(fetch_bytes)
                            .or_else(|| fetch_cover(&artist, &title));
                        if let Some(jpeg) = jpeg {
                            let _ = events2.send(AppEvent::Cover { station_index: si, key, jpeg });
                        }
                    });
                }
                let _ = events.send(AppEvent::NowPlaying(np));
                poll_delay(end, now_secs())
            }
            Ok(None) => {
                last_key = None;
                let _ = events.send(AppEvent::MetadataCleared {
                    station_index: station,
                });
                FALLBACK_POLL
            }
            Err(e @ (LiveError::Unauthorized(_) | LiveError::Graphql(_))) => {
                eprintln!("metadata: {e}");
                last_key = None;
                let _ = events.send(AppEvent::ApiError {
                    unauthorized: matches!(e, LiveError::Unauthorized(_)),
                    detail: e.to_string(),
                });
                API_ERROR_POLL
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

/// How long to wait before the next poll, given the announced track end
/// (epoch seconds) and the current time. Bounded by `MAX_WAIT`.
fn poll_delay(end: Option<i64>, now: i64) -> Duration {
    match end {
        Some(end) if end > now => {
            (Duration::from_secs((end - now) as u64) + END_MARGIN).min(MAX_WAIT)
        }
        _ => FALLBACK_POLL,
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
    #[serde(default)]
    errors: Vec<GqlError>,
}
#[derive(Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
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
    /// May be `null` for some steps: must not fail the whole response.
    title: Option<String>,
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

/// `livemeta/live/<id>/fip_player` response (only what we use).
#[derive(Deserialize)]
struct LivemetaResp {
    now: Option<LivemetaNow>,
}

/// For a song: `secondLine` = title, `thirdLine` = artist, `secondLineSongUuid`
/// set. For a programme / "Le direct": `secondLine` = label, `thirdLine` null,
/// no song uuid.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LivemetaNow {
    second_line: Option<String>,
    third_line: Option<String>,
    second_line_song_uuid: Option<String>,
    cover: Option<String>,
    end_time: Option<i64>,
}

impl LivemetaNow {
    /// The playing song, or `None` for a programme / idle slot.
    fn into_now_playing(self, station: usize) -> Option<(NowPlaying, Option<i64>)> {
        self.second_line_song_uuid.as_ref()?;
        let title = self.second_line.filter(|t| !t.trim().is_empty())?;
        let np = NowPlaying {
            title,
            artist: self.third_line.unwrap_or_default(),
            station_index: station,
            cover_url: self.cover.map(|uuid| cover_url_for(&uuid)),
        };
        Some((np, self.end_time))
    }
}

fn cover_url_for(uuid: &str) -> String {
    format!("{PIKAPI_IMAGES_URL}/{uuid}/{COVER_SIZE}?webp=false")
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
            .user_agent(crate::USER_AGENT)
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

/// Downloads a small binary resource (cover art). `None` on any failure.
fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let mut resp = http().get(url).call().ok()?;
    resp.body_mut().read_to_vec().ok()
}

/// Resolves the current track: `livemeta` first (no key), then the GraphQL
/// API with the user's key — for stations `livemeta` does not serve (FIP
/// Cultes) or when `livemeta` fails. Without a key, a `livemeta` failure is
/// reported as is, and a station without `livemeta` id resolves to no track.
fn fetch_live(station: usize) -> LiveResult {
    let token = config::api_token();
    if let Some(id) = STATIONS[station].livemeta_id {
        match fetch_livemeta(id, station) {
            Ok(r) => return Ok(r),
            Err(e) if token.is_empty() => return Err(e),
            Err(e) => eprintln!("metadata: livemeta ({}): {e} — falling back to GraphQL", STATIONS[station].name),
        }
    }
    if token.is_empty() {
        return Ok(None);
    }
    fetch_graphql(station, &token)
}

fn fetch_livemeta(id: u32, station: usize) -> LiveResult {
    let url = format!("{LIVEMETA_URL}/{id}/fip_player");
    // Not `?`: a 401/403 here is not about the user's key.
    let mut resp = http()
        .get(&url)
        .call()
        .map_err(|e| LiveError::Other(Box::new(e)))?;
    let parsed: LivemetaResp = resp
        .body_mut()
        .read_json()
        .map_err(|e| LiveError::Other(Box::new(e)))?;
    Ok(parsed.now.and_then(|now| now.into_now_playing(station)))
}

fn fetch_graphql(station: usize, token: &str) -> LiveResult {
    let api = STATIONS[station].api_station;
    let query = format!(
        "{{ live(station: {api}) {{ song {{ end track {{ title mainArtists }} }} }} }}"
    );

    let mut resp = http()
        .post(config::GRAPHQL_ENDPOINT)
        .header("x-token", token)
        .send_json(serde_json::json!({ "query": query }))?;
    let parsed: GqlResp = resp
        .body_mut()
        .read_json()
        .map_err(|e| LiveError::Other(Box::new(e)))?;

    let live = parsed.data.and_then(|d| d.live);
    // A GraphQL error with no `live` payload: surface it (an invalid key
    // answers HTTP 200 with `errors` on some deployments).
    if let (None, Some(err)) = (&live, parsed.errors.first()) {
        let m = err.message.to_ascii_lowercase();
        return Err(if m.contains("token") || m.contains("unauthor") || m.contains("forbidden") {
            LiveError::Unauthorized(200)
        } else {
            LiveError::Graphql(err.message.clone())
        });
    }
    let Some(track) = live.and_then(|l| l.song).and_then(|song| {
        song.track.map(|t| (t, song.end))
    }) else {
        return Ok(None);
    };
    let (track, end) = track;
    let Some(title) = track.title else {
        return Ok(None);
    };

    let np = NowPlaying {
        title,
        artist: track.main_artists.unwrap_or_default().join(", "),
        station_index: station,
        // The open API has no artwork: iTunes lookup.
        cover_url: None,
    };
    Ok(Some((np, end)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_term_strips_qualifiers_and_credits() {
        assert_eq!(clean_term("So What (Remastered 2001)"), "So What");
        assert_eq!(clean_term("Song [Live] (Edit)"), "Song");
        assert_eq!(clean_term("Artist feat. Someone"), "Artist");
        assert_eq!(clean_term("Artist Ft. Someone"), "Artist");
        assert_eq!(clean_term("Taylor Swift. Hello"), "Taylor Swift. Hello");
        assert_eq!(clean_term("  spaced   out  "), "spaced out");
        assert_eq!(clean_term("unbalanced ) paren"), "unbalanced paren");
    }

    #[test]
    fn poll_delay_is_bounded_and_falls_back() {
        assert_eq!(poll_delay(None, 1000), FALLBACK_POLL);
        assert_eq!(poll_delay(Some(900), 1000), FALLBACK_POLL);
        assert_eq!(poll_delay(Some(1010), 1000), Duration::from_secs(10) + END_MARGIN);
        assert_eq!(poll_delay(Some(1000 + 36_000), 1000), MAX_WAIT);
    }

    #[test]
    fn livemeta_song_and_programme_are_told_apart() {
        let song: LivemetaResp = serde_json::from_str(
            r#"{"now":{"firstLine":"FIP Jazz","secondLine":"Nica's tempo","secondLineSongUuid":"338d","thirdLine":"Art Blakey","cover":"a1e7","startTime":1,"endTime":99},"delayToRefresh":300000}"#,
        )
        .unwrap();
        let (np, end) = song.now.unwrap().into_now_playing(2).unwrap();
        assert_eq!((np.title.as_str(), np.artist.as_str(), np.station_index), ("Nica's tempo", "Art Blakey", 2));
        assert_eq!(end, Some(99));
        assert_eq!(np.cover_url.as_deref(), Some("https://www.radiofrance.fr/pikapi/images/a1e7/600x600?webp=false"));

        let idle: LivemetaResp = serde_json::from_str(
            r#"{"now":{"firstLine":"FIP","secondLine":"Le direct","thirdLine":null,"cover":"34e9","startTime":null,"endTime":null}}"#,
        )
        .unwrap();
        assert!(idle.now.unwrap().into_now_playing(0).is_none());
        let empty: LivemetaResp = serde_json::from_str(r#"{"prev":[]}"#).unwrap();
        assert!(empty.now.is_none());
    }

    #[test]
    fn gql_response_tolerates_null_title_and_reports_errors() {
        let r: GqlResp = serde_json::from_str(
            r#"{"data":{"live":{"song":{"end":1,"track":{"title":null,"mainArtists":null}}}}}"#,
        )
        .unwrap();
        assert!(r.data.unwrap().live.unwrap().song.unwrap().track.unwrap().title.is_none());
        let r: GqlResp =
            serde_json::from_str(r#"{"data":null,"errors":[{"message":"Invalid token"}]}"#).unwrap();
        assert_eq!(r.errors[0].message, "Invalid token");
    }
}
