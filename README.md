<img src="fip-logo.png" alt="Fiplet logo" width="96" align="left" />

# Fiplet


[FIP](https://www.radiofrance.fr/fip) ([France Inter Paris](https://fr.wikipedia.org/wiki/FIP_(radio))) is originally a French public radio station run by
Radio France. It's known for broadcasting music exclusively, ad-free and
around the clock, and for an eclectic playlist — curated by humans, not
algorithms — that makes it easy to discover new sounds. Think of it as a
free, ad-free alternative to Spotify with a particularly refined musical
selection.

FIP now offers several web-radios by genre, and you'll find all of them here.

## Screenshots

<p align="center">
  <img src="docs/screenshot-menu.png" alt="Mini-player and the Stations submenu" width="420" />
  <br/>
  <em>The menu-bar mini-player — cover art, title/artist, play/pause, ♥ (like) and ⧉ (copy) — above the Stations / Favorites / Quit menu.</em>
</p>

<p align="center">
  <img src="docs/screenshot-settings.png" alt="Settings window" width="380" />
  <br/>
  <em>Settings: your Open FIP API key and the "Launch at login" option.</em>
</p>

A **lightweight, standalone** FIP player for the macOS menu bar. Plays the
**13 FIP webradios** and shows the current track and cover art — no account or
API key needed — plus a local **like**.

> **Unofficial project.** This is an independent, community-made app. It is
> **not** an official Radio France product and is not affiliated with,
> sponsored by, or endorsed by Radio France.

## Features

- **13 FIP webradios**, one click away in the menu bar.
- **Now playing**: title, artist and **cover art**, out of the box (no API key).
- **Cover art in Control Center** and on the **lock screen**, plus hardware
  **media keys** (play/pause, next/previous station).
- **♥ Like** the current track; **click** a favorite in the list to **copy**
  it, **double-click** it to **remove** it.
- **⧉ Copy** the current track ("Artist — Title") to the clipboard.
- **Remembers the last station** and reopens on it.
- **Auto-reconnects** if the live stream drops, with an *"offline"* hint.
- Optional **launch at login** (stays silent when opened automatically at
  startup — it only auto-plays when you open it yourself).
- **English or French** interface, following the system language.


## Menu contents

A **♪** icon in the menu bar opens a **menu** (`NSMenu`):

- **Mini-player**: radio name + gear ⚙, cover art, title, artist, ▶/⏸, ♥ (like)
  and ⧉ (copy the current track).
- **"Stations" submenu**: the 13 webradios (the current station is checkmarked).
- **"Favorites (N)" submenu**: the liked tracks; **click** one to copy it
  ("Artist — Title"), **double-click** it to remove it.
- **Quit**.

The interface is in English, or in French when the system language is French
(`src/i18n.rs`).

## Now playing data

Track info comes from Radio France's **`livemeta` API**, the unauthenticated
endpoint the radiofrance.fr player itself uses (`api.radiofrance.fr/livemeta/live`).
It returns the current title, artist, track end time and a cover id, for 12 of
the 13 webradios — **no API key required**. The app polls it once per track
(aligned on the announced end time, capped at 5 minutes).

### API key — optional

The **Radio France open API** (GraphQL, personal key) is used in two cases only:

- **FIP Cultes**, which `livemeta` does not serve: without a key, that station
  plays but shows no track info.
- **Fallback** if `livemeta` fails or disappears (it is a legacy, undocumented
  API).

Getting a key is **free**: create a **developer account** on the
[Radio France open API portal](https://developers.radiofrance.fr) and copy the
token. Paste it in Settings (gear ⚙, or the window that opens on first launch).

## Cover art

With `livemeta`, the cover id is resolved through radiofrance.fr's image service
(`pikapi/images/<id>/600x600`), so the artwork is Radio France's own. When the
track comes from the GraphQL API (FIP Cultes, fallback), which has no artwork,
or when that download fails, the app falls back to the public **iTunes Search
API** with "artist title" (stripped of `(Remaster)`, `[Live]`, `feat.` …
qualifiers), keeping the first match's artwork upgraded to 600×600. When nothing
is found, the player simply shows no image. The cover is also published to
**Control Center** and the **lock screen**.

## A note about the audio streams

FIP is famously **ad-free**. However, Radio France's Icecast streams, when
opened by a third-party player, begin with a **~30-second audio advert** on
every connection. This pre-roll does **not** occur in the stream served on
Radio France's own website: its player requests the very same Icecast URL with
an `?id=radiofrance` query parameter, and the app uses that same URL. The
stream itself is identical (AAC 192 kbps); the app neither adds nor removes
anything from it. This parameter is undocumented: should Radio France change
it, the pre-roll would come back but playback would keep working.

Note that **resuming after a pause reconnects to the live edge** so audio and
track info stay in sync.

If the stream drops (network blip, server hiccup), the app **reconnects to the
live edge automatically** (with an exponential backoff, 2 s → 30 s) and shows an
*"offline — reconnecting…"* hint until it is back.

## ♥ Like

The ♥ button saves the current track to `likes.json`; saved tracks appear in the
**"Favorites"** submenu. **Click** a row to copy that track ("Artist — Title")
to the clipboard; **double-click** it to **remove** it. A single click never
deletes, so you can't remove a favorite by accident, and the menu stays open.

The **⧉** button copies the current track ("Artist — Title", or just the title)
to the clipboard.

## Requirements

- **macOS 13 (Ventura) or later** (the "launch at login" option relies on
  `SMAppService`, introduced in macOS 13).
- Apple Silicon by default; `UNIVERSAL=1 ./bundle.sh` also builds an Intel
  slice (needs `rustup target add x86_64-apple-darwin`). The GitHub Release
  bundles are universal.
- Stable Rust (to build from source).

## Run / install

Download `FIP-vX.Y.Z.zip` from the
[Releases](../../releases) page (built by CI from the tagged sources; a
`.sha256` checksum is attached), unzip and drag `FIP.app` to `/Applications`.

Or build it yourself:

```bash
cargo run --release
# or build the app bundle:
./bundle.sh
open "FIP.app"        # or: cp -r "FIP.app" /Applications/
```

`bundle.sh` signs the app ad-hoc. The app is **not notarized**: Gatekeeper
will refuse a downloaded copy until you clear the quarantine flag once:
`xattr -dr com.apple.quarantine '/Applications/FIP.app'`. Only do this for a
bundle whose checksum matches the one published with the release.

## Configuration

| Variable | Purpose | Default |
|---|---|---|
| `RADIOFRANCE_TOKEN` | Radio France open API token (used when Settings hold no key) | *(none — FIP Cultes shows no track info, no GraphQL fallback)* |

The key entered in Settings is persisted in
`~/Library/Application Support/fip-menubar-light/settings.json` (owner-only
permissions, written atomically). It is sent to the API in the `x-token`
header, never in a URL. Likes are stored in `likes.json` in the same folder.

## Architecture

| File | Role |
|---|---|
| `src/main.rs` | entry point: `ui::run()` |
| `src/ui.rs` | native AppKit interface (`NSStatusItem` + `NSMenu`, Stations/Favorites submenus) |
| `src/i18n.rs` | UI strings: English, or French when the system language is French |
| `src/audio.rs` | live AAC stream decoded internally (`ureq` + symphonia → rodio), with auto-reconnect |
| `src/metadata.rs` | now-playing poller: `livemeta` (no key) → GraphQL fallback (key); cover from radiofrance.fr, iTunes fallback |
| `src/likes.rs` | local likes: add / remove (writes `likes.json`) |
| `src/config.rs` | API token + last station + `settings.json` |
| `src/stations.rs` | table of the 13 stations (stream URL, GraphQL enum, `livemeta` id) |
| `src/model.rs` | shared types |

Two worker threads (audio, metadata); their `AppEvent`s are drained by an
`NSTimer` on the main thread (registered in the common run-loop modes so the
mini-player keeps updating while the menu is open). No async runtime, no
subprocess.

Development: `cargo fmt`, `cargo test`, `cargo clippy --all-targets -- -D warnings`;
the same checks run in CI (`.github/workflows/ci.yml`), and tagging `vX.Y.Z`
publishes a universal `FIP.app` to GitHub Releases. Diagnostics go to stderr
(visible with `cargo run`, or in Console.app for the bundled app).

> The container is an `NSMenu` (not an `NSPopover`): a menu that is being opened
> keeps the menu bar revealed on top of a full-screen app, which the popover did
> not do.

## License

The **code** is released under the [MIT License](LICENSE).

This does **not** cover Radio France's intellectual property: the name **"FIP"**,
the **FIP logo/icon**, and the audio streams and metadata served by Radio France
remain the property of Radio France and are used here only to interoperate with
their public services. See the unofficial-project notice at the top of this file.
