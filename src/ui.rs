//! Native macOS interface (light version): a menu-bar icon that opens a menu —
//! mini-player (radio name + gear ⚙, cover art, title, artist, play/pause), a
//! "Radios" submenu, and Quit. Settings: API key only.

use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSControlStateValueOff, NSControlStateValueOn, NSImage, NSImageScaling, NSImageView,
    NSMenu, NSMenuItem, NSPasteboard, NSPasteboardTypeString, NSSecureTextField, NSStatusBar,
    NSStatusItem, NSTextAlignment, NSTextField, NSVariableStatusItemLength, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{
    NSData, NSDictionary, NSNotification, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use objc2_media_player::{
    MPMediaItemArtwork, MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork,
    MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyPlaybackRate,
    MPNowPlayingPlaybackState,
};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::audio::{self, AudioCmd};
use crate::config;
use crate::i18n::tr;
use crate::likes::{self, Like};
use crate::metadata::{self, MetaCmd};
use crate::model::{AppEvent, NowPlaying};
use crate::stations::STATIONS;

const W: f64 = 300.0;
const H: f64 = 350.0;
/// Height of one row in the Favorites list view.
const FAV_ROW_H: f64 = 26.0;
/// Two clicks on the same favorite within this delay count as a double-click.
const FAV_DOUBLE_CLICK: Duration = Duration::from_millis(600);

struct State {
    current_station: usize,
    paused: bool,
    /// Last resolved track, used to feed Now Playing (Control Center).
    now: Option<NowPlaying>,
    /// Current cover art, mirrored to Now Playing (Control Center / lock screen).
    artwork: Option<Retained<NSImage>>,
}

struct Ivars {
    state: RefCell<State>,
    events_rx: Receiver<AppEvent>,
    audio_tx: Sender<AudioCmd>,
    meta_tx: Sender<MetaCmd>,
    name: Retained<NSTextField>,
    title: Retained<NSTextField>,
    artist: Retained<NSTextField>,
    cover: Retained<NSImageView>,
    play_btn: Retained<NSButton>,
    like_btn: Retained<NSButton>,
    station_items: Vec<Retained<NSMenuItem>>,
    /// "Favorites" submenu: parent item (title with count) and the custom row
    /// view it hosts. `last_fav_click` tracks the previous click to detect a
    /// double-click (remove) on a favorite row.
    fav_view: Retained<FlippedView>,
    fav_item: Retained<NSMenuItem>,
    last_fav_click: RefCell<Option<(isize, Instant)>>,
    status_item: Retained<NSStatusItem>,
    settings_window: Retained<NSWindow>,
    token_field: Retained<NSSecureTextField>,
    login_item_checkbox: Retained<NSButton>,
    mtm: MainThreadMarker,
}

// View with its origin at the top, for a simple layout.
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "FipLightFlippedView"]
    struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "FipLightController"]
    #[ivars = Ivars]
    struct Controller;

    impl Controller {
        #[unsafe(method(playPause:))]
        fn play_pause(&self, _sender: Option<&AnyObject>) {
            self.toggle_pause();
        }

        #[unsafe(method(like:))]
        fn like(&self, _sender: Option<&AnyObject>) {
            self.like_current();
        }

        #[unsafe(method(copyTrack:))]
        fn copy_track(&self, _sender: Option<&AnyObject>) {
            self.copy_current();
        }

        #[unsafe(method(stationSelected:))]
        fn station_selected(&self, sender: &NSMenuItem) {
            let i = sender.tag();
            if i >= 0 {
                self.play_station(i as usize);
            }
        }

        #[unsafe(method(favRowClicked:))]
        fn on_fav_row(&self, sender: &NSButton) {
            self.favorite_row_clicked(sender.tag());
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            std::process::exit(0);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: Option<&AnyObject>) {
            self.show_settings();
        }

        #[unsafe(method(saveSettings:))]
        fn save_settings(&self, _sender: Option<&AnyObject>) {
            let iv = self.ivars();
            let settings = config::Settings {
                token: iv.token_field.stringValue().to_string(),
                // Preserve the persisted last station rather than resetting it.
                ..config::load_settings()
            };
            if let Err(e) = config::save_settings(&settings) {
                eprintln!("settings: failed to save: {e}");
            }
            iv.settings_window.close();
            // The key may have just been entered or removed: re-poll so FIP
            // Cultes / the GraphQL fallback pick it up right away.
            let station = iv.state.borrow().current_station;
            self.refresh_meta(station);
        }

        #[unsafe(method(tick:))]
        fn tick(&self, _timer: Option<&AnyObject>) {
            while let Ok(ev) = self.ivars().events_rx.try_recv() {
                self.handle_event(ev);
            }
        }

        #[unsafe(method(toggleLoginItem:))]
        fn on_toggle_login_item(&self, _sender: Option<&AnyObject>) {
            self.toggle_login_item();
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSApplicationDelegate for Controller {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            // Auto-play on a normal (manual) launch, but stay paused when the app
            // was opened automatically at session startup (login item), to avoid
            // blasting music unexpectedly when the user logs in.
            //
            // This must run here — while the launch Apple event is still current —
            // because `launched_as_login_item()` reads that event; it is no longer
            // available by the time `run()` returns to the run loop.
            if !launched_as_login_item() {
                let station = self.ivars().state.borrow().current_station;
                self.select_station(station, true);
            }
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker, ivars: Ivars) -> Retained<Self> {
        let this = mtm.alloc::<Controller>().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    /// Pre-fills and shows the Settings window (used by the gear ⚙ and by the
    /// onboarding flow on first launch).
    fn show_settings(&self) {
        let iv = self.ivars();
        iv.token_field.setStringValue(&s(&config::api_token()));
        iv.login_item_checkbox.setState(if login_item_enabled() {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        // Close the menu (if open) before showing the Settings window.
        if let Some(menu) = iv.status_item.menu(iv.mtm) {
            menu.cancelTracking();
        }
        // Menu-bar app (LSUIElement): `activate` alone does not always bring the
        // window to the front, hence `orderFrontRegardless`.
        NSApplication::sharedApplication(iv.mtm).activate();
        iv.settings_window.makeKeyAndOrderFront(None);
        iv.settings_window.orderFrontRegardless();
    }

    fn play_station(&self, i: usize) {
        // Remember the user's choice so the next launch reopens on this station.
        config::save_last_station(i);
        self.select_station(i, true);
    }

    /// Selects a station and updates the UI; only starts the audio stream when
    /// `play` is true (used at startup to select the default station while
    /// staying paused, e.g. when launched as a login item).
    fn select_station(&self, i: usize, play: bool) {
        let iv = self.ivars();
        {
            let mut st = iv.state.borrow_mut();
            st.current_station = i;
            st.paused = !play;
            st.now = None;
            // Drop the previous station's cover too, or Now Playing would keep
            // publishing it under the new station's name.
            st.artwork = None;
        }
        self.update_station_check();
        self.refresh_like_button();
        iv.name.setStringValue(&s(STATIONS[i].name));
        iv.artist.setStringValue(&s(""));
        iv.cover.setHidden(true);
        iv.cover.setImage(None);
        iv.play_btn.setTitle(&s(if play { "⏸" } else { "▶" }));
        if play {
            let _ = iv
                .audio_tx
                .send(AudioCmd::PlayStream(STATIONS[i].stream_url.to_string()));
            self.refresh_meta(i);
        } else {
            // Paused (e.g. login-item launch): don't poll the API for events the
            // UI would ignore anyway, and don't show a misleading "Loading…".
            let _ = iv.meta_tx.send(MetaCmd::Pause);
            iv.title.setStringValue(&s(""));
        }
        self.update_now_playing();
    }

    /// Registers or unregisters the app as a macOS login item, following the
    /// Settings checkbox state. Applied immediately (no Save button needed).
    fn toggle_login_item(&self) {
        let iv = self.ivars();
        let checked = iv.login_item_checkbox.state() == NSControlStateValueOn;
        let service = unsafe { SMAppService::mainAppService() };
        let result = unsafe {
            if checked {
                service.registerAndReturnError()
            } else {
                service.unregisterAndReturnError()
            }
        };
        match result {
            Ok(()) => {
                if checked && unsafe { service.status() } == SMAppServiceStatus::RequiresApproval {
                    unsafe { SMAppService::openSystemSettingsLoginItems() };
                }
            }
            Err(e) => {
                eprintln!("launch at login: failed: {e}");
                // Reflect the actual system state rather than the failed click.
                iv.login_item_checkbox.setState(if login_item_enabled() {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
        }
    }

    /// Checks the current station in the menu and unchecks the others.
    fn update_station_check(&self) {
        let iv = self.ivars();
        let cur = iv.state.borrow().current_station;
        for (i, item) in iv.station_items.iter().enumerate() {
            item.setState(if i == cur {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    /// Adds the current track to favorites (like without download).
    fn like_current(&self) {
        let iv = self.ivars();
        let np = iv.state.borrow().now.clone();
        let Some(np) = np else { return };
        match likes::add(&np) {
            Ok(Some(_)) => {
                self.rebuild_fav_menu();
                iv.like_btn.setTitle(&s("✓"));
            }
            Ok(None) => {}
            Err(e) => eprintln!("likes: failed to save: {e}"),
        }
    }

    /// Copies the current track ("Artist — Title", or just the title) to the
    /// clipboard. No-op if no track is resolved.
    fn copy_current(&self) {
        let now = self.ivars().state.borrow().now.clone();
        let Some(np) = now else { return };
        if np.title.is_empty() {
            return;
        }
        copy_to_clipboard(&track_text(&np.artist, &np.title));
    }

    /// Rebuilds the "Favorites" list view from the JSON. Each favorite is a
    /// borderless button (a clickable row that doesn't dismiss the menu);
    /// clicking a row copies it, double-clicking removes it (see
    /// `favorite_row_clicked`).
    fn rebuild_fav_menu(&self) {
        let iv = self.ivars();
        let mtm = iv.mtm;
        let favs = likes::all();

        // Clear the previous rows (iterate a snapshot; removal mutates the view).
        let subs = iv.fav_view.subviews();
        for i in 0..subs.count() {
            subs.objectAtIndex(i).removeFromSuperview();
        }

        let width = 260.0;
        let rows = favs.len().max(1);
        iv.fav_view
            .setFrame(rect(0.0, 0.0, width, rows as f64 * FAV_ROW_H));

        if favs.is_empty() {
            let label = make_left_label(
                mtm,
                tr("— Aucun favori —", "— No favorites —"),
                rect(10.0, 0.0, width - 20.0, FAV_ROW_H),
            );
            iv.fav_view.addSubview(&label);
        } else {
            let target: &AnyObject = self;
            for (i, l) in favs.iter().enumerate() {
                let btn = make_button(
                    mtm,
                    &fav_text(l),
                    rect(6.0, i as f64 * FAV_ROW_H, width - 12.0, FAV_ROW_H),
                );
                btn.setBordered(false);
                btn.setAlignment(NSTextAlignment::Left);
                btn.setTag(i as isize);
                unsafe {
                    btn.setTarget(Some(target));
                    btn.setAction(Some(sel!(favRowClicked:)));
                }
                iv.fav_view.addSubview(&btn);
            }
        }

        iv.fav_item.setTitle(&s(&format!(
            "{} ({})",
            tr("Favoris", "Favorites"),
            favs.len()
        )));
    }

    /// Handles a click on a favorite row. A single click copies the favorite
    /// ("Artist — Title") to the clipboard and arms the row; a second click on
    /// the same row within `FAV_DOUBLE_CLICK` removes that favorite.
    fn favorite_row_clicked(&self, tag: isize) {
        let now = Instant::now();
        let is_double = match *self.ivars().last_fav_click.borrow() {
            Some((prev, at)) => prev == tag && now.duration_since(at) < FAV_DOUBLE_CLICK,
            None => false,
        };
        let Some(like) = likes::all().get(tag as usize).cloned() else {
            return;
        };
        if is_double {
            *self.ivars().last_fav_click.borrow_mut() = None;
            likes::remove_by_key(&like.key());
            self.rebuild_fav_menu();
        } else {
            *self.ivars().last_fav_click.borrow_mut() = Some((tag, now));
            copy_to_clipboard(&fav_text(&like));
        }
    }

    /// Updates the ♥ button state based on availability and the track's status.
    fn refresh_like_button(&self) {
        let iv = self.ivars();
        let now = iv.state.borrow().now.clone();
        match now {
            Some(np) => {
                iv.like_btn.setEnabled(true);
                iv.like_btn
                    .setTitle(&s(if likes::contains(&np) { "✓" } else { "♥" }));
            }
            None => {
                iv.like_btn.setEnabled(false);
                iv.like_btn.setTitle(&s("♥"));
            }
        }
    }

    /// (Re)starts track polling for `station`. No API key is required: the
    /// poller uses the public `livemeta` API, and the key (if any) for FIP
    /// Cultes and as a fallback.
    fn refresh_meta(&self, station: usize) {
        let iv = self.ivars();
        let _ = iv.meta_tx.send(MetaCmd::Switch(station));
        iv.title.setStringValue(&s(tr("Chargement…", "Loading…")));
    }

    fn toggle_pause(&self) {
        let paused = self.ivars().state.borrow().paused;
        self.set_paused(!paused);
    }

    /// Pauses or resumes playback; no-op if already in the requested state
    /// (so a stray "pause" from Control Center never starts playback).
    fn set_paused(&self, paused: bool) {
        let iv = self.ivars();
        // Release the RefCell borrow before any AppKit call.
        let station = {
            let mut st = iv.state.borrow_mut();
            if st.paused == paused {
                return;
            }
            st.paused = paused;
            st.current_station
        };
        if paused {
            let _ = iv.audio_tx.send(AudioCmd::Pause);
            let _ = iv.meta_tx.send(MetaCmd::Pause);
            iv.play_btn.setTitle(&s("▶"));
        } else {
            // Resume: reconnect to the live stream to resync audio and track.
            let _ = iv.audio_tx.send(AudioCmd::PlayStream(
                STATIONS[station].stream_url.to_string(),
            ));
            iv.play_btn.setTitle(&s("⏸"));
            self.refresh_meta(station);
        }
        self.update_now_playing();
    }

    fn handle_event(&self, ev: AppEvent) {
        let iv = self.ivars();
        match ev {
            AppEvent::NowPlaying(np) => {
                let show = {
                    let st = iv.state.borrow();
                    !st.paused && np.station_index == st.current_station
                };
                if show {
                    iv.name.setStringValue(&s(STATIONS[np.station_index].name));
                    iv.title.setStringValue(&s(&np.title));
                    iv.artist.setStringValue(&s(if np.artist.is_empty() {
                        "—"
                    } else {
                        &np.artist
                    }));
                    iv.cover.setHidden(true);
                    iv.cover.setImage(None);
                    {
                        let mut st = iv.state.borrow_mut();
                        st.now = Some(np);
                        // The old cover must not linger in Control Center if
                        // no artwork is found for the new track.
                        st.artwork = None;
                    }
                    self.refresh_like_button();
                    self.update_now_playing();
                }
            }
            AppEvent::MetadataCleared { station_index } => {
                if station_index == iv.state.borrow().current_station {
                    iv.name.setStringValue(&s(STATIONS[station_index].name));
                    iv.title.setStringValue(&s("—"));
                    iv.artist.setStringValue(&s(""));
                    iv.cover.setHidden(true);
                    iv.cover.setImage(None);
                    {
                        let mut st = iv.state.borrow_mut();
                        st.now = None;
                        st.artwork = None;
                    }
                    self.refresh_like_button();
                    self.update_now_playing();
                }
            }
            AppEvent::Cover {
                station_index,
                key,
                jpeg,
            } => {
                // Only apply a cover to the track it was fetched for: a slow
                // download must not land on the next track (or on the same
                // station after an A→B→A switch).
                let matches = {
                    let st = iv.state.borrow();
                    station_index == st.current_station
                        && st.now.as_ref().map(|np| np.dedup_key()) == Some(key)
                };
                if matches {
                    let data = NSData::with_bytes(&jpeg);
                    if let Some(img) = NSImage::initWithData(iv.mtm.alloc(), &data) {
                        iv.cover.setImage(Some(&img));
                        iv.cover.setHidden(false);
                        iv.state.borrow_mut().artwork = Some(img);
                        // Publish the artwork to Control Center / the lock screen.
                        self.update_now_playing();
                    }
                }
            }
            AppEvent::StreamOffline => {
                if !iv.state.borrow().paused {
                    iv.title.setStringValue(&s(tr(
                        "Hors ligne — reconnexion…",
                        "Offline — reconnecting…",
                    )));
                    iv.artist.setStringValue(&s(""));
                }
            }
            AppEvent::StreamOnline => {
                // Reconnected: restore the normal track display for the station.
                let station = iv.state.borrow().current_station;
                self.refresh_meta(station);
            }
            AppEvent::ApiError {
                unauthorized,
                detail,
            } => {
                eprintln!("metadata: {detail}");
                if !iv.state.borrow().paused {
                    iv.title.setStringValue(&s(if unauthorized {
                        tr("Clé API invalide — voir ⚙", "Invalid API key — see ⚙")
                    } else {
                        tr("Erreur API", "API error")
                    }));
                    iv.artist.setStringValue(&s(""));
                }
            }
            AppEvent::AudioUnavailable => {
                iv.title
                    .setStringValue(&s(tr("Sortie audio indisponible", "No audio output")));
                iv.artist.setStringValue(&s(""));
                iv.play_btn.setEnabled(false);
            }
            AppEvent::MediaPlayPause => self.toggle_pause(),
            AppEvent::MediaPlay => self.set_paused(false),
            AppEvent::MediaPause => self.set_paused(true),
            AppEvent::MediaNext => {
                let (c, n) = {
                    let st = iv.state.borrow();
                    (st.current_station, STATIONS.len())
                };
                self.play_station((c + 1) % n);
            }
            AppEvent::MediaPrev => {
                let (c, n) = {
                    let st = iv.state.borrow();
                    (st.current_station, STATIONS.len())
                };
                self.play_station((c + n - 1) % n);
            }
        }
    }

    /// Publishes the current state to `MPNowPlayingInfoCenter` (title/artist +
    /// play/pause state). Required so macOS routes the media keys to the app and
    /// shows the info in Control Center.
    fn update_now_playing(&self) {
        let iv = self.ivars();
        let (title, artist, paused, artwork_img) = {
            let st = iv.state.borrow();
            let (title, artist) = match &st.now {
                Some(np) => (np.title.clone(), np.artist.clone()),
                None => (STATIONS[st.current_station].name.to_string(), String::new()),
            };
            (title, artist, st.paused, st.artwork.clone())
        };

        let title_ns = s(&title);
        let artist_ns = s(&artist);
        let rate_num = NSNumber::new_f64(if paused { 0.0 } else { 1.0 });
        let mut keys: Vec<&NSString> = unsafe {
            vec![
                MPMediaItemPropertyTitle,
                MPMediaItemPropertyArtist,
                MPNowPlayingInfoPropertyPlaybackRate,
            ]
        };
        let mut vals: Vec<&AnyObject> = vec![&*title_ns, &*artist_ns, &*rate_num];

        // Cover art, if we have it (shown in Control Center / on the lock screen).
        let artwork = artwork_img.as_ref().map(make_artwork);
        if let Some(art) = artwork.as_ref() {
            keys.push(unsafe { MPMediaItemPropertyArtwork });
            vals.push(&**art);
        }

        let info = NSDictionary::from_slices(&keys, &vals);

        let center = unsafe { MPNowPlayingInfoCenter::defaultCenter() };
        unsafe {
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(if paused {
                MPNowPlayingPlaybackState::Paused
            } else {
                MPNowPlayingPlaybackState::Playing
            });
        }
    }
}

/// Starts the native application (does not return).
pub fn run() {
    let mtm = MainThreadMarker::new().expect("ui::run must be called on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let (events_tx, events_rx) = mpsc::channel::<AppEvent>();
    // Restore the last station the user was on (0/FIP by default).
    let start_station = config::last_station().min(STATIONS.len().saturating_sub(1));
    let audio_tx = audio::spawn(events_tx.clone());
    let meta_tx = metadata::spawn(events_tx.clone(), start_station);

    // Views (flipped coordinates: Y measured from the top).
    let content: Retained<FlippedView> =
        unsafe { msg_send![mtm.alloc::<FlippedView>(), initWithFrame: rect(0.0, 0.0, W, H)] };
    let name = make_label(mtm, rect(20.0, 12.0, 228.0, 22.0));
    let gear = make_button(mtm, "⚙", rect(250.0, 10.0, 30.0, 24.0));
    let cover = make_cover_view(mtm, rect(50.0, 42.0, 200.0, 200.0));
    let title = make_label(mtm, rect(20.0, 250.0, 260.0, 24.0));
    let artist = make_label(mtm, rect(20.0, 276.0, 260.0, 20.0));
    let play_btn = make_button(mtm, "⏸", rect(110.0, 306.0, 56.0, 34.0));
    let like_btn = make_button(mtm, "♥", rect(178.0, 306.0, 40.0, 34.0));
    let copy_btn = make_button(mtm, "⧉", rect(230.0, 306.0, 40.0, 34.0));

    for v in [
        content_child(&name),
        content_child(&gear),
        content_child(&title),
        content_child(&artist),
        content_child(&play_btn),
        content_child(&like_btn),
        content_child(&copy_btn),
    ] {
        content.addSubview(v);
    }
    content.addSubview(&cover);

    let (settings_window, token_field, login_item_checkbox, save_btn) = build_settings_window(mtm);

    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(btn) = status_item.button(mtm) {
        let data = NSData::with_bytes(include_bytes!("menubar-icon.png"));
        if let Some(img) = NSImage::initWithData(mtm.alloc(), &data) {
            img.setSize(NSSize::new(18.0, 18.0));
            // Template image: rendered monochrome, adapts to the light/dark menu bar.
            img.setTemplate(true);
            btn.setImage(Some(&img));
        } else {
            btn.setTitle(&s("♪"));
        }
    }

    // The mini-player is hosted in the `view` of a single NSMenuItem. An NSMenu
    // being opened (menu tracking) pins the menu bar in full screen — which an
    // NSPopover does not do. This is the heart of the fix.
    let menu = NSMenu::new(mtm);
    let player_item = NSMenuItem::new(mtm);
    let content_view: &NSView = &content;
    player_item.setView(Some(content_view));
    menu.addItem(&player_item);
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    // Stations grouped into a "Radios" submenu to save space (the drop-down list
    // cannot open inside a menu that is being tracked). The current station is
    // checked via update_station_check.
    let radios_item = NSMenuItem::new(mtm);
    radios_item.setTitle(&s(tr("Radios", "Stations")));
    menu.addItem(&radios_item);
    let radios_menu = NSMenu::new(mtm);
    let mut station_items = Vec::with_capacity(STATIONS.len());
    for (i, st) in STATIONS.iter().enumerate() {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&s(st.name));
        item.setTag(i as isize);
        radios_menu.addItem(&item);
        station_items.push(item);
    }
    radios_item.setSubmenu(Some(&radios_menu));
    // "Favorites" submenu: liked tracks, display only (filled in later via
    // rebuild_fav_menu). Same sub-list presentation as "Radios".
    let fav_item = NSMenuItem::new(mtm);
    fav_item.setTitle(&s(tr("Favoris (0)", "Favorites (0)")));
    menu.addItem(&fav_item);
    // The favorites list is a custom row view (like the mini-player), hosted in
    // a single submenu item, so rows can be double-clicked without the menu
    // closing. Populated by rebuild_fav_menu.
    let fav_menu = NSMenu::new(mtm);
    let fav_view: Retained<FlippedView> = unsafe {
        msg_send![mtm.alloc::<FlippedView>(), initWithFrame: rect(0.0, 0.0, 260.0, FAV_ROW_H)]
    };
    let fav_host = NSMenuItem::new(mtm);
    let fav_view_ref: &NSView = &fav_view;
    fav_host.setView(Some(fav_view_ref));
    fav_menu.addItem(&fav_host);
    fav_item.setSubmenu(Some(&fav_menu));

    // Quit at the very bottom of the menu.
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let quit_item = NSMenuItem::new(mtm);
    quit_item.setTitle(&s(tr("Quitter", "Quit")));
    menu.addItem(&quit_item);

    status_item.setMenu(Some(&menu));

    let controller = Controller::new(
        mtm,
        Ivars {
            state: RefCell::new(State {
                current_station: start_station,
                paused: false,
                now: None,
                artwork: None,
            }),
            events_rx,
            audio_tx,
            meta_tx,
            name,
            title,
            artist,
            cover,
            play_btn,
            like_btn,
            station_items,
            fav_view,
            fav_item,
            last_fav_click: RefCell::new(None),
            status_item,
            settings_window,
            token_field,
            login_item_checkbox,
            mtm,
        },
    );

    {
        let iv = controller.ivars();
        let target: &AnyObject = &controller;
        unsafe {
            iv.play_btn.setTarget(Some(target));
            iv.play_btn.setAction(Some(sel!(playPause:)));
            iv.like_btn.setTarget(Some(target));
            iv.like_btn.setAction(Some(sel!(like:)));
            for item in &iv.station_items {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(stationSelected:)));
            }
            quit_item.setTarget(Some(target));
            quit_item.setAction(Some(sel!(quit:)));
            gear.setTarget(Some(target));
            gear.setAction(Some(sel!(openSettings:)));
            copy_btn.setTarget(Some(target));
            copy_btn.setAction(Some(sel!(copyTrack:)));
            save_btn.setTarget(Some(target));
            save_btn.setAction(Some(sel!(saveSettings:)));
            iv.login_item_checkbox.setTarget(Some(target));
            iv.login_item_checkbox
                .setAction(Some(sel!(toggleLoginItem:)));
            // `scheduledTimer…` would only fire in the default run-loop mode,
            // i.e. NOT while a menu is open (event-tracking mode) — exactly
            // when the user is looking at the mini-player. Register it in the
            // common modes so track/cover/offline updates keep flowing.
            let timer = NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                0.25,
                target,
                sel!(tick:),
                None,
                true,
            );
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
    }

    controller.rebuild_fav_menu();
    // Select the default station but stay paused for now. Whether we actually
    // start playing is decided in `applicationDidFinishLaunching:`, once we can
    // tell (via the launch Apple event) if this launch was a manual one — play —
    // or an automatic session-startup launch (login item) — stay silent.
    controller.select_station(start_station, false);

    // First launch (no settings.json yet): open Settings once (optional API
    // key, launch at login). Once saved, the window no longer reopens.
    if config::needs_onboarding() {
        controller.show_settings();
    }

    // Hardware media keys + Control Center.
    crate::media_keys::install(events_tx.clone());

    // The controller is the app delegate so it receives applicationDidFinishLaunching:
    // (see above). NSApplication does not retain its delegate, but `controller`
    // is kept alive by the `Retained` held here until after `app.run()` returns.
    app.setDelegate(Some(ProtocolObject::from_ref(&*controller)));

    app.run();
    drop(controller);
}

fn build_settings_window(
    mtm: MainThreadMarker,
) -> (
    Retained<NSWindow>,
    Retained<NSSecureTextField>,
    Retained<NSButton>,
    Retained<NSButton>,
) {
    let sw = 440.0;
    let sh = 144.0;
    let content: Retained<FlippedView> =
        unsafe { msg_send![mtm.alloc::<FlippedView>(), initWithFrame: rect(0.0, 0.0, sw, sh)] };

    let token_label = make_left_label(
        mtm,
        tr(
            "Clé API Open FIP (optionnelle : FIP Cultes et secours) :",
            "Open FIP API key (optional: FIP Cultes and fallback):",
        ),
        rect(20.0, 14.0, 400.0, 18.0),
    );
    // Secure field: the key is a credential, don't display it in clear.
    let token_field = NSSecureTextField::initWithFrame(mtm.alloc(), rect(20.0, 36.0, 400.0, 24.0));
    let login_item_checkbox = unsafe {
        NSButton::checkboxWithTitle_target_action(
            &s(tr("Lancer au démarrage de la session", "Launch at login")),
            None,
            None,
            mtm,
        )
    };
    login_item_checkbox.setFrame(rect(20.0, 68.0, 400.0, 22.0));
    let save_btn = make_button(
        mtm,
        tr("Enregistrer", "Save"),
        rect(330.0, 104.0, 90.0, 30.0),
    );
    // Enter saves (default button), instead of silently doing nothing.
    save_btn.setKeyEquivalent(&s("\r"));

    for v in [
        content_child(&token_label),
        content_child(&token_field),
        content_child(&login_item_checkbox),
        content_child(&save_btn),
    ] {
        content.addSubview(v);
    }

    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            rect(0.0, 0.0, sw, sh),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // Without this, closing the window frees it (isReleasedWhenClosed = true by
    // default) even though we keep a `Retained` to it: the 2nd open would target a
    // dead object. So we reuse it from one open to the next.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&s(tr("Paramètres", "Settings")));
    window.setContentView(Some(&content));
    window.center();
    (window, token_field, login_item_checkbox, save_btn)
}

/// True if the app is currently registered as a macOS login item.
fn login_item_enabled() -> bool {
    unsafe { SMAppService::mainAppService().status() == SMAppServiceStatus::Enabled }
}

/// True if *this* launch was triggered automatically at session startup (the app
/// is a login item and macOS opened it at login), as opposed to the user
/// launching it manually.
///
/// This is distinct from [`login_item_enabled`], which only tells whether the app
/// is *registered* to launch at login — it stays true even for a later manual
/// launch during the same session.
///
/// Detection reads the launch Apple event: a login-item launch is a
/// `kAEOpenApplication` event carrying `keyAELaunchedAsLogInItem`. `currentAppleEvent`
/// is only set while that event is being handled, so this must be called from
/// `applicationDidFinishLaunching:` (or earlier), not after the run loop starts.
fn launched_as_login_item() -> bool {
    // Carbon four-char codes (AppleEvents.h / AERegistry.h).
    const K_AE_OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
    const KEY_AE_PROP_DATA: u32 = u32::from_be_bytes(*b"prdt");
    const KEY_AE_LAUNCHED_AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgin");

    unsafe {
        let Some(cls) = AnyClass::get(c"NSAppleEventManager") else {
            return false;
        };
        let mgr: *mut AnyObject = msg_send![cls, sharedAppleEventManager];
        if mgr.is_null() {
            return false;
        }
        let event: *mut AnyObject = msg_send![mgr, currentAppleEvent];
        if event.is_null() {
            return false;
        }
        let event_id: u32 = msg_send![event, eventID];
        if event_id != K_AE_OPEN_APPLICATION {
            return false;
        }
        let prop: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: KEY_AE_PROP_DATA];
        if prop.is_null() {
            return false;
        }
        let code: u32 = msg_send![prop, enumCodeValue];
        code == KEY_AE_LAUNCHED_AS_LOGIN_ITEM
    }
}

/// Wraps an `NSImage` as an `MPMediaItemArtwork` for Now Playing. The request
/// handler returns the same image for any requested size.
fn make_artwork(img: &Retained<NSImage>) -> Retained<MPMediaItemArtwork> {
    let size = img.size();
    let img_for_block = img.clone();
    let handler =
        RcBlock::new(move |_size: NSSize| -> NonNull<NSImage> { NonNull::from(&*img_for_block) });
    unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    }
}

fn s(t: &str) -> Retained<NSString> {
    NSString::from_str(t)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn content_child(v: &Retained<impl AsRef<NSView>>) -> &NSView {
    (**v).as_ref()
}

fn make_label(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&s(""), mtm);
    label.setFrame(frame);
    label.setAlignment(NSTextAlignment::Center);
    label
}

fn make_cover_view(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSImageView> {
    let iv = NSImageView::initWithFrame(mtm.alloc(), frame);
    iv.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
    iv.setHidden(true);
    iv
}

fn make_left_label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&s(text), mtm);
    label.setFrame(frame);
    label
}

fn make_button(mtm: MainThreadMarker, title: &str, frame: NSRect) -> Retained<NSButton> {
    let btn = unsafe { NSButton::buttonWithTitle_target_action(&s(title), None, None, mtm) };
    btn.setFrame(frame);
    btn
}

/// Label for a favorite in the submenu: "Artist — Title" (or just the title).
/// "Artist — Title", or just the title when the artist is unknown. Used both
/// for the favorites rows and for what gets copied to the clipboard.
fn track_text(artist: &str, title: &str) -> String {
    if artist.is_empty() {
        title.to_string()
    } else {
        format!("{artist} — {title}")
    }
}

fn fav_text(like: &Like) -> String {
    track_text(&like.artist, &like.title)
}

/// Replaces the clipboard contents with `text`.
fn copy_to_clipboard(text: &str) {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    unsafe { pb.setString_forType(&s(text), NSPasteboardTypeString) };
}
