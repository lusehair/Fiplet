//! Hardware media keys (Play/Pause, Next, Previous) and Control Center, via
//! `MPRemoteCommandCenter` from the MediaPlayer framework.
//!
//! We register handlers on the remote commands. Combined with publishing the
//! "Now Playing" info (see [`crate::ui`]), this makes the app the current
//! "Now Playing" app, and macOS then routes the media keys to it — without any
//! accessibility permission.
//!
//! The handlers may run **off the main thread**: they therefore ONLY push an
//! [`AppEvent`] onto the existing channel. All the actual processing (state
//! mutation + AppKit) happens later in `tick:`, on the main thread.

use std::ptr::NonNull;
use std::sync::mpsc::Sender;

use block2::RcBlock;
use objc2_media_player::{
    MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};

use crate::model::AppEvent;

/// Registers the media key handlers. To be called once from `run()`, on the
/// main thread. The center, blocks and tokens are retained by
/// MediaPlayer.framework and deliberately leaked (the app never exits cleanly,
/// `run()` does not return).
pub fn install(tx: Sender<AppEvent>) {
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    unsafe {
        register(&center.togglePlayPauseCommand(), tx.clone(), || {
            AppEvent::MediaPlayPause
        });
        register(&center.playCommand(), tx.clone(), || AppEvent::MediaPlayPause);
        register(&center.pauseCommand(), tx.clone(), || AppEvent::MediaPlayPause);
        register(&center.nextTrackCommand(), tx.clone(), || AppEvent::MediaNext);
        register(&center.previousTrackCommand(), tx, || AppEvent::MediaPrev);
    }
    std::mem::forget(center);
}

/// Attaches a handler to a remote command. `make` builds the `AppEvent` to
/// emit (a constructor because `AppEvent` is not `Clone`).
fn register(cmd: &MPRemoteCommand, tx: Sender<AppEvent>, make: fn() -> AppEvent) {
    unsafe { cmd.setEnabled(true) };
    let handler = RcBlock::new(move |_ev: NonNull<MPRemoteCommandEvent>| {
        // This block may run off the main thread: we only touch the channel.
        let _ = tx.send(make());
        MPRemoteCommandHandlerStatus::Success
    });
    let token = unsafe { cmd.addTargetWithHandler(&handler) };
    std::mem::forget(token);
    std::mem::forget(handler);
}
