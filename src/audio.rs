//! Audio thread (light version): plays the live AAC Icecast stream decoded
//! in-process (`ureq` + symphonia via rodio). No local files, no ffmpeg.
//!
//! The HTTP stream is read by a dedicated thread into a bounded queue, and
//! playback starts only once a few seconds are buffered: rodio decodes inside
//! the real-time audio callback, so reading the socket directly there turned
//! every network hiccup into an audible skip.
//!
//! A small watchdog reconnects automatically if the live stream drops (network
//! blip, server hiccup, decoder giving up) with an exponential backoff, and
//! reports outages to the UI via [`AppEvent`].

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::{Decoder, DeviceSinkBuilder, Player};

use crate::model::AppEvent;

/// Commands accepted by the audio thread.
///
/// Pause / resume are explicit (not a toggle): `rodio::Player::clear()` pauses
/// the player as a side effect, so the player's own `is_paused()` cannot be
/// used to infer what the *user* wants.
pub enum AudioCmd {
    /// Stops playback and starts the live stream (Icecast URL). Also used to
    /// resume: reconnecting resyncs audio with the live edge.
    PlayStream(String),
    /// Pauses playback and stops the watchdog from reconnecting.
    Pause,
}

/// How often the watchdog wakes to check for a dropped stream.
const WATCHDOG: Duration = Duration::from_secs(2);
/// Reconnect backoff: first retry after `BACKOFF_MIN`, doubling up to
/// `BACKOFF_MAX` while the stream keeps failing; reset on success.
const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Size of the chunks read from the network by the fetch thread.
const CHUNK: usize = 4 * 1024;
/// Bytes buffered before playback starts (~4 s at the hifi 192 kbit/s rate):
/// the margin that absorbs network jitter.
const PREBUFFER: usize = 96 * 1024;
/// Maximum time spent waiting for the prebuffer before starting anyway.
const PREBUFFER_TIMEOUT: Duration = Duration::from_secs(10);
/// Capacity of the chunk queue (~1 MiB, ~45 s); TCP backpressure beyond that.
const QUEUE_CHUNKS: usize = 256;

/// Starts the audio thread and returns the command sender. `events` is used to
/// report stream outages / recovery to the UI.
pub fn spawn(events: Sender<AppEvent>) -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || run(rx, events))
        .expect("failed to spawn the audio thread");
    tx
}

/// HTTP agent for the stream: a connect timeout so a dead network fails fast
/// (enabling a quick reconnect), but deliberately NO read/global timeout — the
/// live stream must stay open indefinitely.
fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .user_agent(crate::USER_AGENT)
        .build()
        .new_agent()
}

/// Mutable state of the audio thread, shared between the command handler and
/// the watchdog.
struct Session {
    player: Player,
    agent: ureq::Agent,
    /// Set to `true` by `StreamReader` when the underlying HTTP stream ends or
    /// errors, so the watchdog knows to reconnect.
    ended: Arc<AtomicBool>,
    current_url: Option<String>,
    /// Whether the user wants audio (distinguishes a real outage from a pause).
    should_play: bool,
    /// Whether we last reported an outage (to emit each transition only once).
    offline: bool,
    /// Current backoff delay and the earliest time of the next reconnect.
    backoff: Duration,
    next_retry: Instant,
    events: Sender<AppEvent>,
}

fn run(rx: Receiver<AudioCmd>, events: Sender<AppEvent>) {
    let handle = match DeviceSinkBuilder::open_default_sink() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("audio: no audio output: {e}");
            let _ = events.send(AppEvent::AudioUnavailable);
            return;
        }
    };
    let player = Player::connect_new(handle.mixer());
    player.set_volume(1.0);

    let mut s = Session {
        player,
        agent: http(),
        ended: Arc::new(AtomicBool::new(false)),
        current_url: None,
        should_play: false,
        offline: false,
        backoff: BACKOFF_MIN,
        next_retry: Instant::now(),
        events,
    };

    loop {
        match rx.recv_timeout(WATCHDOG) {
            Ok(AudioCmd::PlayStream(url)) => {
                s.current_url = Some(url.clone());
                s.should_play = true;
                s.backoff = BACKOFF_MIN;
                s.connect(&url);
            }
            Ok(AudioCmd::Pause) => {
                s.should_play = false;
                s.player.pause();
            }
            Err(RecvTimeoutError::Timeout) => s.watchdog(),
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

impl Session {
    /// Auto-reconnect: we want audio, but the stream ended on its own.
    ///
    /// `ended` covers HTTP EOF / I/O errors seen by our reader. `empty()`
    /// covers the decoder giving up on its own (e.g. lost ADTS sync): rodio's
    /// symphonia decoder ends the source on non-recoverable errors without
    /// our reader ever seeing a failure.
    fn watchdog(&mut self) {
        if !self.should_play {
            return;
        }
        let dropped = self.ended.load(Ordering::SeqCst) || self.player.empty();
        if !dropped || Instant::now() < self.next_retry {
            return;
        }
        if let Some(url) = self.current_url.clone() {
            self.connect(&url);
        }
    }

    /// (Re)connects to the live stream and updates the outage state, emitting an
    /// `AppEvent` only on transitions (offline ↔ online).
    fn connect(&mut self, url: &str) {
        self.player.clear();
        self.ended.store(false, Ordering::SeqCst);
        match play_stream(&self.agent, &self.player, url, self.ended.clone()) {
            Ok(()) => {
                self.backoff = BACKOFF_MIN;
                if self.offline {
                    self.offline = false;
                    let _ = self.events.send(AppEvent::StreamOnline);
                }
            }
            Err(e) => {
                eprintln!("audio: cannot open stream ({url}): {e}, retrying in {:?}", self.backoff);
                // Leave `ended` set so the watchdog keeps retrying (with backoff).
                self.ended.store(true, Ordering::SeqCst);
                self.next_retry = Instant::now() + self.backoff;
                self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                if !self.offline {
                    self.offline = true;
                    let _ = self.events.send(AppEvent::StreamOffline);
                }
            }
        }
    }
}

fn play_stream(
    agent: &ureq::Agent,
    player: &Player,
    url: &str,
    ended: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = agent.get(url).call()?.into_body().into_reader();
    let (tx, rx) = mpsc::sync_channel(QUEUE_CHUNKS);
    std::thread::Builder::new()
        .name("audio-fetch".into())
        .spawn(move || fetch(reader, tx))?;

    // Prebuffer: fill the jitter margin before handing the stream to rodio.
    let mut stream = StreamReader { rx: Mutex::new(rx), chunk: Vec::new(), pos: 0, ended };
    let deadline = Instant::now() + PREBUFFER_TIMEOUT;
    while stream.chunk.len() < PREBUFFER {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.rx.get_mut().unwrap().recv_timeout(left) {
            Ok(c) => stream.chunk.extend_from_slice(&c),
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) if stream.chunk.is_empty() => {
                return Err("stream ended before any data".into());
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let decoder = Decoder::builder()
        .with_data(stream)
        .with_hint("aac")
        .with_mime_type("audio/aac")
        .with_seekable(false)
        .build()?;
    player.append(decoder);
    // `clear()` (called before every connect) pauses the player: un-pause.
    player.play();
    Ok(())
}

/// Fetch thread: pumps the HTTP stream into the chunk queue. Exits on EOF /
/// read error (dropping `tx` signals the end to the reader), or as soon as the
/// reader is dropped (`clear()` on reconnect / station change).
fn fetch(mut reader: impl Read, tx: SyncSender<Vec<u8>>) {
    loop {
        let mut buf = vec![0; CHUNK];
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                buf.truncate(n);
                if tx.send(buf).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                eprintln!("audio: stream read error: {e}");
                return;
            }
        }
    }
}

/// Adapts the buffered stream (read-only) to the decoder's `Read + Seek`
/// interface. Reads are served from memory; they only block if the buffer has
/// fully drained. The stream is infinite and non-rewindable: `seek` fails
/// (non-seekable decoder). End-of-stream flips `ended` so the audio thread
/// reconnects.
struct StreamReader {
    /// In a `Mutex` only to make the reader `Sync` (rodio requires it); always
    /// accessed through `get_mut`, so never actually locked.
    rx: Mutex<Receiver<Vec<u8>>>,
    chunk: Vec<u8>,
    pos: usize,
    ended: Arc<AtomicBool>,
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.chunk.len() {
            match self.rx.get_mut().unwrap().recv() {
                Ok(c) => {
                    self.chunk = c;
                    self.pos = 0;
                }
                Err(_) => {
                    self.ended.store(true, Ordering::SeqCst);
                    return Ok(0);
                }
            }
        }
        let n = buf.len().min(self.chunk.len() - self.pos);
        buf[..n].copy_from_slice(&self.chunk[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "flux non rembobinable",
        ))
    }
}
