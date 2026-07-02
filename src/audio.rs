//! Audio thread (light version): plays the live AAC Icecast stream decoded
//! in-process (`ureq` + symphonia via rodio). No local files, no ffmpeg.
//!
//! A small watchdog reconnects automatically if the live stream drops (network
//! blip, server hiccup) and reports outages to the UI via [`AppEvent`].

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use rodio::{Decoder, DeviceSinkBuilder, Player};

use crate::model::AppEvent;

/// Commands accepted by the audio thread.
pub enum AudioCmd {
    /// Stops playback and starts the live stream (Icecast URL).
    PlayStream(String),
    /// Toggles play / pause.
    TogglePause,
}

/// How often the watchdog wakes to check for a dropped stream.
const WATCHDOG: Duration = Duration::from_secs(2);

/// Starts the audio thread and returns the command sender. `events` is used to
/// report stream outages / recovery to the UI.
pub fn spawn(events: Sender<AppEvent>) -> Sender<AudioCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(move || run(rx, events))
        .expect("démarrage du thread audio");
    tx
}

/// HTTP agent for the stream: a connect timeout so a dead network fails fast
/// (enabling a quick reconnect), but deliberately NO read/global timeout — the
/// live stream must stay open indefinitely.
fn http() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(8)))
        .build()
        .new_agent()
}

fn run(rx: Receiver<AudioCmd>, events: Sender<AppEvent>) {
    let handle = match DeviceSinkBuilder::open_default_sink() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("audio: pas de sortie audio : {e}");
            return;
        }
    };
    let player = Player::connect_new(handle.mixer());
    player.set_volume(1.0);
    let agent = http();

    // Set to `true` by `StreamReader` when the underlying HTTP stream ends or
    // errors, so the watchdog knows to reconnect.
    let ended = Arc::new(AtomicBool::new(false));
    let mut current_url: Option<String> = None;
    // Whether the user wants audio (distinguishes a real outage from a pause).
    let mut should_play = false;
    // Whether we last reported an outage (to emit each transition only once).
    let mut offline = false;

    loop {
        match rx.recv_timeout(WATCHDOG) {
            Ok(AudioCmd::PlayStream(url)) => {
                current_url = Some(url.clone());
                should_play = true;
                connect(&agent, &player, &url, &ended, &events, &mut offline);
            }
            Ok(AudioCmd::TogglePause) => {
                if player.is_paused() {
                    player.play();
                } else {
                    player.pause();
                }
                should_play = !player.is_paused();
            }
            Err(RecvTimeoutError::Timeout) => {
                // Auto-reconnect: we want audio, but the stream ended on its own.
                if should_play && ended.load(Ordering::SeqCst) {
                    if let Some(url) = current_url.clone() {
                        connect(&agent, &player, &url, &ended, &events, &mut offline);
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// (Re)connects to the live stream and updates the outage state, emitting an
/// `AppEvent` only on transitions (offline ↔ online).
fn connect(
    agent: &ureq::Agent,
    player: &Player,
    url: &str,
    ended: &Arc<AtomicBool>,
    events: &Sender<AppEvent>,
    offline: &mut bool,
) {
    player.clear();
    ended.store(false, Ordering::SeqCst);
    match play_stream(agent, player, url, ended.clone()) {
        Ok(()) => {
            if *offline {
                *offline = false;
                let _ = events.send(AppEvent::StreamOnline);
            }
        }
        Err(e) => {
            eprintln!("audio: flux impossible ({url}) : {e}");
            // Leave `ended` set so the watchdog keeps retrying every tick.
            ended.store(true, Ordering::SeqCst);
            if !*offline {
                *offline = true;
                let _ = events.send(AppEvent::StreamOffline);
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
    let decoder = Decoder::builder()
        .with_data(StreamReader { inner: reader, ended })
        .with_hint("aac")
        .with_mime_type("audio/aac")
        .with_seekable(false)
        .build()?;
    player.append(decoder);
    player.play();
    Ok(())
}

/// Adapts an HTTP stream (read-only) to the decoder's `Read + Seek` interface.
/// The stream is infinite and non-rewindable: `seek` fails (non-seekable decoder).
/// End-of-stream or a read error flips `ended` so the audio thread reconnects.
struct StreamReader<R> {
    inner: R,
    ended: Arc<AtomicBool>,
}

impl<R: Read> Read for StreamReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.inner.read(buf) {
            Ok(0) => {
                self.ended.store(true, Ordering::SeqCst);
                Ok(0)
            }
            Ok(n) => Ok(n),
            Err(e) => {
                self.ended.store(true, Ordering::SeqCst);
                Err(e)
            }
        }
    }
}

impl<R> Seek for StreamReader<R> {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "flux non rembobinable",
        ))
    }
}
