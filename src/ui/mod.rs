//! The interface: a sidebar, a content panel and a player bar.
//!
//! Rendering is reactive: nothing is redrawn unless something happens (input,
//! backend event, or once per second while music plays), so the CPU stays idle.

mod theme;
mod views;
mod widgets;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers, TextureHandle, TextureOptions};

use crate::backend::{AppState, AppStatus, Backend, Command, Event};
use crate::config::{Paths, Settings};
use crate::media::MediaKeys;
use crate::model::{AlbumSummary, PlaylistSummary, Repeat, SearchResults, Track, ViewKey};
use crate::sys;
use theme::Palette;

/// Pages kept in memory for instant back-navigation; older ones are dropped
/// (they come back from the disk cache without any download).
const MAX_PAGES: usize = 12;
const MAX_TEXTURES: usize = 48;

pub enum Page {
    Tracks { title: String, subtitle: String, tracks: Arc<Vec<Track>> },
    Albums { title: String, albums: Vec<AlbumSummary> },
    Artist { name: String, top: Arc<Vec<Track>>, albums: Vec<AlbumSummary> },
    Search(SearchResults),
}

#[derive(Clone, PartialEq)]
enum Auth {
    Unknown,
    NeedLogin,
    Pending { url: String },
    Connecting,
    LoggedIn,
    Offline,
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
    order: VecDeque<String>,
    requested: HashSet<String>,
}

pub struct App {
    backend: Backend,
    paths: Paths,
    settings: Settings,
    palette: Palette,
    auth: Auth,
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
    /// Tracks Spotify refuses to third-party clients (greyed out, skipped).
    refused: HashSet<String>,
    toasts: Vec<Toast>,
    usage: (u64, u64),
    memory: sys::Memory,
    memory_at: Option<Instant>,
    /// State of the official engine and memory of its WebView2 processes.
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
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Paths, settings: Settings) -> Self {
        let ctx = &cc.egui_ctx;
        theme::install_fonts(ctx);
        let palette = Palette::amoled();
        theme::apply(ctx, &palette);
        ctx.set_zoom_factor(settings.ui_scale);
        ctx.options_mut(|o| o.zoom_with_keyboard = true);

        let backend = Backend::spawn(ctx.clone(), paths.clone(), settings.clone());
        let media = MediaKeys::new(cc);
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
            auth: Auth::Unknown,
            user: String::new(),
            app_status: AppStatus {
                state: AppState::Unknown,
                client_id: String::new(),
                has_secret: false,
                needs_playback_auth: false,
            },
            playlists: Vec::new(),
            view: ViewKey::Welcome,
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
            refused: HashSet::new(),
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
    pub fn with_demo(mut self) -> Self {
        use crate::model::ArtistRef;
        if std::env::var_os("SPOTILITE_DEMO").is_none() {
            return self;
        }
        self.demo = true;
        self.auth = Auth::LoggedIn;
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
                image: None,
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
                })
                .collect();
        self.player.now = Some(tracks[2].clone());
        self.player.playing = true;
        self.player.position_ms = 83_000;
        self.player.at = Some(Instant::now());
        self.player.liked.insert(tracks[2].id.clone(), true);
        self.player.upcoming = tracks[3..12].to_vec();
        self.refused.insert(tracks[5].id.clone());
        self.app_status.state = if std::env::var_os("SPOTILITE_DEMO_SETUP").is_some() {
            AppState::NotConfigured
        } else {
            AppState::Connected
        };
        self.view = ViewKey::Liked;
        self.pages.insert(
            ViewKey::Liked,
            Page::Tracks { title: "Titres likés".into(), subtitle: String::new(), tracks: Arc::new(tracks) },
        );
        if std::env::var_os("SPOTILITE_DEMO_OFFICIAL").is_some() {
            self.settings.engine = crate::config::Engine::Official;
            self.engine_status = "Prêt · DRM Widevine".into();
            self.engine_memory = 118 * 1024 * 1024;
        }
        if let Ok(view) = std::env::var("SPOTILITE_DEMO_VIEW") {
            self.view = match view.as_str() {
                "settings" => ViewKey::Settings,
                "queue" => ViewKey::Queue,
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
            ViewKey::Welcome | ViewKey::Settings | ViewKey::Queue => {}
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
                if !matches!(previous, ViewKey::Welcome | ViewKey::Settings | ViewKey::Queue) {
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
            Event::Refused(ids) => self.refused = ids,
            Event::TrackRefused(id) => {
                self.refused.insert(id);
            }
            Event::NeedLogin => self.auth = Auth::NeedLogin,
            Event::LoginPending { url } => self.auth = Auth::Pending { url },
            Event::Offline => {
                if self.auth != Auth::LoggedIn {
                    self.auth = Auth::Offline;
                    if self.view == ViewKey::Welcome {
                        self.navigate(ViewKey::Liked);
                    }
                }
            }
            Event::Connecting => {
                if !matches!(self.auth, Auth::LoggedIn | Auth::Offline) {
                    self.auth = Auth::Connecting;
                }
            }
            Event::LoggedIn { user } => {
                let first = self.auth != Auth::LoggedIn;
                self.auth = Auth::LoggedIn;
                self.user = user;
                if first && self.view == ViewKey::Welcome {
                    self.navigate(ViewKey::Liked);
                }
            }
            Event::App(status) => self.on_app_status(status),
            Event::Playlists(list) => self.playlists = list,
            Event::Loading(view) => {
                self.failures.remove(&view);
                self.loading.insert(view);
            }
            Event::Tracks { view, title, subtitle, tracks } => {
                self.store_page(view, Page::Tracks { title, subtitle, tracks });
            }
            Event::Albums { view, title, albums } => self.store_page(view, Page::Albums { title, albums }),
            Event::Artist { id, name, top, albums } => {
                self.store_page(ViewKey::Artist(id), Page::Artist { name, top, albums });
            }
            Event::Search(results) => {
                self.store_page(ViewKey::Search(results.query.clone()), Page::Search(results));
            }
            Event::ViewFailed { view, message } => {
                self.loading.remove(&view);
                self.failures.insert(view, message);
            }
            Event::NowPlaying(track) => {
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
            Event::Queue { upcoming, shuffle, repeat } => {
                self.player.upcoming = upcoming;
                self.player.shuffle = shuffle;
                self.player.repeat = repeat;
            }
            Event::Liked { track_id, liked } => {
                self.player.liked.insert(track_id, liked);
            }
            Event::Image { url, image } => {
                if let Some(image) = image {
                    let texture = ctx.load_texture(url.clone(), image, TextureOptions::LINEAR);
                    self.covers.textures.insert(url.clone(), texture);
                    self.covers.order.push_back(url);
                    while self.covers.order.len() > MAX_TEXTURES {
                        if let Some(old) = self.covers.order.pop_front() {
                            self.covers.textures.remove(&old);
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

    /// Returns the cover texture for `url`, asking the backend for it if needed.
    fn cover(&mut self, url: Option<&String>) -> Option<egui::TextureId> {
        let url = url?;
        if let Some(texture) = self.covers.textures.get(url) {
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
            if self.view == ViewKey::Welcome {
                self.navigate(ViewKey::Liked);
            } else if !matches!(self.view, ViewKey::Settings | ViewKey::Queue) {
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
        if !matches!(self.auth, Auth::LoggedIn | Auth::Offline) {
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
            Page::Artist { top, .. } => top.clone(),
            Page::Search(results) => results.tracks.clone(),
            Page::Albums { .. } => return None,
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
            self.covers.order.clear();
            self.covers.requested.clear();
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
            match action {
                MediaAction::Toggle => self.send(Command::PlayPause),
                MediaAction::Play => {
                    if !self.player.playing {
                        self.send(Command::PlayPause);
                    }
                }
                MediaAction::Pause => {
                    if self.player.playing {
                        self.send(Command::PlayPause);
                    }
                }
                MediaAction::Next => self.send(Command::Next),
                MediaAction::Previous => self.send(Command::Previous),
                MediaAction::Seek(ms) => self.send(Command::Seek(ms)),
                MediaAction::Raise => ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            }
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events(ctx);
        self.handle_media_keys(ctx);
        self.housekeeping(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_keys(&ctx);
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            self.settings.window_size = [rect.width() * ctx.zoom_factor(), rect.height() * ctx.zoom_factor()];
        }
        match self.auth {
            Auth::LoggedIn | Auth::Offline if self.needs_setup() => views::setup_screen(self, ui),
            Auth::LoggedIn | Auth::Offline => views::main_layout(self, ui),
            _ => views::login_screen(self, ui),
        }
        views::toasts(self, &ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.bg.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.settings.save(&self.paths);
        self.backend.shutdown();
    }
}
