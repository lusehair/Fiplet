//! Types shared between threads (light version).

/// Currently playing track, resolved via the Radio France open API.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// Index of the relevant station in [`crate::stations::STATIONS`].
    pub station_index: usize,
}

impl NowPlaying {
    /// Case-insensitive deduplication key (artist, title).
    pub fn dedup_key(&self) -> (String, String) {
        (self.artist.to_lowercase(), self.title.to_lowercase())
    }
}

/// Events pushed to the main loop from the metadata thread.
#[derive(Debug)]
pub enum AppEvent {
    /// New resolved track.
    NowPlaying(NowPlaying),
    /// No track resolved (spoken program / error).
    MetadataCleared { station_index: usize },
    /// Album cover art downloaded from iTunes.
    Cover { station_index: usize, jpeg: Vec<u8> },
    /// Play/Pause media key (keyboard or Control Center).
    MediaPlayPause,
    /// Next media key (next station).
    MediaNext,
    /// Previous media key (previous station).
    MediaPrev,
    /// The live stream could not be (re)connected (network down): the audio
    /// thread is retrying. Used to show an "offline" hint.
    StreamOffline,
    /// The live stream is connected/playing again after an outage.
    StreamOnline,
}
