//! Backend thread: Web API, playback engine and caches.
//!
//! The UI sends [`Command`]s and receives [`Event`]s. Everything network related
//! runs here, on a single-threaded Tokio runtime, so the interface never blocks.
//! Sound comes from the playback engine (Spotify's own player in an invisible
//! WebView2, see [`engine`]), started on demand and stopped when idle.

mod auth;
mod engine;
mod store;
mod vault;
mod webapi;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::config::{Paths, Settings};
use crate::model::{
    AlbumSummary, ArtistSummary, ArtistsPage, PlaylistSummary, Repeat, SearchResults, Track, ViewKey,
    artists_by_count,
};
use crate::queue::Queue;
pub use auth::AppCredentials;
use auth::{AuthFlow, OAuthToken};
use engine::{EngineCommand, EngineEvent, ErrorKind, PlayerState, Profile, Progress, Tracker};
use store::Store;
use webapi::{ApiError, Offset, PlayRequest, WebApi};

/// Requests from the interface.
pub enum Command {
    /// Saves the user's Spotify application and authorizes it in the browser.
    SetupApp(AppCredentials),
    /// Authorizes the saved application again.
    ReconnectApp,
    CancelAppLogin,
    /// Disconnects the application and deletes its saved credentials.
    ForgetApp,
    /// Forgets the authorization and the cached library (the application stays saved).
    Logout,
    LoadPlaylists,
    Open {
        view: ViewKey,
        force: bool,
    },
    Play {
        tracks: Arc<Vec<Track>>,
        index: usize,
    },
    /// Lets Spotify play a whole playlist or artist (`spotify:playlist:…`).
    PlayContext {
        uri: String,
        title: String,
        shuffle: bool,
        /// Number of tracks, when known (random start for shuffle).
        total: u32,
    },
    PlayPause,
    /// Explicit pause and play (media keys, Windows media panel): never a toggle,
    /// so that they act right even if the shown state is out of date.
    Pause,
    Resume,
    Next,
    Previous,
    Seek(u32),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeat(Repeat),
    Enqueue(Track),
    /// Removes the track at this position of the upcoming list (if still there).
    RemoveFromQueue {
        index: usize,
        track_id: String,
    },
    AddToPlaylist {
        playlist_id: String,
        playlist_name: String,
        track: Track,
    },
    RemoveFromPlaylist {
        playlist_id: String,
        playlist_name: String,
        track: Track,
    },
    SetLiked {
        track: Track,
        liked: bool,
    },
    FetchImage(String),
    ApplySettings(Box<Settings>),
    ClearCache,
    /// Measures the disk cache (answered with `Event::CacheSize`).
    MeasureCache,
    Shutdown,
}

/// State of the user's own Spotify application, which serves the library and playback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppState {
    /// Unknown until the backend has read the saved credentials.
    Unknown,
    NotConfigured,
    /// Waiting for the user to accept in the browser.
    Authorizing,
    Disconnected,
    Connected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppStatus {
    pub state: AppState,
    pub client_id: String,
    pub has_secret: bool,
    /// The authorization predates playback: it lacks the playback scopes.
    pub needs_playback_auth: bool,
}

/// Notifications for the interface.
pub enum Event {
    Info(String),
    Error(String),
    /// A message that replaces the previous one with the same key.
    Notice {
        key: &'static str,
        text: String,
        error: bool,
    },
    LoggedIn {
        user: String,
        /// Spotify id of the user.
        id: String,
    },
    App(AppStatus),
    Playlists(Vec<PlaylistSummary>),
    Loading(ViewKey),
    Tracks {
        view: ViewKey,
        title: String,
        subtitle: String,
        /// Cover shown in the page banner (album or playlist).
        cover: Option<String>,
        tracks: Arc<Vec<Track>>,
    },
    /// A playlist whose tracks Spotify does not give to development mode
    /// applications: it can only be played as a whole, by Spotify.
    PlaylistContext {
        view: ViewKey,
        title: String,
        subtitle: String,
        cover: Option<String>,
        uri: String,
        total: u32,
    },
    Albums {
        view: ViewKey,
        title: String,
        albums: Vec<AlbumSummary>,
    },
    Artists(ArtistsPage),
    /// An authorization was accepted again (new scopes).
    Authorized,
    Artist {
        id: String,
        name: String,
        image: Option<String>,
        /// The user's liked tracks by this artist.
        liked: Arc<Vec<Track>>,
        albums: Vec<AlbumSummary>,
    },
    Search(SearchResults),
    ViewFailed {
        view: ViewKey,
        message: String,
    },
    NowPlaying(Option<Track>),
    Playback {
        playing: bool,
        buffering: bool,
        position_ms: u32,
    },
    Queue {
        upcoming: Vec<Track>,
        shuffle: bool,
        repeat: Repeat,
        /// Name of the playlist or artist when Spotify chooses the tracks itself.
        context: Option<String>,
    },
    Liked {
        track_id: String,
        liked: bool,
    },
    Image {
        url: String,
        image: Option<egui::ColorImage>,
    },
    DataUsage {
        api_bytes: u64,
        audio_bytes: u64,
    },
    /// What the playback engine is doing, for the settings page.
    EngineStatus(String),
    /// Memory of the engine's WebView2 processes (0 when stopped).
    EngineMemory(u64),
    /// Size of the disk cache: covers and library.
    CacheSize(u64),
}

/// Event sender that also wakes the UI up.
#[derive(Clone)]
struct UiTx {
    tx: std::sync::mpsc::Sender<Event>,
    ctx: egui::Context,
}

impl UiTx {
    fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    fn error(&self, message: impl Into<String>) {
        self.send(Event::Error(message.into()));
    }
}

pub struct Backend {
    pub commands: UnboundedSender<Command>,
    pub events: std::sync::mpsc::Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Backend {
    pub fn spawn(ctx: egui::Context, paths: Paths, settings: Settings) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (ev_tx, ev_rx) = std::sync::mpsc::channel();
        let ui = UiTx { tx: ev_tx, ctx };
        let thread = std::thread::Builder::new()
            .name("spotilite-core".into())
            .spawn(move || {
                // One thread is plenty for a few HTTP requests at a time.
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .max_blocking_threads(2)
                    .thread_name("spotilite-io")
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                runtime.block_on(async move {
                    let (internal_tx, internal_rx) = unbounded_channel();
                    let core = Core::new(ui, internal_tx, paths, settings);
                    core.run(cmd_rx, internal_rx).await;
                });
                runtime.shutdown_timeout(Duration::from_secs(2));
            })
            .expect("backend thread");
        Self { commands: cmd_tx, events: ev_rx, thread: Some(thread) }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Internal {
    AppToken(Result<(AppCredentials, OAuthToken), String>),
    LikedIds(HashSet<String>),
    Playlists(Vec<PlaylistSummary>),
    SearchDone(SearchResults),
    /// Delayed move to the next track after an unplayable one (carries the load
    /// generation so that any newer user action cancels it).
    SkipAfterFailure(u64),
    /// Event of the engine instance `.0`.
    Engine(u64, EngineEvent),
    /// Starting playback failed (load generation, error).
    PlayFailed(u64, ApiError),
    /// Checks that the engine followed pause (false) or play (true) command `.0`.
    CheckFollowed(u64, bool),
    /// A playlist changed (its summary is read again).
    PlaylistsChanged,
    /// A track was removed from a playlist (playlist id, track id).
    RemovedFromPlaylist(String, String),
    /// Playback scopes missing from the authorization (`.1`: the browser may be opened).
    Scopes(Vec<&'static str>, bool),
    TokenFailed(String),
}

#[derive(Serialize, Deserialize)]
struct CachedTracks {
    snapshot: String,
    saved_at: u64,
    tracks: Vec<Track>,
}

#[derive(Serialize, Deserialize)]
struct CachedAlbum {
    album: AlbumSummary,
    tracks: Vec<Track>,
}

/// The user, as cached once logged in.
#[derive(Serialize, Deserialize)]
struct UserProfile {
    id: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct CachedArtist {
    saved_at: u64,
    name: String,
    #[serde(default)]
    image: Option<String>,
    albums: Vec<AlbumSummary>,
}

/// Artists of the liked tracks shown under the followed ones.
const LIBRARY_ARTISTS: usize = 36;

fn liked_cache(store: &Store) -> Vec<Track> {
    store.load::<CachedTracks>("liked").map(|c| c.tracks).unwrap_or_default()
}

fn artists_page(
    liked: &[Track],
    followed: Vec<ArtistSummary>,
    problem: Option<String>,
    needs_auth: bool,
) -> ArtistsPage {
    let library = artists_by_count(liked, &followed, LIBRARY_ARTISTS);
    ArtistsPage { followed, library, problem, needs_auth }
}

/// The user's liked tracks by an artist (no request: from the cache).
fn liked_by(store: &Store, artist: &str) -> Arc<Vec<Track>> {
    let mut tracks = liked_cache(store);
    tracks.retain(|t| t.artists.iter().any(|a| a.id == artist));
    Arc::new(tracks)
}

/// Pause after which the engine is put to sleep (when enabled).
const ENGINE_SLEEP: Duration = Duration::from_secs(5 * 60);

/// Bitrate assumed when the engine's traffic cannot be measured.
const ESTIMATED_KBPS: u64 = 128;
/// Present when the lean engine profile could not play on this PC.
const COMPATIBLE_MARKER: &str = "engine-compatible";

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A whole playlist or artist played by Spotify itself.
struct Remote {
    uri: String,
    title: String,
    current: Option<Track>,
    upcoming: Vec<Track>,
    started: bool,
}

struct Core {
    ui: UiTx,
    internal: UnboundedSender<Internal>,
    paths: Paths,
    settings: Settings,
    http: reqwest::Client,
    store: Store,
    api_bytes: Arc<AtomicU64>,
    api: Arc<WebApi>,
    /// The user's Spotify application is authorized.
    app_connected: bool,
    app_login: Option<Arc<AtomicBool>>,
    queue: Queue,
    /// Set while Spotify chooses the tracks (playlist of another user, artist).
    remote: Option<Remote>,
    playing: bool,
    position_ms: u32,
    position_at: Instant,
    /// A play request was sent for the current track.
    loaded: bool,
    failure_streak: u32,
    /// Incremented by every load: identifies the attempt a delayed action belongs to.
    load_generation: u64,
    load_started: Instant,
    listened_ms: u64,
    last_usage: (u64, u64),
    playlists: HashMap<String, PlaylistSummary>,
    liked_ids: Option<HashSet<String>>,
    refreshed: HashMap<ViewKey, Instant>,
    searches: VecDeque<SearchResults>,
    /// Spotify's player in WebView2, started on demand.
    engine: Option<engine::Engine>,
    /// Instance number, so that events of a stopped engine are ignored.
    engine_id: u64,
    /// Spotify device id of the engine, once registered.
    device_id: Option<String>,
    /// Playback waiting for the engine to be ready (load generation, request).
    pending: Option<(u64, PlayRequest)>,
    tracker: Tracker,
    engine_bytes: u64,
    /// Something played on the current engine instance.
    engine_played: bool,
    /// Repeat already switched off on the device (one track at a time).
    device_prepared: bool,
    /// The lean profile failed on this PC: WebView2 defaults are used.
    compatible: bool,
    /// Load generation for which a stalled start was already handled.
    stalled: u64,
    /// Paused state last reported by the engine (None before any report).
    engine_paused: Option<bool>,
    /// Number of the last pause or play command (for `CheckFollowed`).
    control: u64,
    last_active: Instant,
    /// The authorization lacks the playback scopes.
    needs_playback_auth: bool,
    /// The browser was already opened once for those scopes in this session.
    playback_auth_asked: bool,
    drm: String,
}

impl Core {
    fn new(ui: UiTx, internal: UnboundedSender<Internal>, paths: Paths, settings: Settings) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("SpotiLite/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        let store = Store::new(paths.data_cache(), paths.image_cache());
        let api_bytes = Arc::new(AtomicU64::new(0));
        let api = Arc::new(WebApi::new(http.clone(), paths.clone(), api_bytes.clone()));
        let mut queue = Queue::default();
        queue.set_shuffle(settings.shuffle);
        queue.set_repeat(settings.repeat);
        let compatible = paths.cache.join(COMPATIBLE_MARKER).exists();
        Self {
            ui,
            internal,
            http,
            store,
            api_bytes,
            api,
            app_connected: false,
            app_login: None,
            queue,
            remote: None,
            playing: false,
            position_ms: 0,
            position_at: Instant::now(),
            loaded: false,
            failure_streak: 0,
            load_generation: 0,
            load_started: Instant::now(),
            listened_ms: 0,
            last_usage: (u64::MAX, u64::MAX),
            playlists: HashMap::new(),
            liked_ids: None,
            refreshed: HashMap::new(),
            searches: VecDeque::new(),
            engine: None,
            engine_id: 0,
            device_id: None,
            pending: None,
            tracker: Tracker::default(),
            engine_bytes: 0,
            engine_played: false,
            device_prepared: false,
            compatible,
            stalled: 0,
            engine_paused: None,
            control: 0,
            last_active: Instant::now(),
            needs_playback_auth: false,
            playback_auth_asked: false,
            drm: String::new(),
            paths,
            settings,
        }
    }

    async fn run(
        mut self,
        mut commands: UnboundedReceiver<Command>,
        mut internal: UnboundedReceiver<Internal>,
    ) {
        self.startup().await;
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                cmd = commands.recv() => match cmd {
                    Some(Command::Shutdown) | None => break,
                    Some(cmd) => self.handle(cmd).await,
                },
                Some(msg) = internal.recv() => self.handle_internal(msg).await,
                _ = tick.tick() => self.tick(),
            }
        }
        for cancel in self.app_login.iter() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.engine = None;
    }

    async fn startup(&mut self) {
        let store = self.store.clone();
        let legacy = [self.paths.legacy_audio_cache(), self.paths.legacy_tmp()];
        let credentials = self.paths.config.join("credentials.json");
        tokio::task::spawn_blocking(move || {
            store.prune();
            // Leftovers of version 0.1 (librespot): audio cache (up to 4 GB) and session.
            for dir in legacy {
                let _ = std::fs::remove_dir_all(dir);
            }
            let _ = std::fs::remove_file(credentials);
        });
        if let Some(list) = self.store.load::<Vec<PlaylistSummary>>("playlists") {
            self.remember_playlists(&list);
            self.ui.send(Event::Playlists(list));
        }
        self.send_queue();
        // Version 0.1 kept the client id in the settings: move it to the vault.
        if self.api.saved_app().is_none() && !self.settings.legacy_client_id.is_empty() {
            self.api.save_app(&AppCredentials {
                client_id: self.settings.legacy_client_id.clone(),
                client_secret: String::new(),
            });
        }
        self.app_connected = self.api.restore().await;
        self.send_app_status();
        if self.app_connected {
            if let Some(profile) = self.store.load::<UserProfile>("profile") {
                self.ui.send(Event::LoggedIn { user: profile.name, id: profile.id });
            }
            self.after_connect();
        }
    }

    /// Everything that needs the authorized application.
    fn after_connect(&mut self) {
        self.load_playlists();
        self.fetch_display_name();
        if let Some(liked) = self.store.load::<CachedTracks>("liked") {
            self.liked_ids = Some(liked.tracks.into_iter().map(|t| t.id).collect());
        }
        self.preload_liked();
        self.check_playback_scopes(false);
    }

    /// Reads the liked tracks in the background (only the first page when the
    /// cache is up to date): the artist pages and the Artists page use them.
    fn preload_liked(&self) {
        let Some(api) = self.api() else { return };
        let (store, internal) = (self.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            let cached = store.load::<CachedTracks>("liked");
            let previous = cached.as_ref().map(|c| c.tracks.as_slice());
            match api.liked_tracks(previous).await {
                Ok(tracks) => {
                    let _ = internal.send(Internal::LikedIds(tracks.iter().map(|t| t.id.clone()).collect()));
                    if previous != Some(tracks.as_slice()) {
                        store.save(
                            "liked",
                            &CachedTracks { snapshot: String::new(), saved_at: now_secs(), tracks },
                        );
                    }
                }
                Err(e) => log::info!("liked tracks not preloaded: {e}"),
            }
        });
    }

    // ----------------------------------------------------------------------
    // The user's Spotify application

    /// Opens the authorization of the user's Spotify application in the browser.
    fn start_app_login(&mut self, app: AppCredentials) {
        if let Some(cancel) = self.app_login.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let flow = match AuthFlow::start(app.clone(), self.settings.redirect_port, auth::WEB_API_SCOPES) {
            Ok(flow) => flow,
            Err(e) => {
                self.send_app_status();
                return self.ui.error(format!(
                    "Impossible d'écouter sur le port {} : {e}. Changez le port dans les réglages (et dans le tableau de bord Spotify).",
                    self.settings.redirect_port
                ));
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.app_login = Some(cancel.clone());
        self.send_app_status();
        let _ = open::that_detached(&flow.auth_url);
        let (http, internal) = (self.http.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = run_flow(flow, cancel, http).await.map(|token| (app, token));
            let _ = internal.send(Internal::AppToken(result));
        });
    }

    fn send_app_status(&self) {
        let saved = self.api.saved_app();
        // Authorizing again an application that works (playback scopes) keeps the
        // interface usable: only a first authorization shows the setup screen.
        let state = if self.app_login.is_some() && !self.app_connected {
            AppState::Authorizing
        } else if self.app_connected {
            AppState::Connected
        } else if saved.is_some() {
            AppState::Disconnected
        } else {
            AppState::NotConfigured
        };
        self.ui.send(Event::App(AppStatus {
            state,
            has_secret: saved.as_ref().is_some_and(|a| !a.client_secret.is_empty()),
            client_id: saved.map(|a| a.client_id).unwrap_or_default(),
            needs_playback_auth: self.needs_playback_auth && state == AppState::Connected,
        }));
    }

    /// The account name shown in the interface (one small request, cached).
    fn fetch_display_name(&self) {
        let Some(api) = self.api() else { return };
        if self.store.load::<UserProfile>("profile").is_some() {
            return;
        }
        let (ui, store) = (self.ui.clone(), self.store.clone());
        tokio::spawn(async move {
            if let Ok((id, name)) = api.me().await {
                store.save("profile", &UserProfile { id: id.clone(), name: name.clone() });
                ui.send(Event::LoggedIn { user: name, id });
            }
        });
    }

    fn remove_from_playlist(&self, playlist_id: String, playlist_name: String, track: Track) {
        let Some(api) = self.api() else { return };
        let (ui, internal) = (self.ui.clone(), self.internal.clone());
        tokio::spawn(async move {
            match api.remove_from_playlist(&playlist_id, &track.id).await {
                Ok(()) => {
                    ui.send(Event::Info(format!("« {} » retiré de « {playlist_name} »", track.name)));
                    let _ = internal.send(Internal::RemovedFromPlaylist(playlist_id, track.id));
                }
                Err(e) if e.is_missing_scope() => {
                    let _ = internal.send(Internal::Scopes(vec!["playlist-modify-private"], true));
                }
                Err(ApiError::Forbidden(_)) => ui.error("Spotify refuse de modifier cette playlist."),
                Err(e) => ui.error(format!("Retrait de « {playlist_name} » impossible : {e}")),
            }
        });
    }

    /// The page of a playlist without a removed track, at once (the cache too).
    fn drop_from_playlist_cache(&self, playlist_id: &str, track_id: &str) {
        let key = format!("playlist-{playlist_id}");
        let Some(mut cached) = self.store.load::<CachedTracks>(&key) else { return };
        cached.tracks.retain(|t| t.id != track_id);
        // The playlist's snapshot changed: the next opening reads it again from Spotify.
        cached.snapshot.clear();
        self.store.save(&key, &cached);
        let summary = self.playlists.get(playlist_id);
        self.ui.send(Event::Tracks {
            view: ViewKey::Playlist(playlist_id.to_string()),
            title: summary.map(|p| p.name.clone()).unwrap_or_default(),
            subtitle: summary.map(|p| p.owner.clone()).unwrap_or_default(),
            cover: summary.and_then(|p| p.image.clone()),
            tracks: Arc::new(cached.tracks),
        });
    }

    fn add_to_playlist(&self, playlist_id: String, playlist_name: String, track: Track) {
        let Some(api) = self.api() else { return };
        let (ui, internal) = (self.ui.clone(), self.internal.clone());
        tokio::spawn(async move {
            match api.add_to_playlist(&playlist_id, &track.id).await {
                Ok(()) => {
                    ui.send(Event::Info(format!("« {} » ajouté à « {playlist_name} »", track.name)));
                    let _ = internal.send(Internal::PlaylistsChanged);
                }
                // Authorizations given before this menu existed lack the scope.
                Err(e) if e.is_missing_scope() => {
                    let _ = internal.send(Internal::Scopes(vec!["playlist-modify-private"], true));
                }
                Err(ApiError::Forbidden(_)) => ui.error("Spotify refuse de modifier cette playlist."),
                Err(e) => ui.error(format!("Ajout à « {playlist_name} » impossible : {e}")),
            }
        });
    }

    async fn logout(&mut self) {
        self.stop_engine();
        // The application credentials stay saved: logging in again only needs one click.
        self.api.disconnect(false).await;
        self.app_connected = false;
        self.needs_playback_auth = false;
        self.send_app_status();
        self.store.clear();
        self.queue.clear();
        self.remote = None;
        self.playlists.clear();
        self.liked_ids = None;
        self.refreshed.clear();
        self.searches.clear();
        self.playing = false;
        self.loaded = false;
        self.ui.send(Event::NowPlaying(None));
        self.ui.send(Event::Playlists(Vec::new()));
        self.send_playback(false);
        self.send_queue();
    }

    /// Checks that the authorization allows playback.
    fn check_playback_scopes(&self, interactive: bool) {
        if !self.app_connected {
            return;
        }
        let (api, internal) = (self.api.clone(), self.internal.clone());
        tokio::spawn(async move {
            if let Ok(missing) = api.missing_scopes(auth::REQUIRED_SCOPES).await {
                let _ = internal.send(Internal::Scopes(missing, interactive));
            }
        });
    }

    /// The authorization of the user's application predates playback: it must be
    /// accepted again with the playback scopes.
    fn ask_playback_auth(&mut self, open_browser: bool) {
        self.needs_playback_auth = true;
        let app = self.api.saved_app();
        if open_browser
            && !self.playback_auth_asked
            && self.app_login.is_none()
            && let Some(app) = app
        {
            self.playback_auth_asked = true;
            self.ui.send(Event::Notice {
                key: "auth",
                text: "SpotiLite a besoin d'une autorisation supplémentaire de votre application Spotify : \
                       acceptez-la dans le navigateur qui vient de s'ouvrir."
                    .into(),
                error: false,
            });
            return self.start_app_login(app);
        }
        self.send_app_status();
        self.ui.send(Event::Notice {
            key: "auth",
            text: "Autorisation à renouveler : Réglages → « Autoriser ». Si Spotify refuse encore la \
                   lecture, cochez « Web Playback SDK » dans votre application (tableau de bord Spotify)."
                .into(),
            error: true,
        });
    }

    // ----------------------------------------------------------------------
    // Commands

    async fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SetupApp(app) => {
                let app = AppCredentials {
                    client_id: app.client_id.trim().to_string(),
                    client_secret: app.client_secret.trim().to_string(),
                };
                // A new application invalidates the previous authorization.
                self.stop_engine();
                self.api.disconnect(true).await;
                self.app_connected = false;
                self.api.save_app(&app);
                self.start_app_login(app);
            }
            Command::ReconnectApp => match self.api.saved_app() {
                Some(app) => {
                    if self.app_connected {
                        self.ui.send(Event::Info("Acceptez l'autorisation dans le navigateur.".into()));
                    }
                    self.start_app_login(app);
                }
                None => self.send_app_status(),
            },
            Command::CancelAppLogin => {
                if let Some(cancel) = self.app_login.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.send_app_status();
            }
            Command::ForgetApp => {
                if let Some(cancel) = self.app_login.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                self.stop_engine();
                self.api.disconnect(true).await;
                self.app_connected = false;
                self.send_app_status();
            }
            Command::Logout => self.logout().await,
            Command::LoadPlaylists => self.load_playlists(),
            Command::Open { view, force } => self.open(view, force),
            Command::Play { tracks, index } => {
                self.remote = None;
                if let Some(track) = self.queue.play_context(tracks, index) {
                    self.load(track, true, 0);
                }
            }
            Command::PlayContext { uri, title, shuffle, total } => {
                self.queue.set_shuffle(shuffle);
                let offset = (shuffle && total > 1).then(|| Offset::Position(rand::random_range(0..total)));
                self.play_remote(uri, title, offset, 0);
            }
            Command::PlayPause => self.play_pause(),
            Command::Pause => {
                if self.playing || self.engine_paused == Some(false) {
                    self.pause();
                }
            }
            Command::Resume => {
                if !self.playing {
                    self.play_pause();
                }
            }
            Command::Next => self.skip(false),
            Command::Previous => self.previous(),
            Command::Seek(ms) => self.seek_to(ms),
            Command::SetVolume(volume) => {
                self.settings.volume = volume;
                if let Some(engine) = &self.engine {
                    engine.send(EngineCommand::Volume(volume));
                }
            }
            Command::SetShuffle(on) => {
                self.queue.set_shuffle(on);
                if self.remote.is_some() {
                    self.device_call(move |api, device| async move { api.set_shuffle(&device, on).await });
                }
                self.send_queue();
            }
            Command::SetRepeat(mode) => {
                self.queue.set_repeat(mode);
                if self.remote.is_some() {
                    self.device_call(move |api, device| async move {
                        api.set_repeat(&device, repeat_state(mode)).await
                    });
                }
                self.send_queue();
            }
            Command::Enqueue(track) => {
                let name = track.name.clone();
                if self.remote.is_some() {
                    let id = track.id.clone();
                    self.device_call(move |api, device| async move { api.add_to_queue(&device, &id).await });
                } else {
                    self.queue.enqueue(track);
                    self.send_queue();
                }
                self.ui.send(Event::Info(format!("« {name} » ajouté à la file")));
            }
            Command::RemoveFromQueue { index, track_id } => {
                if self.remote.is_none() && self.queue.remove_upcoming(index, &track_id) {
                    self.send_queue();
                }
            }
            Command::AddToPlaylist { playlist_id, playlist_name, track } => {
                self.add_to_playlist(playlist_id, playlist_name, track);
            }
            Command::RemoveFromPlaylist { playlist_id, playlist_name, track } => {
                self.remove_from_playlist(playlist_id, playlist_name, track);
            }
            Command::SetLiked { track, liked } => self.set_liked(track, liked),
            Command::FetchImage(url) => self.fetch_image(url),
            Command::ApplySettings(settings) => self.apply_settings(*settings),
            Command::ClearCache => self.clear_cache(),
            Command::MeasureCache => self.measure_cache(),
            Command::Shutdown => {}
        }
    }

    async fn handle_internal(&mut self, msg: Internal) {
        match msg {
            // A cancelled flow was replaced or abandoned on purpose: its handle is
            // already gone and must not clear the one of a newer flow.
            Internal::AppToken(Err(e)) if e.contains("annulée") => {}
            Internal::AppToken(result) => {
                self.app_login = None;
                match result {
                    Ok((app, token)) => {
                        let missing = token.missing_scopes(auth::REQUIRED_SCOPES);
                        let resume = self.needs_playback_auth && missing.is_empty();
                        let first = !self.app_connected;
                        self.needs_playback_auth = !missing.is_empty();
                        self.api.connect(app, token).await;
                        self.app_connected = true;
                        self.refreshed.clear();
                        self.ui.send(Event::Info("Application Spotify connectée.".into()));
                        if first {
                            self.after_connect();
                        } else {
                            self.ui.send(Event::Authorized);
                        }
                        if resume && !self.playing {
                            // The play that needed the new authorization.
                            self.stop_engine();
                            self.play_pause();
                        }
                    }
                    Err(e) => self.ui.error(app_error_hint(&e)),
                }
                self.send_app_status();
            }
            Internal::CheckFollowed(control, playing) => self.ensure_followed(control, playing),
            Internal::PlaylistsChanged => self.load_playlists(),
            Internal::RemovedFromPlaylist(playlist_id, track_id) => {
                self.drop_from_playlist_cache(&playlist_id, &track_id);
                self.load_playlists();
            }
            Internal::SkipAfterFailure(generation) => {
                if generation == self.load_generation {
                    self.skip(false);
                }
            }
            Internal::Engine(id, event) => {
                if id == self.engine_id && self.engine.is_some() {
                    self.on_engine(event);
                }
            }
            Internal::PlayFailed(generation, error) => {
                if generation == self.load_generation {
                    self.on_play_failed(error);
                }
            }
            Internal::Scopes(missing, interactive) => {
                if missing.is_empty() {
                    if self.needs_playback_auth {
                        self.needs_playback_auth = false;
                        self.send_app_status();
                    }
                } else {
                    log::info!("authorization lacks {missing:?} for playback");
                    self.ask_playback_auth(interactive);
                }
            }
            Internal::TokenFailed(e) => {
                self.ui.error(format!("Lecture : jeton d'accès indisponible ({e})."));
            }
            Internal::LikedIds(ids) => self.liked_ids = Some(ids),
            Internal::Playlists(list) => {
                self.remember_playlists(&list);
                self.ui.send(Event::Playlists(list));
            }
            Internal::SearchDone(results) => {
                for playlist in &results.playlists {
                    self.playlists.entry(playlist.id.clone()).or_insert_with(|| playlist.clone());
                }
                self.searches.retain(|s| s.query != results.query);
                self.searches.push_front(results);
                self.searches.truncate(20);
            }
        }
    }

    fn tick(&mut self) {
        if self.api.take_lost() {
            self.app_connected = false;
            self.stop_engine();
            self.ui
                .error("L'autorisation de votre application Spotify a expiré : cliquez sur « Reconnecter ».");
            self.send_app_status();
        }
        if self.playing {
            self.last_active = Instant::now();
        }
        if self.engine.is_some() {
            let now = Instant::now();
            if self.remote.is_none() && self.tracker.overdue(now) {
                log::info!("end of track not reported");
                self.flush_listened();
                self.playing = false;
                self.skip(true);
            } else if self.loaded
                && !self.started()
                && self.load_started.elapsed() > Duration::from_secs(25)
                && self.stalled != self.load_generation
            {
                self.stalled = self.load_generation;
                if !self.compatible && !self.engine_played {
                    return self.fall_back_to_compatible("rien ne démarre");
                }
                self.loaded = false;
                self.ui.error("Spotify ne démarre pas la lecture. Réessayez dans un instant.");
                self.send_playback(false);
            }
            // Asleep after a pause: its WebView2 processes give their memory back.
            if self.settings.engine_sleep && !self.playing && self.last_active.elapsed() >= ENGINE_SLEEP {
                log::info!("engine idle: stopped");
                self.loaded = false;
                self.stop_engine();
                self.ui.send(Event::EngineStatus("En veille : il redémarre à la prochaine lecture".into()));
                self.send_playback(false);
            }
        }
        self.send_usage();
    }

    fn send_usage(&mut self) {
        let current = if self.playing { self.position_at.elapsed().as_millis() as u64 } else { 0 };
        // What WebView2 reports, or an estimate if its audio requests are not seen.
        let estimate = (self.listened_ms + current) * ESTIMATED_KBPS / 8;
        let audio = self.engine_bytes.max(estimate);
        let api = self.api_bytes.load(Ordering::Relaxed);
        if (api, audio) != self.last_usage {
            self.last_usage = (api, audio);
            self.ui.send(Event::DataUsage { api_bytes: api, audio_bytes: audio });
        }
    }

    fn apply_settings(&mut self, new: Settings) {
        let covers_off = self.settings.show_covers && !new.show_covers;
        self.settings = new;
        if covers_off {
            self.ui.send(Event::Info("Pochettes désactivées : plus aucune image téléchargée.".into()));
        }
    }

    fn clear_cache(&mut self) {
        let before = self.store.size();
        self.store.clear();
        self.refreshed.clear();
        self.ui.send(Event::Info(format!("Cache vidé ({}).", human_bytes(before))));
        self.measure_cache();
    }

    fn measure_cache(&self) {
        let (store, ui) = (self.store.clone(), self.ui.clone());
        tokio::spawn(async move {
            if let Ok(size) = tokio::task::spawn_blocking(move || store.size()).await {
                ui.send(Event::CacheSize(size));
            }
        });
    }

    // ----------------------------------------------------------------------
    // Playback

    /// A track of SpotiLite's own queue (one track is sent at a time).
    fn load(&mut self, track: Track, play: bool, position_ms: u32) {
        self.begin_load();
        self.position_ms = position_ms;
        self.ui.send(Event::NowPlaying(Some(track.clone())));
        self.send_queue();
        self.refresh_liked_state(&track);
        if !play || !self.ensure_engine() {
            if let Some(engine) = &self.engine {
                engine.send(EngineCommand::Pause);
            }
            return self.send_playback(false);
        }
        self.tracker.expect(track.id.clone(), track.duration_ms);
        self.loaded = true;
        self.send_playback(true);
        self.request_play(PlayRequest::Track { id: track.id, position_ms });
    }

    /// A whole playlist or artist, played by Spotify itself.
    fn play_remote(&mut self, uri: String, title: String, offset: Option<Offset>, position_ms: u32) {
        self.queue.clear();
        self.begin_load();
        self.position_ms = position_ms;
        self.remote =
            Some(Remote { uri: uri.clone(), title, current: None, upcoming: Vec::new(), started: false });
        self.ui.send(Event::NowPlaying(None));
        self.send_queue();
        if !self.ensure_engine() {
            return self.send_playback(false);
        }
        self.loaded = true;
        self.send_playback(true);
        self.request_play(PlayRequest::Context { uri, offset, position_ms });
    }

    fn begin_load(&mut self) {
        self.load_generation += 1;
        self.load_started = Instant::now();
        self.flush_listened();
        self.position_at = Instant::now();
        self.playing = false;
        self.loaded = false;
        self.tracker.clear();
        self.pending = None;
        self.last_active = Instant::now();
    }

    /// Asks Spotify to start playback on the engine (one Web API call).
    fn request_play(&mut self, request: PlayRequest) {
        let generation = self.load_generation;
        let Some(device) = self.device_id.clone() else {
            // Sent when the engine announces itself.
            self.pending = Some((generation, request));
            return;
        };
        // Single tracks must not loop with the account's "repeat"; whole contexts
        // follow SpotiLite's shuffle and repeat buttons.
        let (shuffle, repeat) = (self.queue.shuffle(), self.queue.repeat());
        let prepare = !self.device_prepared;
        self.device_prepared = true;
        let (api, internal) = (self.api.clone(), self.internal.clone());
        tokio::spawn(async move {
            let context = matches!(request, PlayRequest::Context { .. });
            let mut attempt = 0;
            let result = loop {
                match api.play_on_device(&device, &request).await {
                    // Spotify's servers learn about a new device a moment after it is ready.
                    Err(ApiError::NotFound) if attempt < 3 => {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(700 * attempt)).await;
                    }
                    other => break other,
                }
            };
            match result {
                Ok(()) if context => {
                    // Applied once the device is active (a random start is already chosen).
                    let _ = api.set_shuffle(&device, shuffle).await;
                    let _ = api.set_repeat(&device, repeat_state(repeat)).await;
                }
                Ok(()) if prepare => {
                    let _ = api.set_repeat(&device, "off").await;
                }
                Ok(()) => {}
                Err(e) => {
                    let _ = internal.send(Internal::PlayFailed(generation, e));
                }
            }
        });
    }

    /// A player call on the engine's device (shuffle, repeat, queue).
    fn device_call<F, Fut>(&self, call: F)
    where
        F: FnOnce(Arc<WebApi>, String) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ApiError>> + Send,
    {
        let Some(device) = self.device_id.clone() else { return };
        let (api, ui) = (self.api.clone(), self.ui.clone());
        tokio::spawn(async move {
            if let Err(e) = call(api, device).await {
                ui.error(format!("Spotify : {e}"));
            }
        });
    }

    fn started(&self) -> bool {
        match &self.remote {
            Some(remote) => remote.started,
            None => self.tracker.started(),
        }
    }

    fn play_pause(&mut self) {
        if self.playing {
            return self.pause();
        }
        if self.loaded && self.started() && self.device_id.is_some() {
            if let Some(engine) = &self.engine {
                engine.send(EngineCommand::Resume);
            }
            self.check_followed(true);
            return;
        }
        // Nothing loaded (engine asleep, other device…): start again where we were.
        let position = self.position_ms;
        if let Some(remote) = &self.remote {
            let (uri, title) = (remote.uri.clone(), remote.title.clone());
            let offset = remote.current.as_ref().map(|t| Offset::Track(t.id.clone()));
            let position = if offset.is_some() { position } else { 0 };
            self.play_remote(uri, title, offset, position);
        } else if let Some(track) = self.queue.current().cloned() {
            self.load(track, true, position);
        }
    }

    fn pause(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(EngineCommand::Pause);
            self.check_followed(false);
        }
        let position = self.current_position();
        self.flush_listened();
        self.playing = false;
        self.position_ms = position;
        self.position_at = Instant::now();
        self.tracker.paused(Instant::now());
        self.send_playback(false);
    }

    /// In a moment, checks that the engine paused (or played) as asked.
    fn check_followed(&mut self, playing: bool) {
        self.control += 1;
        let (control, internal) = (self.control, self.internal.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(2500)).await;
            let _ = internal.send(Internal::CheckFollowed(control, playing));
        });
    }

    /// The engine did not follow a pause or play command (its own state can get
    /// out of step after a pause made elsewhere): Spotify's servers are asked to
    /// do it, as when the Spotify app controls SpotiLite.
    fn ensure_followed(&mut self, control: u64, playing: bool) {
        if control != self.control || self.engine_paused != Some(playing) {
            return;
        }
        let (Some(api), Some(device)) = (self.api(), self.device_id.clone()) else { return };
        log::warn!("engine did not {}: asking Spotify", if playing { "resume" } else { "pause" });
        tokio::spawn(async move {
            if let Err(e) = api.set_playing(&device, playing).await {
                log::warn!("remote {}: {e}", if playing { "resume" } else { "pause" });
            }
        });
    }

    fn skip(&mut self, auto: bool) {
        if self.remote.is_some() {
            if let Some(engine) = &self.engine {
                engine.send(EngineCommand::Next);
            }
            return;
        }
        match self.queue.advance(auto) {
            Some(track) => self.load(track, true, 0),
            None => {
                if let Some(engine) = &self.engine {
                    engine.send(EngineCommand::Pause);
                }
                self.tracker.clear();
                self.loaded = false;
                self.flush_listened();
                self.playing = false;
                self.set_position(0);
                self.send_queue();
            }
        }
    }

    fn previous(&mut self) {
        if self.current_position() > 3000 {
            return self.seek_to(0);
        }
        if self.remote.is_some() {
            if let Some(engine) = &self.engine {
                engine.send(EngineCommand::Previous);
            }
            return;
        }
        match self.queue.back() {
            Some(track) => self.load(track, true, 0),
            None => self.seek_to(0),
        }
    }

    fn seek_to(&mut self, ms: u32) {
        if self.loaded
            && let Some(engine) = &self.engine
        {
            engine.send(EngineCommand::Seek(ms));
            self.tracker.seeked(ms, Instant::now());
        }
        self.set_position(ms);
    }

    fn current_position(&self) -> u32 {
        if self.playing {
            self.position_ms + self.position_at.elapsed().as_millis() as u32
        } else {
            self.position_ms
        }
    }

    fn set_position(&mut self, ms: u32) {
        self.flush_listened();
        self.position_ms = ms;
        self.position_at = Instant::now();
        self.send_playback(false);
    }

    fn flush_listened(&mut self) {
        if self.playing {
            self.listened_ms += self.position_at.elapsed().as_millis() as u64;
            self.position_at = Instant::now();
        }
    }

    fn send_playback(&self, buffering: bool) {
        self.ui.send(Event::Playback { playing: self.playing, buffering, position_ms: self.position_ms });
    }

    fn send_queue(&self) {
        let event = match &self.remote {
            Some(remote) => Event::Queue {
                upcoming: remote.upcoming.clone(),
                shuffle: self.queue.shuffle(),
                repeat: self.queue.repeat(),
                context: Some(remote.title.clone()),
            },
            None => Event::Queue {
                upcoming: self.queue.upcoming(100),
                shuffle: self.queue.shuffle(),
                repeat: self.queue.repeat(),
                context: None,
            },
        };
        self.ui.send(event);
    }

    fn schedule_skip(&self, delay: Duration) {
        let (internal, generation) = (self.internal.clone(), self.load_generation);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = internal.send(Internal::SkipAfterFailure(generation));
        });
    }

    // ----------------------------------------------------------------------
    // Playback engine (Spotify's Web Playback SDK in WebView2)

    fn ensure_engine(&mut self) -> bool {
        if self.engine.is_some() {
            return true;
        }
        if !self.app_connected {
            self.ui
                .error("Connectez votre application Spotify pour écouter (Réglages → Application Spotify).");
            return false;
        }
        self.engine_id += 1;
        let (id, internal) = (self.engine_id, self.internal.clone());
        let profile = if self.compatible { Profile::Compatible } else { Profile::Lean };
        log::info!("starting the engine ({profile:?})");
        self.device_id = None;
        self.device_prepared = false;
        self.engine_played = false;
        self.drm.clear();
        self.last_active = Instant::now();
        self.ui.send(Event::EngineStatus("Démarrage…".into()));
        self.engine =
            Some(engine::Engine::start(self.paths.webview(), self.settings.volume, profile, move |event| {
                let _ = internal.send(Internal::Engine(id, event));
            }));
        true
    }

    fn stop_engine(&mut self) {
        if self.engine.take().is_some() {
            log::info!("engine stopped");
            self.ui.send(Event::EngineMemory(0));
            self.ui.send(Event::EngineStatus("Arrêté".into()));
        }
        self.device_id = None;
        self.pending = None;
        self.engine_paused = None;
        self.tracker.clear();
        if let Some(remote) = &mut self.remote {
            remote.started = false;
        }
    }

    /// The lean profile does not play on this PC: WebView2 defaults from now on.
    fn fall_back_to_compatible(&mut self, reason: &str) {
        log::warn!("lean engine profile failed ({reason}): compatible profile from now on");
        self.compatible = true;
        let _ = std::fs::write(self.paths.cache.join(COMPATIBLE_MARKER), reason);
        self.stop_engine();
        self.loaded = false;
        self.playing = false;
        self.ui.send(Event::Notice {
            key: "engine",
            text: "Le lecteur redémarre en mode compatible (un peu plus de mémoire sur ce PC).".into(),
            error: false,
        });
        self.play_pause();
    }

    fn on_engine(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Ready(device) => {
                log::info!("engine ready");
                self.device_id = Some(device);
                self.send_engine_status();
                if let Some((generation, request)) = self.pending.take()
                    && generation == self.load_generation
                {
                    self.request_play(request);
                }
            }
            EngineEvent::NotReady => {
                self.device_id = None;
                self.ui.send(Event::EngineStatus("Reconnexion à Spotify…".into()));
            }
            EngineEvent::NeedToken => {
                let Some(sender) = self.engine.as_ref().map(engine::Engine::sender) else { return };
                let (api, internal) = (self.api.clone(), self.internal.clone());
                tokio::spawn(async move {
                    match api.access_token().await {
                        Ok(token) => sender.send(EngineCommand::Token(token)),
                        Err(e) => {
                            let _ = internal.send(Internal::TokenFailed(e.to_string()));
                        }
                    }
                });
            }
            EngineEvent::State(Some(state)) => {
                if self.engine_paused != Some(state.paused) {
                    log::info!(
                        "engine {} at {} ms",
                        if state.paused { "paused" } else { "playing" },
                        state.position_ms
                    );
                    self.engine_paused = Some(state.paused);
                }
                if self.remote.is_some() {
                    self.on_remote_state(state);
                } else {
                    match self.tracker.on_state(&state, Instant::now()) {
                        Progress::Ignore => {}
                        Progress::Started => {
                            self.failure_streak = 0;
                            self.engine_played = true;
                            self.progress(&state);
                        }
                        Progress::Update => self.progress(&state),
                        Progress::Ended => {
                            self.flush_listened();
                            self.playing = false;
                            self.skip(true);
                        }
                    }
                }
            }
            EngineEvent::State(None) => {
                self.engine_paused = None;
                // Playback moved to another device (phone, other computer…).
                if self.loaded && self.started() {
                    let position = self.current_position();
                    self.flush_listened();
                    self.playing = false;
                    self.loaded = false;
                    self.position_ms = position;
                    self.position_at = Instant::now();
                    self.tracker.clear();
                    if let Some(remote) = &mut self.remote {
                        remote.started = false;
                    }
                    self.send_playback(false);
                    self.ui.send(Event::Info("La lecture continue sur un autre appareil Spotify.".into()));
                }
            }
            EngineEvent::Error(kind, message) => self.on_engine_error(kind, message),
            EngineEvent::Drm(systems) => {
                log::info!("DRM systems: {systems:?}");
                self.drm = if systems.iter().any(|s| s.contains("widevine")) {
                    "Widevine".into()
                } else if systems.iter().any(|s| s.contains("playready")) {
                    "PlayReady".into()
                } else {
                    String::new()
                };
                if systems.is_empty() {
                    self.ui.error(
                        "WebView2 ne propose aucun DRM (Widevine, PlayReady) sur ce PC : Spotify ne pourra rien lire. \
                         Mettez Windows et Microsoft Edge WebView2 à jour.",
                    );
                }
                self.send_engine_status();
            }
            EngineEvent::Log(message) => log::info!("engine: {message}"),
            EngineEvent::Media(action) => {
                log::info!("media action in the engine page: {action}");
                match action.as_str() {
                    "play" if !self.playing => self.play_pause(),
                    "pause" | "stop" if self.playing || self.engine_paused == Some(false) => self.pause(),
                    "nexttrack" => self.skip(false),
                    "previoustrack" => self.previous(),
                    _ => {}
                }
            }
            EngineEvent::Bytes(bytes) => self.engine_bytes += bytes,
            EngineEvent::Memory(bytes) => self.ui.send(Event::EngineMemory(bytes)),
            EngineEvent::Failed(message) => {
                self.stop_engine();
                self.playing = false;
                self.loaded = false;
                self.send_playback(false);
                self.ui.error(format!("Lecteur : {message}"));
                self.ui.send(Event::EngineStatus(format!("Arrêté : {message}")));
            }
        }
    }

    fn on_remote_state(&mut self, state: PlayerState) {
        let Some(remote) = &mut self.remote else { return };
        let mut changed = None;
        if let Some(track) = &state.current
            && remote.current.as_ref().is_none_or(|t| t.id != track.id)
        {
            remote.current = Some(track.clone());
            changed = Some(track.clone());
        }
        if !state.paused && !state.loading && !remote.started {
            remote.started = true;
            self.failure_streak = 0;
            self.engine_played = true;
        }
        let upcoming_changed = remote.upcoming != state.next;
        remote.upcoming = state.next.clone();
        if let Some(track) = changed {
            self.ui.send(Event::NowPlaying(Some(track.clone())));
            self.refresh_liked_state(&track);
        }
        if upcoming_changed {
            self.send_queue();
        }
        self.progress(&state);
    }

    fn send_engine_status(&self) {
        let mut text =
            if self.device_id.is_some() { "Prêt".to_string() } else { "Démarrage…".to_string() };
        if !self.drm.is_empty() {
            text.push_str(&format!(" · DRM {}", self.drm));
        }
        if self.compatible {
            text.push_str(" · mode compatible");
        }
        self.ui.send(Event::EngineStatus(text));
    }

    fn progress(&mut self, state: &PlayerState) {
        self.flush_listened();
        self.playing = !state.paused && !state.loading;
        self.position_ms = state.position_ms;
        self.position_at = Instant::now();
        if self.playing {
            self.last_active = Instant::now();
        }
        self.send_playback(state.loading);
    }

    fn on_engine_error(&mut self, kind: ErrorKind, message: String) {
        log::warn!("engine error {kind:?}: {message}");
        // The lean profile is the first suspect while nothing has played yet.
        let lean_suspect = !self.compatible && !self.engine_played;
        let fatal = match kind {
            ErrorKind::Playback if lean_suspect => return self.fall_back_to_compatible(&message),
            ErrorKind::Playback => return self.track_failed(message),
            ErrorKind::Initialization if lean_suspect => return self.fall_back_to_compatible(&message),
            ErrorKind::Authentication => {
                self.stop_engine();
                self.ask_playback_auth(true);
                true
            }
            ErrorKind::Connect => {
                // Usually a token without the playback scopes: checked (and asked for)
                // right away; otherwise the application or the account is the cause.
                self.check_playback_scopes(true);
                self.ui.error(
                    "Spotify refuse de connecter le lecteur. Vérifiez que « Web Playback SDK » est coché \
                     dans votre application (tableau de bord Spotify → Settings → Edit) et que le compte est Premium.",
                );
                true
            }
            ErrorKind::Account => {
                self.ui.error("Spotify refuse ce compte au lecteur : un abonnement Premium est nécessaire.");
                true
            }
            ErrorKind::Initialization => {
                self.ui.error(format!(
                    "Le lecteur de Spotify ne peut pas démarrer dans WebView2 ({message}). \
                     Mettez Windows et Microsoft Edge WebView2 à jour."
                ));
                true
            }
            ErrorKind::Load => {
                self.ui
                    .error("Lecteur de Spotify injoignable (sdk.scdn.co) : vérifiez la connexion Internet.");
                true
            }
            ErrorKind::Autoplay => {
                self.ui.error("WebView2 a bloqué le démarrage du son : relancez la lecture.");
                false
            }
            ErrorKind::Other => {
                self.ui.error(format!("Lecteur : {message}"));
                false
            }
        };
        if fatal {
            self.stop_engine();
            self.ui.send(Event::EngineStatus("Arrêté (erreur)".into()));
        }
        self.playing = false;
        self.loaded = false;
        self.send_playback(false);
    }

    /// One track could not be played.
    fn track_failed(&mut self, message: String) {
        self.playing = false;
        self.loaded = false;
        self.tracker.clear();
        self.failure_streak += 1;
        self.send_playback(false);
        if self.failure_streak >= 3 || self.remote.is_some() {
            self.failure_streak = 0;
            return self.ui.error(format!("Lecture impossible ({message}) : lecture arrêtée."));
        }
        let name = self.queue.current().map(|t| t.name.clone()).unwrap_or_default();
        self.ui.error(format!("« {name} » : lecture impossible ({message}). Titre suivant…"));
        self.schedule_skip(Duration::from_millis(1500));
    }

    fn on_play_failed(&mut self, error: ApiError) {
        log::warn!("play request failed: {error}");
        self.playing = false;
        self.loaded = false;
        self.tracker.clear();
        self.send_playback(false);
        if error.is_missing_scope() {
            return self.ask_playback_auth(true);
        }
        match error {
            ApiError::NotFound => {
                // The device is unknown to Spotify: start a fresh engine next time.
                self.stop_engine();
                self.ui.error("Spotify ne trouve pas le lecteur : relancez la lecture.");
            }
            ApiError::NotConnected => {
                self.ui.error("Connectez votre application Spotify (Réglages → Application Spotify).")
            }
            other => self.ui.error(format!("Lecture impossible : {other}")),
        }
    }

    // ----------------------------------------------------------------------
    // Library

    /// The Web API client, once the user's application is authorized.
    fn api(&self) -> Option<Arc<WebApi>> {
        self.app_connected.then(|| self.api.clone())
    }

    fn remember_playlists(&mut self, list: &[PlaylistSummary]) {
        self.playlists = list.iter().map(|p| (p.id.clone(), p.clone())).collect();
    }

    fn load_playlists(&mut self) {
        let Some(api) = self.api() else { return };
        let (ui, store, internal) = (self.ui.clone(), self.store.clone(), self.internal.clone());
        tokio::spawn(async move {
            match api.playlists().await {
                Ok(list) => {
                    store.save("playlists", &list);
                    let _ = internal.send(Internal::Playlists(list));
                }
                Err(e) => ui.error(format!("Playlists : {e}")),
            }
        });
    }

    /// Returns true if `view` was refreshed from the network recently enough.
    fn fresh(&mut self, view: &ViewKey, max_age: Duration, force: bool) -> bool {
        let fresh = !force && self.refreshed.get(view).is_some_and(|t| t.elapsed() < max_age);
        if !fresh {
            self.refreshed.insert(view.clone(), Instant::now());
        }
        fresh
    }

    fn open(&mut self, view: ViewKey, force: bool) {
        let ui = self.ui.clone();
        let store = self.store.clone();
        let Some(api) = self.api() else {
            return self.open_offline(view);
        };
        match view.clone() {
            ViewKey::Liked => {
                let cached = store.load::<CachedTracks>("liked");
                if let Some(c) = &cached {
                    ui.send(Event::Tracks {
                        view: view.clone(),
                        title: "Titres likés".into(),
                        subtitle: String::new(),
                        cover: None,
                        tracks: Arc::new(c.tracks.clone()),
                    });
                    if self.fresh(&view, Duration::from_secs(600), force) {
                        return;
                    }
                } else {
                    self.fresh(&view, Duration::ZERO, true);
                    ui.send(Event::Loading(view.clone()));
                }
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    let previous = cached.as_ref().map(|c| c.tracks.as_slice());
                    match api.liked_tracks(if force { None } else { previous }).await {
                        Ok(tracks) => {
                            let unchanged = previous.is_some_and(|p| p == tracks.as_slice());
                            let _ = internal
                                .send(Internal::LikedIds(tracks.iter().map(|t| t.id.clone()).collect()));
                            if !unchanged {
                                ui.send(Event::Tracks {
                                    view: view.clone(),
                                    title: "Titres likés".into(),
                                    subtitle: String::new(),
                                    cover: None,
                                    tracks: Arc::new(tracks.clone()),
                                });
                                store.save(
                                    "liked",
                                    &CachedTracks { snapshot: String::new(), saved_at: now_secs(), tracks },
                                );
                            }
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::SavedAlbums => {
                let cached = store.load::<Vec<AlbumSummary>>("albums");
                if let Some(albums) = &cached {
                    ui.send(Event::Albums {
                        view: view.clone(),
                        title: "Albums".into(),
                        albums: albums.clone(),
                    });
                    if self.fresh(&view, Duration::from_secs(1800), force) {
                        return;
                    }
                } else {
                    self.fresh(&view, Duration::ZERO, true);
                    ui.send(Event::Loading(view.clone()));
                }
                tokio::spawn(async move {
                    match api.saved_albums().await {
                        Ok(albums) => {
                            if cached.as_ref() != Some(&albums) {
                                store.save("albums", &albums);
                                ui.send(Event::Albums { view, title: "Albums".into(), albums });
                            }
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::Artists => {
                // The artists of the liked tracks are always shown: the page is
                // never empty, even when followed artists cannot be read.
                let liked = liked_cache(&store);
                let cached = store.load::<Vec<ArtistSummary>>("artists");
                if let Some(followed) = &cached {
                    ui.send(Event::Artists(artists_page(&liked, followed.clone(), None, false)));
                    if self.fresh(&view, Duration::from_secs(1800), force) {
                        return;
                    }
                } else {
                    self.fresh(&view, Duration::ZERO, true);
                    ui.send(Event::Loading(view.clone()));
                }
                tokio::spawn(async move {
                    let page = match api.followed_artists().await {
                        Ok(followed) => {
                            if cached.as_ref() == Some(&followed) {
                                return;
                            }
                            store.save("artists", &followed);
                            artists_page(&liked, followed, None, false)
                        }
                        // Authorizations given before this page existed lack the scope.
                        Err(e) if e.is_missing_scope() => artists_page(
                            &liked,
                            cached.unwrap_or_default(),
                            Some("Autorisez SpotiLite à lire les artistes que vous suivez.".into()),
                            true,
                        ),
                        Err(e) => {
                            log::warn!("followed artists: {e}");
                            artists_page(
                                &liked,
                                cached.unwrap_or_default(),
                                Some(format!("Artistes suivis indisponibles pour le moment ({e}).")),
                                false,
                            )
                        }
                    };
                    ui.send(Event::Artists(page));
                });
            }
            ViewKey::Playlist(id) => {
                let summary = self.playlists.get(&id).cloned();
                let title = summary.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Playlist".into());
                let subtitle = summary.as_ref().map(|p| p.owner.clone()).unwrap_or_default();
                let total = summary.as_ref().map_or(0, |p| p.total);
                let cover = summary.as_ref().and_then(|p| p.image.clone());
                let key = format!("playlist-{id}");
                let cached = store.load::<CachedTracks>(&key);
                let cache_valid = cached.as_ref().is_some_and(|c| match &summary {
                    Some(s) if !s.snapshot_id.is_empty() => c.snapshot == s.snapshot_id,
                    _ => now_secs().saturating_sub(c.saved_at) < 3600,
                });
                if let Some(c) = &cached {
                    ui.send(Event::Tracks {
                        view: view.clone(),
                        title: title.clone(),
                        subtitle: subtitle.clone(),
                        cover: cover.clone(),
                        tracks: Arc::new(c.tracks.clone()),
                    });
                    if cache_valid && !force {
                        return;
                    }
                } else {
                    ui.send(Event::Loading(view.clone()));
                }
                let snapshot = summary.map(|s| s.snapshot_id).unwrap_or_default();
                tokio::spawn(async move {
                    match api.playlist_tracks(&id).await {
                        Ok(tracks) => {
                            store.save(
                                &key,
                                &CachedTracks { snapshot, saved_at: now_secs(), tracks: tracks.clone() },
                            );
                            ui.send(Event::Tracks { view, title, subtitle, cover, tracks: Arc::new(tracks) });
                        }
                        // Development mode applications only read the user's own
                        // playlists: Spotify can still play the others as a whole.
                        Err(e @ (ApiError::Forbidden(_) | ApiError::NotFound)) => {
                            log::info!("playlist {id} not readable ({e}): played as a whole");
                            let uri = format!("spotify:playlist:{id}");
                            ui.send(Event::PlaylistContext { view, title, subtitle, cover, uri, total });
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::Album(id) => {
                let key = format!("album-{id}");
                // Albums cached before banners existed lack the larger cover: read them again.
                if !force
                    && let Some(c) = store.load::<CachedAlbum>(&key)
                    && (c.album.cover.is_some() || c.album.image.is_none())
                {
                    return ui.send(album_event(view, c));
                }
                ui.send(Event::Loading(view.clone()));
                tokio::spawn(async move {
                    match api.album(&id).await {
                        Ok((album, tracks)) => {
                            let cached = CachedAlbum { album, tracks };
                            store.save(&key, &cached);
                            ui.send(album_event(view, cached));
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::Artist(id) => {
                let key = format!("artist-{id}");
                let liked = liked_by(&store, &id);
                if !force
                    && let Some(c) = store.load::<CachedArtist>(&key)
                    && now_secs().saturating_sub(c.saved_at) < 86_400
                {
                    return ui.send(Event::Artist {
                        id,
                        name: c.name,
                        image: c.image,
                        liked,
                        albums: c.albums,
                    });
                }
                ui.send(Event::Loading(view.clone()));
                tokio::spawn(async move {
                    match api.artist(&id).await {
                        Ok(artist) => {
                            store.save(
                                &key,
                                &CachedArtist {
                                    saved_at: now_secs(),
                                    name: artist.name.clone(),
                                    image: artist.image.clone(),
                                    albums: artist.albums.clone(),
                                },
                            );
                            let (name, image, albums) = (artist.name, artist.image, artist.albums);
                            ui.send(Event::Artist { id, name, image, liked, albums });
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::Search(query) => {
                let query = query.trim().to_string();
                if query.is_empty() {
                    return;
                }
                if !force && let Some(hit) = self.searches.iter().find(|s| s.query == query) {
                    return ui.send(Event::Search(hit.clone()));
                }
                ui.send(Event::Loading(view.clone()));
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    match api.search(&query).await {
                        Ok(results) => {
                            ui.send(Event::Search(results.clone()));
                            let _ = internal.send(Internal::SearchDone(results));
                        }
                        Err(e) => ui.send(Event::ViewFailed { view, message: e.to_string() }),
                    }
                });
            }
            ViewKey::Queue | ViewKey::Settings | ViewKey::Home => {}
        }
    }

    fn open_offline(&self, view: ViewKey) {
        let store = &self.store;
        let event =
            match &view {
                ViewKey::Liked => store.load::<CachedTracks>("liked").map(|c| Event::Tracks {
                    view: view.clone(),
                    title: "Titres likés".into(),
                    subtitle: String::new(),
                    cover: None,
                    tracks: Arc::new(c.tracks),
                }),
                ViewKey::SavedAlbums => store
                    .load::<Vec<AlbumSummary>>("albums")
                    .map(|albums| Event::Albums { view: view.clone(), title: "Albums".into(), albums }),
                ViewKey::Artists => Some(Event::Artists(artists_page(
                    &liked_cache(store),
                    store.load::<Vec<ArtistSummary>>("artists").unwrap_or_default(),
                    None,
                    false,
                ))),
                ViewKey::Artist(id) => {
                    store.load::<CachedArtist>(&format!("artist-{id}")).map(|c| Event::Artist {
                        liked: liked_by(store, id),
                        id: id.clone(),
                        name: c.name,
                        image: c.image,
                        albums: c.albums,
                    })
                }
                ViewKey::Playlist(id) => store.load::<CachedTracks>(&format!("playlist-{id}")).map(|c| {
                    let summary = self.playlists.get(id);
                    Event::Tracks {
                        view: view.clone(),
                        title: summary.map(|p| p.name.clone()).unwrap_or_default(),
                        subtitle: summary.map(|p| p.owner.clone()).unwrap_or_default(),
                        cover: summary.and_then(|p| p.image.clone()),
                        tracks: Arc::new(c.tracks),
                    }
                }),
                ViewKey::Album(id) => {
                    store.load::<CachedAlbum>(&format!("album-{id}")).map(|c| album_event(view.clone(), c))
                }
                _ => None,
            };
        match event {
            Some(event) => self.ui.send(event),
            None => self.ui.send(Event::ViewFailed {
                view,
                message: "Connectez votre application Spotify pour charger ce contenu.".into(),
            }),
        }
    }

    fn refresh_liked_state(&self, track: &Track) {
        if let Some(ids) = &self.liked_ids {
            return self.ui.send(Event::Liked { track_id: track.id.clone(), liked: ids.contains(&track.id) });
        }
        let Some(api) = self.api() else { return };
        let (ui, id) = (self.ui.clone(), track.id.clone());
        tokio::spawn(async move {
            if let Ok(liked) = api.is_liked(&id).await {
                ui.send(Event::Liked { track_id: id, liked });
            }
        });
    }

    fn set_liked(&mut self, track: Track, liked: bool) {
        let Some(api) = self.api() else { return };
        if let Some(ids) = &mut self.liked_ids {
            if liked {
                ids.insert(track.id.clone());
            } else {
                ids.remove(&track.id);
            }
        }
        let (ui, store) = (self.ui.clone(), self.store.clone());
        self.ui.send(Event::Liked { track_id: track.id.clone(), liked });
        tokio::spawn(async move {
            match api.set_liked(&track.id, liked).await {
                Ok(()) => {
                    if let Some(mut cached) = store.load::<CachedTracks>("liked") {
                        cached.tracks.retain(|t| t.id != track.id);
                        if liked {
                            cached.tracks.insert(0, track);
                        }
                        store.save("liked", &cached);
                    }
                }
                Err(e) => {
                    ui.send(Event::Liked { track_id: track.id.clone(), liked: !liked });
                    ui.error(format!("« J'aime » non enregistré : {e}"));
                }
            }
        });
    }

    fn fetch_image(&self, url: String) {
        let store = self.store.clone();
        let (ui, api, allowed) = (self.ui.clone(), self.api(), self.settings.show_covers);
        tokio::spawn(async move {
            let bytes = match store.load_image(&url) {
                Some(bytes) => Some(bytes),
                None if allowed => match api {
                    Some(api) => match api.download(&url).await {
                        Ok(bytes) => {
                            store.save_image(&url, &bytes);
                            Some(bytes)
                        }
                        Err(e) => {
                            log::info!("cover download failed: {e}");
                            None
                        }
                    },
                    None => None,
                },
                None => None,
            };
            let image = match bytes {
                Some(bytes) => tokio::task::spawn_blocking(move || decode_image(&bytes)).await.ok().flatten(),
                None => None,
            };
            ui.send(Event::Image { url, image });
        });
    }
}

/// Spotify's name for a repeat mode.
fn repeat_state(mode: Repeat) -> &'static str {
    match mode {
        Repeat::Off => "off",
        Repeat::All => "context",
        Repeat::One => "track",
    }
}

/// Turns token endpoint errors into advice for the setup screen.
fn app_error_hint(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("invalid_client") || lower.contains("invalid client") {
        "Client ID ou Client Secret incorrect : recopiez-les depuis les réglages de votre application Spotify.".into()
    } else if lower.contains("redirect") {
        "URI de redirection refusée : ajoutez exactement celle affichée par SpotiLite dans votre application Spotify.".into()
    } else if lower.contains("délai") {
        "Aucune réponse du navigateur : vérifiez l'URI de redirection de votre application puis réessayez."
            .into()
    } else {
        format!("Connexion de l'application impossible : {error}")
    }
}

async fn run_flow(
    flow: AuthFlow,
    cancel: Arc<AtomicBool>,
    http: reqwest::Client,
) -> Result<auth::OAuthToken, String> {
    let flow = Arc::new(flow);
    let waiting = flow.clone();
    let code = tokio::task::spawn_blocking(move || waiting.wait_for_code(&cancel))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    flow.exchange(&http, &code).await.map_err(|e| e.to_string())
}

fn album_event(view: ViewKey, cached: CachedAlbum) -> Event {
    let subtitle = if cached.album.year.is_empty() {
        cached.album.artists.clone()
    } else {
        format!("{} · {}", cached.album.artists, cached.album.year)
    };
    let cover = cached.album.cover.or(cached.album.image);
    Event::Tracks { view, title: cached.album.name, subtitle, cover, tracks: Arc::new(cached.tracks) }
}

fn decode_image(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    // Covers are displayed at 56 px at most: never keep more pixels than needed.
    let image = if image.width() > 128 { image.thumbnail(128, 128) } else { image };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// Size in French notation ("1,5 Mo").
pub fn human_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    let text = if bytes == 0 {
        "0 Ko".to_string()
    } else if b < 1024.0 {
        "< 1 Ko".to_string()
    } else if b < 1024.0 * 1024.0 {
        format!("{:.0} Ko", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} Mo", b / 1024.0 / 1024.0)
    } else {
        format!("{:.2} Go", b / 1024.0 / 1024.0 / 1024.0)
    };
    text.replace('.', ",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_modes_map_to_spotify() {
        assert_eq!(repeat_state(Repeat::Off), "off");
        assert_eq!(repeat_state(Repeat::All), "context");
        assert_eq!(repeat_state(Repeat::One), "track");
    }

    #[test]
    fn explains_authorization_errors() {
        assert!(app_error_hint("400 : invalid_client").contains("Client Secret"));
        assert!(app_error_hint("Illegal redirect_uri").contains("redirection"));
        assert!(app_error_hint("autre").contains("autre"));
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_bytes(0), "0 Ko");
        assert_eq!(human_bytes(512), "< 1 Ko");
        assert_eq!(human_bytes(2048), "2 Ko");
        assert_eq!(human_bytes(5 * 1024 * 1024 + 300 * 1024), "5,3 Mo");
    }

    #[test]
    fn decodes_and_downsizes_jpeg() {
        let img = image::RgbImage::from_pixel(300, 300, image::Rgb([200, 100, 50]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        let decoded = decode_image(&jpeg).unwrap();
        assert_eq!(decoded.size, [128, 128]);
    }
}
