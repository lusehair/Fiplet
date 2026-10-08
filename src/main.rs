//! fip-menubar-light — a player for the FIP webradios in the macOS menu bar.
//!
//! *Light* version: listen to the 13 FIP webradios with display of the current
//! track via the Radio France open API, plus a local like (display only). No
//! downloads, no local playback, no third-party services. An independent project
//! from `fip-menubar` (the full version).

mod audio;
mod config;
mod i18n;
mod likes;
mod media_keys;
mod metadata;
mod model;
mod stations;
mod ui;

/// `User-Agent` sent with every HTTP request (API etiquette).
pub const USER_AGENT: &str = concat!("Fiplet/", env!("CARGO_PKG_VERSION"));

fn main() {
    ui::run();
}
