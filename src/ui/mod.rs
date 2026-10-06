//! The interface: a sidebar, a content panel and a player bar, as rounded
//! surfaces on a black window.
//!
//! Rendering is reactive: nothing is redrawn unless something happens (input,
//! backend event, or once per second while music plays), so the CPU stays idle.

mod ambient;
mod theme;
mod titlebar;
mod views;
mod widgets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers, TextureHandle, TextureOptions};

use crate::backend::{AppState, AppStatus, Backend, Command, Event};
use crate::config::{Paths, Settings};
use crate::media::MediaKeys;
use crate::model::{
    AlbumSummary, ArtistSummary, ArtistsPage, PlaylistSummary, Repeat, SearchResults, Track, ViewKey,
};
use crate::sys;
use theme::Palette;

/// Recently played tracks kept for the home page.
const MAX_RECENT: usize = 30;

fn load_recent(paths: &Paths) -> Vec<Track> {
    std::fs::read(paths.recent_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Track>>(&bytes).ok())
        .map(|mut tracks| {
            tracks.truncate(MAX_RECENT);
            tracks
        })
        .unwrap_or_default()
}

/// Pages kept in memory for instant back-navigation; older ones are dropped
/// (they come back from the disk cache without any download).
const MAX_PAGES: usize = 12;
const MAX_TEXTURES: usize = 48;

pub enum Page {
    /// Followed artists, then the artists of the liked tracks.
    Artists(ArtistsPage),
    Tracks {
        title: String,
        subtitle: String,
        /// Album or playlist cover, for the banner.
        cover: Option<String>,
        tracks: Arc<Vec<Track>>,
    },
    /// A playlist Spotify only plays as a whole (its tracks are not readable).
    Context {
        title: String,
        subtitle: String,
        cover: Option<String>,
        uri: String,
        total: u32,
    },
    Albums {
        title: String,
        albums: Vec<AlbumSummary>,
    },
    Artist {
        name: String,
        image: Option<String>,
        /// The user's liked tracks by this artist.
        liked: Arc<Vec<Track>>,
        albums: Vec<AlbumSummary>,
    },
    Search(SearchResults),
}

#[derive(Default)]
struct Player {
    now: Option<Track>,
    playing: bool,
    buffering: bool,
    position_ms: u32,
    at: Option<Instant>,
    liked: HashMap<String, bool>,
    upcoming: Vec<Track>,
    /// Playlist or artist that Spotify plays itself, if any.
    context: Option<String>,
    shuffle: bool,
    repeat: Repeat,
    volume_before_mute: Option<f32>,
}

impl Player {
    fn position(&self) -> u32 {
        let mut pos = self.position_ms;
        if self.playing
            && let Some(at) = self.at
        {
            pos += at.elapsed().as_millis() as u32;
        }
        match &self.now {
            Some(t) if t.duration_ms > 0 => pos.min(t.duration_ms),
            _ => pos,
        }
    }

    fn is_liked(&self) -> Option<bool> {
        let now = self.now.as_ref()?;
        self.liked.get(&now.id).copied()
    }
}

struct Toast {
    /// Messages with a key replace each other (e.g. a running counter).
    key: Option<&'static str>,
    text: String,
    error: bool,
    at: Instant,
}

/// Cached result of the list filter, recomputed only when inputs change.
struct Filtered {
    view: ViewKey,
    query: String,
    len: usize,
    tracks: Arc<Vec<Track>>,
}

#[derive(Default)]
struct Covers {
    textures: HashMap<String, TextureHandle>,
    /// Colors of each cover, for the player bar background.
    tints: HashMap<String, ambient::Tint>,
    order: VecDeque<String>,
    requested: HashSet<String>,
    /// Blurred gradients (url, texture): the player bar's and the page banner's.
    ambient: [Option<(String, TextureHandle)>; 2],
}

/// Which blurred gradient: each place keeps its own.
#[derive(Clone, Copy)]
pub enum Ambient {
    Player = 0,
    Banner = 1,
}

pub struct App {
    backend: Backend,
    paths: Paths,
    settings: Settings,
    palette: Palette,
    user: String,
    /// The user's own Spotify application (library, search).
    app_status: AppStatus,
    playlists: Vec<PlaylistSummary>,
    view: ViewKey,
    history: Vec<ViewKey>,
    pages: HashMap<ViewKey, Page>,
    page_order: VecDeque<ViewKey>,
    loading: HashSet<ViewKey>,
    failures: HashMap<ViewKey, String>,
    search_text: String,
    focus_search: bool,
    filter: String,
    filtered: Option<Filtered>,
    selected: Option<usize>,
    reveal_selected: bool,
    player: Player,
    covers: Covers,
    /// The logo in white (128 px), and the liked tracks picture (loaded when shown).
    logo: TextureHandle,
    liked_art: Option<TextureHandle>,
    /// Tracks recently played in SpotiLite, most recent first.
    recent: Arc<Vec<Track>>,
    /// When the playlists were asked again (their refresh button turns meanwhile).
    playlists_loading: Option<Instant>,
    toasts: Vec<Toast>,
    usage: (u64, u64),
    memory: sys::Memory,
    memory_at: Option<Instant>,
    /// State of the playback engine and memory of its WebView2 processes.
    engine_status: String,
    engine_memory: u64,
    media: MediaKeys,
    minimized: bool,
    /// Setup form.
    setup_id: String,
    setup_secret: String,
    show_secret: bool,
    /// The user asked to change the application from the settings.
    editing_app: bool,
    port_draft: String,
    /// Debug builds only: fake data for UI work without a Spotify account.
    demo: bool,
}

impl App {
    pub fn new(
        ctx: &egui::Context,
        window: &winit::window::Window,
        paths: Paths,
        settings: Settings,
    ) -> Self {
        theme::install_fonts(ctx);
        let palette = Palette::amoled();
        theme::apply(ctx, &palette);
        let recent = load_recent(&paths);
        ctx.set_zoom_factor(settings.ui_scale);
        ctx.options_mut(|o| o.zoom_with_keyboard = true);

        let backend = Backend::spawn(ctx.clone(), paths.clone(), settings.clone());
        let media = MediaKeys::new(window, ctx.clone());
        let player = Player { shuffle: settings.shuffle, repeat: settings.repeat, ..Player::default() };
        Self {
            backend,
            setup_id: String::new(),
            setup_secret: String::new(),
            show_secret: false,
            editing_app: false,
            port_draft: settings.redirect_port.to_string(),
            paths,
            palette,
            user: String::new(),
            app_status: AppStatus {
                state: AppState::Unknown,
                client_id: String::new(),
                has_secret: false,
                needs_playback_auth: false,
            },
            playlists: Vec::new(),
            view: ViewKey::Home,
            history: Vec::new(),
            pages: HashMap::new(),
            page_order: VecDeque::new(),
            loading: HashSet::new(),
            failures: HashMap::new(),
            search_text: String::new(),
            focus_search: false,
            filter: String::new(),
            filtered: None,
            selected: None,
            reveal_selected: false,
            player,
            covers: Covers::default(),
            logo: ctx.load_texture("logo", crate::logo::image(), TextureOptions::LINEAR),
            liked_art: None,
            playlists_loading: None,
            recent: Arc::new(recent),
            toasts: Vec::new(),
            usage: (0, 0),
            memory: sys::memory(),
            memory_at: None,
            engine_status: "Arrêté".into(),
            engine_memory: 0,
            media,
            minimized: false,
            settings,
            demo: false,
        }
    }

    /// `SPOTILITE_DEMO=1` (debug builds) fills the interface with fake data.
    #[cfg(debug_assertions)]
    pub fn with_demo(mut self, ctx: &egui::Context) -> Self {
        use crate::model::ArtistRef;
        if std::env::var_os("SPOTILITE_DEMO").is_none() {
            return self;
        }
        self.demo = true;
        self.user = "Démo".into();
        let names =
            ["Lueurs", "Minuit passé", "Rivages", "Sur le fil", "Horizon bas", "Nocturne", "Papier", "Écho"];
        let artists = ["Nora Vale", "Les Ondes", "Kaito", "Mélodie Brune", "Atlas Sud"];
        let tracks: Vec<Track> = (0..120)
            .map(|i| Track {
                id: format!("demo{i}"),
                name: format!("{} {}", names[i % names.len()], i / names.len() + 1),
                artists: vec![ArtistRef {
                    id: format!("a{}", i % 5),
                    name: artists[i % artists.len()].into(),
                }],
                album: format!("Album {}", i % 9 + 1),
                album_id: format!("al{}", i % 9),
                duration_ms: 150_000 + (i as u32 * 7919) % 120_000,
                image: Some(
                    ["demo-cover", "demo-artist-0", "demo-artist-1", "demo-artist-2"][i % 9 % 4].into(),
                ),
                playable: i % 17 != 16,
            })
            .collect();
        self.playlists =
            ["Découvertes", "Focus", "Route", "Chill du dimanche", "Années 2000", "Jazz tranquille"]
                .iter()
                .enumerate()
                .map(|(i, name)| PlaylistSummary {
                    id: format!("p{i}"),
                    name: (*name).into(),
                    owner: "demo".into(),
                    total: 40 + i as u32 * 13,
                    snapshot_id: String::new(),
                    image: None,
                })
                .collect();
        // A synthetic cover (dusk gradient) to show the player bar colors.
        let demo_cover = "demo-cover".to_string();
        let image = egui::ColorImage::new(
            [64, 64],
            (0..64 * 64)
                .map(|i| {
                    let (x, y) = ((i % 64) as f32 / 63.0, (i / 64) as f32 / 63.0);
                    let t = (x + y) / 2.0;
                    egui::Color32::from_rgb(
                        (90.0 + 160.0 * t) as u8,
                        (40.0 + 70.0 * t * t) as u8,
                        (150.0 - 110.0 * t) as u8,
                    )
                })
                .collect(),
        );
        self.covers.tints.insert(demo_cover.clone(), ambient::tint_of(&image));
        let texture = ctx.load_texture(demo_cover.clone(), image, TextureOptions::LINEAR);
        self.covers.textures.insert(demo_cover.clone(), texture);
        let mut now = tracks[2].clone();
        now.image = Some(demo_cover);
        self.player.now = Some(now);
        self.player.playing = true;
        self.player.position_ms = 83_000;
        self.player.at = Some(Instant::now());
        self.player.liked.insert(tracks[2].id.clone(), true);
        self.player.upcoming = tracks[3..12].to_vec();
        self.app_status.state = if std::env::var_os("SPOTILITE_DEMO_SETUP").is_some() {
            AppState::NotConfigured
        } else {
            AppState::Connected
        };
        self.view = ViewKey::Liked;
        self.pages.insert(
            ViewKey::Liked,
            Page::Tracks {
                title: "Titres likés".into(),
                subtitle: String::new(),
                cover: None,
                tracks: Arc::new(tracks.clone()),
            },
        );
        self.pages.insert(
            ViewKey::Playlist("p0".into()),
            Page::Tracks {
                title: "Découvertes".into(),
                subtitle: "Démo".into(),
                cover: Some("demo-cover".into()),
                tracks: Arc::new(tracks.iter().skip(3).step_by(3).take(30).cloned().collect()),
            },
        );
        self.recent = Arc::new(tracks.iter().skip(4).step_by(5).take(12).cloned().collect());
        self.engine_status = "Prêt · DRM Widevine".into();
        self.engine_memory = 112 * 1024 * 1024;
        // Artists: three portraits (synthetic gradients), the rest with initials.
        let portrait = |ctx: &egui::Context, covers: &mut Covers, i: usize| {
            let key = format!("demo-artist-{i}");
            let hue = [(210.0, 120.0, 90.0), (80.0, 140.0, 200.0), (150.0, 90.0, 190.0)][i % 3];
            let image = egui::ColorImage::new(
                [64, 64],
                (0..64 * 64)
                    .map(|p| {
                        let (x, y) = ((p % 64) as f32 / 63.0 - 0.5, (p / 64) as f32 / 63.0 - 0.35);
                        let light = (1.0 - (x * x + y * y).sqrt() * 1.6).clamp(0.25, 1.0);
                        egui::Color32::from_rgb(
                            (hue.0 * light) as u8,
                            (hue.1 * light) as u8,
                            (hue.2 * light) as u8,
                        )
                    })
                    .collect(),
            );
            covers.tints.insert(key.clone(), ambient::tint_of(&image));
            covers.textures.insert(key.clone(), ctx.load_texture(key.clone(), image, TextureOptions::LINEAR));
            key
        };
        let followed: Vec<ArtistSummary> = ["Atlas Sud", "Kaito", "Les Ondes"]
            .iter()
            .enumerate()
            .map(|(i, name)| ArtistSummary {
                id: format!("f{i}"),
                name: (*name).into(),
                image: Some(portrait(ctx, &mut self.covers, i)),
                liked: 0,
            })
            .collect();
        let library = crate::model::artists_by_count(&tracks, &followed, 36);
        self.pages.insert(
            ViewKey::Artists,
            Page::Artists(ArtistsPage { followed, library, problem: None, needs_auth: false }),
        );
        let liked: Vec<Track> = tracks.iter().filter(|t| t.artists[0].id == "a2").take(6).cloned().collect();
        let albums = (0..7)
            .map(|i| AlbumSummary {
                id: format!("al{i}"),
                name: ["Rivages", "Marées", "Phares", "Écume", "Brise", "Sable", "Vagues"][i].into(),
                artists: "Kaito".into(),
                year: format!("{}", 2025 - i * 2),
                total_tracks: 6 + i as u32 * 2,
                image: if i == 0 { Some("demo-cover".into()) } else { None },
                cover: None,
            })
            .collect();
        self.pages.insert(
            ViewKey::Artist("a2".into()),
            Page::Artist {
                name: "Kaito".into(),
                image: Some(portrait(ctx, &mut self.covers, 1)),
                liked: Arc::new(liked),
                albums,
            },
        );
        if let Ok(view) = std::env::var("SPOTILITE_DEMO_VIEW") {
            self.view = match view.as_str() {
                "settings" => ViewKey::Settings,
                "queue" => ViewKey::Queue,
                "artists" => ViewKey::Artists,
                "artist" => ViewKey::Artist("a2".into()),
                "home" => ViewKey::Home,
                "playlist" => ViewKey::Playlist("p0".into()),
                _ => ViewKey::Liked,
            };
            self.history.push(ViewKey::Liked);
        }
        self
    }

    fn send(&self, command: Command) {
        self.backend.send(command);
    }

    // ------------------------------------------------------------------
    // Navigation

    fn navigate(&mut self, view: ViewKey) {
        self.navigate_with(view, false);
    }

    fn navigate_with(&mut self, view: ViewKey, force: bool) {
        if view != self.view {
            self.history.push(std::mem::replace(&mut self.view, view.clone()));
            if self.history.len() > 50 {
                self.history.remove(0);
            }
            self.filter.clear();
            self.selected = None;
        }
        self.failures.remove(&view);
        match &view {
            ViewKey::Home | ViewKey::Settings | ViewKey::Queue => {}
            _ => self.send(Command::Open { view, force }),
        }
    }

    fn back(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.view = previous.clone();
            self.filter.clear();
            self.selected = None;
            if !self.pages.contains_key(&previous) {
                self.failures.remove(&previous);
                if !matches!(previous, ViewKey::Home | ViewKey::Settings | ViewKey::Queue) {
                    self.send(Command::Open { view: previous, force: false });
                }
            }
        }
    }

    fn store_page(&mut self, view: ViewKey, page: Page) {
        self.loading.remove(&view);
        self.failures.remove(&view);
        self.page_order.retain(|v| v != &view);
        self.page_order.push_back(view.clone());
        self.pages.insert(view, page);
        while self.page_order.len() > MAX_PAGES {
            if let Some(old) = self.page_order.pop_front() {
                if old == self.view {
                    self.page_order.push_back(old);
                    break;
                }
                self.pages.remove(&old);
            }
        }
    }

    // ------------------------------------------------------------------
    // Backend events

    fn drain_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.backend.events.try_recv() {
            self.on_event(ctx, event);
        }
    }

    fn on_event(&mut self, ctx: &egui::Context, event: Event) {
        if self.demo && !matches!(event, Event::Info(_) | Event::Error(_)) {
            return;
        }
        match event {
            Event::Info(text) => self.toast(text, false),
            Event::Error(text) => self.toast(text, true),
            Event::Notice { key, text, error } => self.toast_keyed(Some(key), text, error),
            Event::LoggedIn { user } => self.user = user,
            Event::App(status) => self.on_app_status(status),
            Event::Playlists(list) => {
                self.playlists = list;
                self.playlists_loading = None;
            }
            Event::Loading(view) => {
                self.failures.remove(&view);
                self.loading.insert(view);
            }
            Event::Tracks { view, title, subtitle, cover, tracks } => {
                self.store_page(view, Page::Tracks { title, subtitle, cover, tracks });
            }
            Event::PlaylistContext { view, title, subtitle, cover, uri, total } => {
                self.store_page(view, Page::Context { title, subtitle, cover, uri, total });
            }
            Event::Albums { view, title, albums } => self.store_page(view, Page::Albums { title, albums }),
            Event::Artists(page) => self.store_page(ViewKey::Artists, Page::Artists(page)),
            Event::Artist { id, name, image, liked, albums } => {
                self.store_page(ViewKey::Artist(id), Page::Artist { name, image, liked, albums });
            }
            Event::Authorized => {
                // What waited for the new authorization loads now.
                let waiting =
                    matches!(self.pages.get(&self.view), Some(Page::Artists(page)) if page.needs_auth);
                if waiting || self.failures.contains_key(&self.view) {
                    self.failures.remove(&self.view);
                    self.send(Command::Open { view: self.view.clone(), force: true });
                }
            }
            Event::Search(results) => {
                self.store_page(ViewKey::Search(results.query.clone()), Page::Search(results));
            }
            Event::ViewFailed { view, message } => {
                self.loading.remove(&view);
                self.failures.insert(view, message);
            }
            Event::NowPlaying(track) => {
                if let Some(track) = &track {
                    self.remember_played(track);
                }
                self.player.now = track;
                self.media.set_metadata(self.player.now.as_ref(), self.cover_file());
            }
            Event::Playback { playing, buffering, position_ms } => {
                self.player.playing = playing;
                self.player.buffering = buffering;
                self.player.position_ms = position_ms;
                self.player.at = Some(Instant::now());
                self.media.set_playback(playing, position_ms);
            }
            Event::Queue { upcoming, shuffle, repeat, context } => {
                self.player.upcoming = upcoming;
                self.player.context = context;
                self.player.shuffle = shuffle;
                self.player.repeat = repeat;
            }
            Event::Liked { track_id, liked } => {
                self.player.liked.insert(track_id, liked);
            }
            Event::Image { url, image } => {
                if let Some(image) = image {
                    self.covers.tints.insert(url.clone(), ambient::tint_of(&image));
                    let texture = ctx.load_texture(url.clone(), image, TextureOptions::LINEAR);
                    self.covers.textures.insert(url.clone(), texture);
                    self.covers.order.push_back(url);
                    while self.covers.order.len() > MAX_TEXTURES {
                        if let Some(old) = self.covers.order.pop_front() {
                            self.covers.textures.remove(&old);
                            self.covers.tints.remove(&old);
                            self.covers.requested.remove(&old);
                        }
                    }
                    self.media.set_metadata(self.player.now.as_ref(), self.cover_file());
                }
            }
            Event::DataUsage { api_bytes, audio_bytes } => self.usage = (api_bytes, audio_bytes),
            Event::EngineStatus(text) => self.engine_status = text,
            Event::EngineMemory(bytes) => self.engine_memory = bytes,
        }
    }

    pub(super) fn toast(&mut self, text: String, error: bool) {
        self.toast_keyed(None, text, error);
    }

    fn toast_keyed(&mut self, key: Option<&'static str>, text: String, error: bool) {
        if error {
            log::warn!("{text}");
        }
        self.toasts.retain(|t| t.text != text && (key.is_none() || t.key != key));
        self.toasts.push(Toast { key, text, error, at: Instant::now() });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
    }

    fn cover_file(&self) -> Option<String> {
        let url = self.player.now.as_ref()?.image.as_ref()?;
        let name = url.rsplit('/').next()?;
        let path = self.paths.image_cache().join(format!("{name}.jpg"));
        path.exists().then(|| path.to_string_lossy().into_owned())
    }

    /// The liked tracks picture (a white heart on a violet to mint gradient),
    /// decoded the first time it is shown; its colors also paint the banner.
    fn liked_art(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        static ART: &[u8] = include_bytes!("../../assets/liked.jpg");
        if self.liked_art.is_none() || !self.covers.tints.contains_key(ambient::LIKED) {
            let rgba = image::load_from_memory(ART).ok()?.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            self.covers.tints.insert(ambient::LIKED.to_string(), ambient::tint_of(&image));
            if self.liked_art.is_none() {
                self.liked_art = Some(ctx.load_texture("liked", image, TextureOptions::LINEAR));
            }
        }
        self.liked_art.as_ref().map(TextureHandle::id)
    }

    /// Puts a track at the top of the recently played ones (saved at once).
    fn remember_played(&mut self, track: &Track) {
        if self.demo || track.id.is_empty() || self.recent.first().is_some_and(|t| t.id == track.id) {
            return;
        }
        let mut recent = Vec::with_capacity(MAX_RECENT);
        recent.push(track.clone());
        recent.extend(self.recent.iter().filter(|t| t.id != track.id).take(MAX_RECENT - 1).cloned());
        self.recent = Arc::new(recent);
        match serde_json::to_vec(&*self.recent) {
            Ok(bytes) => {
                if let Err(e) = crate::config::write_atomic(&self.paths.recent_file(), &bytes) {
                    log::warn!("could not save recent tracks: {e}");
                }
            }
            Err(e) => log::warn!("could not serialize recent tracks: {e}"),
        }
    }

    /// Forgets the recently played tracks (logout).
    fn forget_played(&mut self) {
        self.recent = Arc::default();
        let _ = std::fs::remove_file(self.paths.recent_file());
    }

    /// Memory of SpotiLite and of its player, together.
    fn total_memory(&self) -> u64 {
        self.memory.private_working_set + self.engine_memory
    }

    /// Blurred gradient of the current track's cover, for the player bar.
    fn player_ambient(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let url = self.player.now.as_ref()?.image.clone()?;
        self.ambient(ctx, &url, Ambient::Player)
    }

    /// Blurred gradient made from the image at `url` (once it is loaded).
    fn ambient(&mut self, ctx: &egui::Context, url: &String, slot: Ambient) -> Option<egui::TextureId> {
        if !self.settings.show_covers {
            return None;
        }
        if let Some((current, texture)) = &self.covers.ambient[slot as usize]
            && current == url
        {
            return Some(texture.id());
        }
        if url == ambient::LIKED && !self.covers.tints.contains_key(url) {
            // The colors come from the picture.
            self.liked_art(ctx);
        }
        let Some(tint) = self.covers.tints.get(url) else {
            // Arrives with the image itself.
            self.cover(Some(url));
            return None;
        };
        let name = format!("ambient-{}", slot as usize);
        let texture = ctx.load_texture(name, ambient::ambient_image(tint), TextureOptions::LINEAR);
        let id = texture.id();
        self.covers.ambient[slot as usize] = Some((url.clone(), texture));
        Some(id)
    }

    /// Returns the cover texture for `url`, asking the backend for it if needed.
    fn cover(&mut self, url: Option<&String>) -> Option<egui::TextureId> {
        let url = url?;
        if let Some(texture) = self.covers.textures.get(url) {
            // Most recently used last: what is on screen is evicted last.
            if self.covers.order.back() != Some(url)
                && let Some(i) = self.covers.order.iter().position(|u| u == url)
            {
                let used = self.covers.order.remove(i).unwrap_or_default();
                self.covers.order.push_back(used);
            }
            return Some(texture.id());
        }
        if self.settings.show_covers && self.covers.requested.insert(url.clone()) {
            self.send(Command::FetchImage(url.clone()));
        }
        None
    }

    // ------------------------------------------------------------------
    // Playback helpers

    fn play(&mut self, tracks: Arc<Vec<Track>>, index: usize) {
        self.send(Command::Play { tracks, index });
    }

    /// Lets Spotify play a whole playlist or artist.
    fn play_context(&mut self, uri: String, title: String, shuffle: bool, total: u32) {
        self.player.shuffle = shuffle;
        self.settings.shuffle = shuffle;
        self.send(Command::PlayContext { uri, title, shuffle, total });
    }

    fn toggle_like_current(&mut self) {
        if let Some(track) = self.player.now.clone() {
            let liked = !self.player.is_liked().unwrap_or(false);
            self.send(Command::SetLiked { track, liked });
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.settings.volume = volume.clamp(0.0, 1.0);
        self.send(Command::SetVolume(self.settings.volume));
    }

    fn on_app_status(&mut self, status: AppStatus) {
        let connected_now =
            status.state == AppState::Connected && self.app_status.state != AppState::Connected;
        if self.setup_id.is_empty() {
            self.setup_id = status.client_id.clone();
        }
        self.app_status = status;
        if connected_now {
            // Everything that failed while the application was missing loads now.
            self.editing_app = false;
            self.setup_secret.clear();
            self.failures.clear();
            self.loading.clear();
            if !matches!(self.view, ViewKey::Home | ViewKey::Settings | ViewKey::Queue) {
                self.send(Command::Open { view: self.view.clone(), force: false });
            }
        }
    }

    /// The setup screen replaces the library until the user's application works.
    pub fn needs_setup(&self) -> bool {
        self.editing_app
            || matches!(
                self.app_status.state,
                AppState::NotConfigured | AppState::Disconnected | AppState::Authorizing
            )
    }

    fn apply_settings(&mut self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.settings.ui_scale);
        self.settings.save(&self.paths);
        self.send(Command::ApplySettings(Box::new(self.settings.clone())));
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.needs_setup() {
            return;
        }
        let typing = ctx.text_edit_focused();
        let mut actions = Vec::new();
        ctx.input_mut(|i| {
            if !typing && i.consume_key(Modifiers::NONE, Key::Space) {
                actions.push("toggle");
            }
            if i.consume_key(Modifiers::COMMAND, Key::ArrowRight) {
                actions.push("next");
            }
            if i.consume_key(Modifiers::COMMAND, Key::ArrowLeft) {
                actions.push("prev");
            }
            if i.consume_key(Modifiers::COMMAND, Key::ArrowUp) {
                actions.push("vol+");
            }
            if i.consume_key(Modifiers::COMMAND, Key::ArrowDown) {
                actions.push("vol-");
            }
            if i.consume_key(Modifiers::COMMAND, Key::F) {
                actions.push("search");
            }
            if i.consume_key(Modifiers::COMMAND, Key::L) {
                actions.push("like");
            }
            if i.consume_key(Modifiers::ALT, Key::ArrowLeft)
                || i.pointer.button_pressed(egui::PointerButton::Extra1)
            {
                actions.push("back");
            }
            if !typing && i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                actions.push("down");
            }
            if !typing && i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                actions.push("up");
            }
            if !typing && i.consume_key(Modifiers::NONE, Key::Enter) {
                actions.push("enter");
            }
        });
        for action in actions {
            match action {
                "toggle" => self.send(Command::PlayPause),
                "next" => self.send(Command::Next),
                "prev" => self.send(Command::Previous),
                "vol+" => self.set_volume(self.settings.volume + 0.05),
                "vol-" => self.set_volume(self.settings.volume - 0.05),
                "search" => self.focus_search = true,
                "like" => self.toggle_like_current(),
                "back" => self.back(),
                "down" | "up" | "enter" => self.keyboard_list(action),
                _ => {}
            }
        }
    }

    fn keyboard_list(&mut self, action: &str) {
        let Some(tracks) = self.visible_tracks() else { return };
        if tracks.is_empty() {
            return;
        }
        let last = tracks.len() - 1;
        match action {
            "down" => self.selected = Some(self.selected.map_or(0, |s| (s + 1).min(last))),
            "up" => self.selected = Some(self.selected.map_or(0, |s| s.saturating_sub(1))),
            _ => {
                if let Some(index) = self.selected {
                    self.play(tracks, index.min(last));
                }
            }
        }
        self.reveal_selected = true;
    }

    /// Track list currently displayed (after filtering), if any.
    fn visible_tracks(&mut self) -> Option<Arc<Vec<Track>>> {
        let tracks = match self.pages.get(&self.view)? {
            Page::Tracks { tracks, .. } => tracks.clone(),
            Page::Artist { liked, .. } => liked.clone(),
            Page::Search(results) => results.tracks.clone(),
            Page::Albums { .. } | Page::Artists(_) | Page::Context { .. } => return None,
        };
        let query = self.filter.trim().to_lowercase();
        if query.is_empty() {
            return Some(tracks);
        }
        let up_to_date = self
            .filtered
            .as_ref()
            .is_some_and(|f| f.view == self.view && f.query == query && f.len == tracks.len());
        if !up_to_date {
            let matched: Vec<Track> = tracks
                .iter()
                .filter(|t| {
                    t.name.to_lowercase().contains(&query)
                        || t.album.to_lowercase().contains(&query)
                        || t.artists.iter().any(|a| a.name.to_lowercase().contains(&query))
                })
                .cloned()
                .collect();
            self.filtered = Some(Filtered {
                view: self.view.clone(),
                query,
                len: tracks.len(),
                tracks: Arc::new(matched),
            });
        }
        self.filtered.as_ref().map(|f| f.tracks.clone())
    }

    fn housekeeping(&mut self, ctx: &egui::Context) {
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized && !self.minimized && self.settings.trim_when_minimized {
            // Give memory back to Windows while the window is not visible.
            self.covers.textures.clear();
            self.covers.tints.clear();
            self.covers.order.clear();
            self.covers.requested.clear();
            self.covers.ambient = Default::default();
            self.liked_art = None;
            sys::trim_working_set();
        }
        self.minimized = minimized;
        if self.memory_at.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.memory = sys::memory();
            self.memory_at = Some(Instant::now());
        }
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.error { 7 } else { 4 }));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if self.player.playing && !minimized {
            // One repaint per second is enough for the clock and the progress bar.
            let into_second = 1000 - self.player.position() % 1000;
            ctx.request_repaint_after(Duration::from_millis(u64::from(into_second) + 5));
        }
    }

    fn handle_media_keys(&mut self, ctx: &egui::Context) {
        use crate::media::MediaAction;
        for action in self.media.poll() {
            log::info!("media key: {action:?}");
            match action {
                MediaAction::Toggle => self.send(Command::PlayPause),
                MediaAction::Play => self.send(Command::Resume),
                MediaAction::Pause => self.send(Command::Pause),
                MediaAction::Next => self.send(Command::Next),
                MediaAction::Previous => self.send(Command::Previous),
                MediaAction::Seek(ms) => self.send(Command::Seek(ms)),
                MediaAction::Raise => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            }
        }
    }
}

impl crate::window::App for App {
    fn logic(&mut self, ctx: &egui::Context) {
        self.drain_events(ctx);
        self.handle_media_keys(ctx);
        self.housekeeping(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.settings.window_size = [rect.width() * ctx.zoom_factor(), rect.height() * ctx.zoom_factor()];
        }
        let palette = self.palette;
        let logo = self.logo.id();
        titlebar::show(ui, &palette, |ui, center, size| views::logo(ui, center, size, logo));
        #[cfg(debug_assertions)]
        if self.demo && std::env::var_os("SPOTILITE_DEMO_ICONS").is_some() {
            return views::icon_sheet(self, ui);
        }
        match self.app_status.state {
            AppState::Unknown => views::splash(self, ui),
            _ if self.needs_setup() => views::setup_screen(self, ui),
            _ => views::main_layout(self, ui),
        }
        views::toasts(self, &ctx);
        titlebar::resize_edges(&ctx);
    }

    fn on_exit(&mut self) {
        self.settings.save(&self.paths);
        self.backend.shutdown();
    }
}
