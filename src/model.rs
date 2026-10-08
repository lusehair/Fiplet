//! Types shared between threads (light version).

/// Case-insensitive (artist, title) pair used to identify a track.
pub type TrackKey = (String, String);

/// Currently playing track, resolved via the Radio France open API.
#[derive(Clone, Debug, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    /// Index of the relevant station in [`crate::stations::STATIONS`].
    pub station_index: usize,
    /// Cover art URL provided by the metadata source (Radio France), if any.
    /// When absent, the cover is looked up on iTunes from artist + title.
    pub cover_url: Option<String>,
}

impl NowPlaying {
    /// Case-insensitive deduplication key (artist, title).
    pub fn dedup_key(&self) -> TrackKey {
        (self.artist.to_lowercase(), self.title.to_lowercase())
    }
}

/// Events pushed to the main loop from the worker threads.
#[derive(Debug)]
pub enum AppEvent {
    /// New resolved track.
    NowPlaying(NowPlaying),
    /// No track resolved (spoken program / error).
    MetadataCleared { station_index: usize },
    /// The API rejected the request (invalid key, …) or returned a GraphQL
    /// error. Shown in place of the track title; `detail` is for the log.
    ApiError { unauthorized: bool, detail: String },
    /// Album cover art downloaded from iTunes. `key` identifies the track it
    /// was fetched for, so a late cover is never applied to a newer track.
    Cover {
        station_index: usize,
        key: TrackKey,
        jpeg: Vec<u8>,
    },
    /// Play/Pause media key (keyboard or Control Center): toggles.
    MediaPlayPause,
    /// Explicit Play command (Control Center): resumes if paused, else no-op.
    MediaPlay,
    /// Explicit Pause command (Control Center, headphones unplugged, another
    /// app starting playback): pauses if playing, else no-op.
    MediaPause,
    /// Next media key (next station).
    MediaNext,
    /// Previous media key (previous station).
    MediaPrev,
    /// The live stream could not be (re)connected (network down): the audio
    /// thread is retrying. Used to show an "offline" hint.
    StreamOffline,
    /// The live stream is connected/playing again after an outage.
    StreamOnline,
    /// No audio output device could be opened: playback is impossible.
    AudioUnavailable,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_key_is_case_insensitive() {
        let a = NowPlaying { title: "Blue In Green".into(), artist: "Miles DAVIS".into(), station_index: 0, cover_url: None };
        let b = NowPlaying { title: "blue in green".into(), artist: "miles davis".into(), station_index: 3, cover_url: None };
        assert_eq!(a.dedup_key(), b.dedup_key());
    }
}
