//! Backend thread: Spotify session, audio player, Web API and caches.
//!
//! The UI sends [`Command`]s and receives [`Event`]s. Everything network or audio
//! related runs here, on a small Tokio runtime (2 worker threads), so the interface
//! never blocks.

mod auth;
mod official;
mod store;
mod vault;
mod webapi;

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use futures_util::StreamExt;
use librespot_core::authentication::{AuthenticationError, Credentials};
use librespot_core::cache::Cache;
use librespot_core::config::SessionConfig;
use librespot_core::session::Session;
use librespot_core::{SpotifyId, SpotifyUri};
use librespot_playback::audio_backend;
use librespot_playback::config::{AudioFormat, Bitrate, PlayerConfig};
use librespot_playback::mixer::softmixer::SoftMixer;
use librespot_playback::mixer::{Mixer, MixerConfig};
use librespot_playback::player::{Player, PlayerEvent, PlayerEventChannel};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::config::{Engine, Paths, Quality, Settings};
use crate::model::{AlbumSummary, ArtistRef, PlaylistSummary, Repeat, SearchResults, Track, ViewKey};
use crate::queue::Queue;
pub use auth::AppCredentials;
use auth::{AuthFlow, OAuthToken};
use official::{EngineCommand, EngineEvent, ErrorKind, PlayerState, Progress, Tracker};
use store::Store;
use webapi::{ApiError, WebApi};

/// Requests from the interface.
pub enum Command {
    Login,
    CancelLogin,
    Logout,
    /// Saves the user's Spotify application and authorizes it in the browser.
    SetupApp(AppCredentials),
    /// Authorizes the saved application again.
    ReconnectApp,
    CancelAppLogin,
    /// Disconnects the application and deletes its saved credentials.
    ForgetApp,
    LoadPlaylists,
    Open {
        view: ViewKey,
        force: bool,
    },
    Play {
        tracks: Arc<Vec<Track>>,
        index: usize,
    },
    PlayPause,
    Next,
    Previous,
    Seek(u32),
    SetVolume(f32),
    SetShuffle(bool),
    SetRepeat(Repeat),
    Enqueue(Track),
    ClearQueue,
    SetLiked {
        track: Track,
        liked: bool,
    },
    FetchImage(String),
    ApplySettings(Box<Settings>),
    ClearCache,
    /// Gives the tracks Spotify refused another chance.
    ClearRefused,
    Shutdown,
}

/// State of the user's own Spotify application, which serves the whole library.
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
    /// The authorization predates the official engine: it lacks the playback scopes.
    pub needs_playback_auth: bool,
}

/// Notifications for the interface.
pub enum Event {
    Info(String),
    Error(String),
    /// A message that replaces the previous one with the same key (counters).
    Notice {
        key: &'static str,
        text: String,
        error: bool,
    },
    /// Tracks Spotify refuses to third-party clients (shown greyed out).
    Refused(HashSet<String>),
    TrackRefused(String),
    NeedLogin,
    LoginPending {
        url: String,
    },
    Connecting,
    /// Saved credentials exist but Spotify is unreachable: the cached library stays usable.
    Offline,
    LoggedIn {
        user: String,
    },
    App(AppStatus),
    Playlists(Vec<PlaylistSummary>),
    Loading(ViewKey),
    Tracks {
        view: ViewKey,
        title: String,
        subtitle: String,
        tracks: Arc<Vec<Track>>,
    },
    Albums {
        view: ViewKey,
        title: String,
        albums: Vec<AlbumSummary>,
    },
    Artist {
        id: String,
        name: String,
        top: Arc<Vec<Track>>,
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
    /// What the official engine is doing, for the settings page.
    EngineStatus(String),
    /// Memory of the official engine's WebView2 processes (0 when stopped).
    EngineMemory(u64),
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
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(4)
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
    StreamingToken(Result<String, String>),
    Connected(Result<Session, (String, bool)>),
    AppToken(Result<(AppCredentials, OAuthToken), String>),
    LikedIds(HashSet<String>),
    Playlists(Vec<PlaylistSummary>),
    SearchDone(SearchResults),
    /// Delayed move to the next track after an unavailable one (carries the load
    /// generation so that any newer user action cancels it).
    SkipAfterUnavailable(u64),
    /// Event of the official engine instance `.0`.
    Official(u64, EngineEvent),
    /// Starting a track on the official engine failed (load generation, error).
    OfficialPlayFailed(u64, ApiError),
    /// Playback scopes missing from the authorization (`.1`: the user just chose
    /// the official engine, so the browser may be opened).
    OfficialScopes(Vec<&'static str>, bool),
    OfficialTokenFailed(String),
}

struct Audio {
    player: Arc<Player>,
    mixer: SoftMixer,
    events: PlayerEventChannel,
    quality: Quality,
    normalisation: bool,
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

#[derive(Serialize, Deserialize)]
struct CachedArtist {
    saved_at: u64,
    name: String,
    top: Vec<Track>,
    albums: Vec<AlbumSummary>,
}

fn bitrate(quality: Quality) -> Bitrate {
    match quality {
        Quality::Eco => Bitrate::Bitrate96,
        Quality::Normal => Bitrate::Bitrate160,
        Quality::High => Bitrate::Bitrate320,
    }
}

/// Next quality to try when Spotify refuses the key of a track's file.
fn next_quality(quality: Quality) -> Option<Quality> {
    match quality {
        Quality::Eco => Some(Quality::Normal),
        Quality::Normal => Some(Quality::High),
        Quality::High => None,
    }
}

/// Consecutive refused tracks before concluding the whole account is refused.
const MAX_REFUSED_IN_A_ROW: u32 = 10;
/// Bitrate assumed for the official engine when its traffic cannot be measured.
const OFFICIAL_KBPS: u64 = 128;
const REFUSED_TTL_SECS: u64 = 14 * 24 * 3600;

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct Core {
    ui: UiTx,
    internal: UnboundedSender<Internal>,
    paths: Paths,
    settings: Settings,
    http: reqwest::Client,
    store: Store,
    cache: Option<Cache>,
    api_bytes: Arc<AtomicU64>,
    session: Option<Session>,
    api: Arc<WebApi>,
    /// The user's Spotify application is authorized: library calls can be made.
    app_connected: bool,
    app_login: Option<Arc<AtomicBool>>,
    audio: Option<Audio>,
    queue: Queue,
    playing: bool,
    position_ms: u32,
    position_at: Instant,
    /// Loads sent to the player whose play request id has not been announced yet:
    /// until then, events still refer to a previous track and are ignored.
    pending_loads: u32,
    /// Whether the player currently holds the current track (false once stopped).
    loaded: bool,
    unavailable_streak: u32,
    /// Incremented by every load: identifies the attempt a delayed skip belongs to.
    load_generation: u64,
    load_started: Instant,
    /// A fresh connection was already tried during the current failure streak.
    reconnect_tried: bool,
    /// Tracks whose audio key Spotify refused (id → when), kept 14 days.
    refused: HashMap<String, u64>,
    skipped_refused: u32,
    /// Quality the player currently uses: the chosen one, or a higher one when
    /// Spotify refused the key of the lower quality file.
    quality: Quality,
    /// Quality at which the current track was first attempted.
    attempt_quality: Option<Quality>,
    listened_ms: u64,
    audio_bytes: u64,
    last_usage: (u64, u64),
    login_cancel: Option<Arc<AtomicBool>>,
    connecting: bool,
    retry_at: Option<Instant>,
    pending_play: bool,
    playlists: HashMap<String, PlaylistSummary>,
    liked_ids: Option<HashSet<String>>,
    refreshed: HashMap<ViewKey, Instant>,
    searches: VecDeque<SearchResults>,
    audio_failed: bool,
    /// Spotify's own player in WebView2, started on demand ("moteur officiel").
    official: Option<official::Engine>,
    /// Instance number, so that events of a stopped engine are ignored.
    official_id: u64,
    /// Spotify device id of the official engine, once registered.
    device_id: Option<String>,
    /// Track waiting for the official engine to be ready (load generation, id, position).
    official_pending: Option<(u64, String, u32)>,
    tracker: Tracker,
    official_bytes: u64,
    official_listened_ms: u64,
    /// Load generation for which a stalled start was already reported.
    official_stalled: u64,
    /// The authorization lacks the scopes of the official engine.
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
            .pool_max_idle_per_host(2)
            .build()
            .expect("http client");
        let store = Store::new(paths.data_cache(), paths.image_cache());
        let api_bytes = Arc::new(AtomicU64::new(0));
        let api = Arc::new(WebApi::new(http.clone(), paths.clone(), api_bytes.clone()));
        let mut queue = Queue::default();
        queue.set_shuffle(settings.shuffle);
        queue.set_repeat(settings.repeat);
        Self {
            ui,
            internal,
            http,
            store,
            cache: None,
            api_bytes,
            session: None,
            api,
            app_connected: false,
            app_login: None,
            audio: None,
            queue,
            playing: false,
            position_ms: 0,
            position_at: Instant::now(),
            pending_loads: 0,
            loaded: false,
            unavailable_streak: 0,
            load_generation: 0,
            load_started: Instant::now(),
            reconnect_tried: false,
            refused: HashMap::new(),
            skipped_refused: 0,
            quality: settings.quality,
            attempt_quality: None,
            listened_ms: 0,
            audio_bytes: 0,
            last_usage: (u64::MAX, u64::MAX),
            login_cancel: None,
            connecting: false,
            retry_at: None,
            pending_play: false,
            playlists: HashMap::new(),
            liked_ids: None,
            refreshed: HashMap::new(),
            searches: VecDeque::new(),
            audio_failed: false,
            official: None,
            official_id: 0,
            device_id: None,
            official_pending: None,
            tracker: Tracker::default(),
            official_bytes: 0,
            official_listened_ms: 0,
            official_stalled: 0,
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
                Some(event) = next_player_event(&mut self.audio) => self.handle_player(event),
                _ = tick.tick() => self.tick(),
            }
        }
        self.shutdown();
    }

    async fn startup(&mut self) {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.prune());
        if let Some(list) = self.store.load::<Vec<PlaylistSummary>>("playlists") {
            self.remember_playlists(&list);
            self.ui.send(Event::Playlists(list));
        }
        self.send_queue();
        // Tracks refused recently are not retried (the decision may change, hence
        // the expiry).
        let now = now_secs();
        self.refused = self.store.load::<HashMap<String, u64>>("refused").unwrap_or_default();
        self.refused.retain(|_, at| now.saturating_sub(*at) < REFUSED_TTL_SECS);
        self.sync_blocked();
        // Version 0.1 kept the client id in the settings: move it to the vault.
        if self.api.saved_app().is_none() && !self.settings.legacy_client_id.is_empty() {
            self.api.save_app(&AppCredentials {
                client_id: self.settings.legacy_client_id.clone(),
                client_secret: String::new(),
            });
        }
        // The library only needs the user's application, not the audio session:
        // it can load while the audio connection is still being established.
        self.app_connected = self.api.restore().await;
        self.send_app_status();
        if self.app_connected {
            self.load_playlists();
            if self.official_mode() {
                self.check_playback_scopes(false);
            }
        }
        match self.librespot_cache().and_then(|c| c.credentials()) {
            Some(credentials) => self.connect(credentials, false),
            None => self.ui.send(Event::NeedLogin),
        }
    }

    fn shutdown(&mut self) {
        for cancel in [&self.login_cancel, &self.app_login].into_iter().flatten() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.official = None;
        if let Some(audio) = self.audio.take() {
            audio.player.stop();
            drop(audio);
        }
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
    }

    fn librespot_cache(&mut self) -> Option<Cache> {
        if self.cache.is_none() {
            let mb = self.settings.audio_cache_mb;
            let audio_dir = (mb > 0).then(|| self.paths.audio_cache());
            match Cache::new(
                Some(self.paths.config.clone()),
                None,
                audio_dir,
                (mb > 0).then_some(mb * 1024 * 1024),
            ) {
                Ok(cache) => self.cache = Some(cache),
                Err(e) => log::warn!("librespot cache unavailable: {e}"),
            }
        }
        self.cache.clone()
    }

    // ----------------------------------------------------------------------
    // Session

    fn connect(&mut self, credentials: Credentials, store_credentials: bool) {
        if self.connecting {
            return;
        }
        self.connecting = true;
        self.retry_at = None;
        self.ui.send(Event::Connecting);
        let _ = std::fs::create_dir_all(self.paths.tmp());
        let config = SessionConfig { tmp_dir: self.paths.tmp(), ..SessionConfig::default() };
        let session = Session::new(config, self.librespot_cache());
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let result = match tokio::time::timeout(
                Duration::from_secs(30),
                session.connect(credentials, store_credentials),
            )
            .await
            {
                Ok(Ok(())) => Ok(session),
                Ok(Err(e)) => {
                    let auth = e.error.downcast_ref::<AuthenticationError>().is_some();
                    Err((e.to_string(), auth))
                }
                Err(_) => Err(("délai de connexion dépassé".to_string(), false)),
            };
            let _ = internal.send(Internal::Connected(result));
        });
    }

    fn reconnect_if_needed(&mut self) -> bool {
        let invalid = self.session.as_ref().is_none_or(Session::is_invalid);
        if invalid
            && !self.connecting
            && let Some(credentials) = self.librespot_cache().and_then(|c| c.credentials())
        {
            log::info!("session lost, reconnecting");
            self.connect(credentials, false);
        }
        invalid
    }

    async fn on_connected(&mut self, session: Session) {
        self.connecting = false;
        let user = session.username();
        log::info!("connected as {user}");
        match &self.audio {
            Some(audio) if !audio.player.is_invalid() => audio.player.set_session(session.clone()),
            _ => self.audio = None,
        }
        self.session = Some(session);

        let display = self.store.load::<String>("me").unwrap_or(user);
        self.ui.send(Event::LoggedIn { user: display });
        self.fetch_display_name();
        if let Some(liked) = self.store.load::<CachedTracks>("liked") {
            self.liked_ids = Some(liked.tracks.into_iter().map(|t| t.id).collect());
        }
        if std::mem::take(&mut self.pending_play)
            && let Some(track) = self.queue.current().cloned()
        {
            let position = self.position_ms;
            self.load(track, true, position);
        }
    }

    fn start_login(&mut self) {
        if let Some(cancel) = self.login_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        let desktop = AppCredentials {
            client_id: auth::SPOTIFY_DESKTOP_CLIENT_ID.into(),
            client_secret: String::new(),
        };
        let flow = match AuthFlow::start(desktop, 0, auth::STREAMING_SCOPES) {
            Ok(flow) => flow,
            Err(e) => return self.ui.error(format!("Connexion impossible : {e}")),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.login_cancel = Some(cancel.clone());
        self.ui.send(Event::LoginPending { url: flow.auth_url.clone() });
        let _ = open::that_detached(&flow.auth_url);
        let (http, internal) = (self.http.clone(), self.internal.clone());
        tokio::spawn(async move {
            let result = run_flow(flow, cancel, http).await.map(|t| t.access_token);
            let _ = internal.send(Internal::StreamingToken(result));
        });
    }

    /// Opens the user's Spotify application authorization in the browser.
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
        if self.store.load::<String>("me").is_some() {
            return;
        }
        let (ui, store) = (self.ui.clone(), self.store.clone());
        tokio::spawn(async move {
            if let Ok(name) = api.me().await {
                store.save("me", &name);
                ui.send(Event::LoggedIn { user: name });
            }
        });
    }

    fn logout(&mut self) {
        if let Some(audio) = self.audio.take() {
            audio.player.stop();
            tokio::task::spawn_blocking(move || drop(audio));
        }
        self.stop_official();
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        let _ = std::fs::remove_file(self.paths.config.join("credentials.json"));
        // The application credentials stay saved: logging in again only needs one click.
        let api = self.api.clone();
        tokio::spawn(async move { api.disconnect(false).await });
        self.app_connected = false;
        self.send_app_status();
        self.store.clear();
        self.cache = None;
        self.queue = Queue::default();
        self.playlists.clear();
        self.liked_ids = None;
        self.refused.clear();
        self.queue.set_blocked(HashSet::new());
        self.ui.send(Event::Refused(HashSet::new()));
        self.refreshed.clear();
        self.searches.clear();
        self.playing = false;
        self.ui.send(Event::NowPlaying(None));
        self.ui.send(Event::Playlists(Vec::new()));
        self.ui.send(Event::NeedLogin);
    }

    // ----------------------------------------------------------------------
    // Commands

    async fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Login => self.start_login(),
            Command::CancelLogin => {
                if let Some(cancel) = self.login_cancel.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
            }
            Command::Logout => self.logout(),
            Command::SetupApp(app) => {
                let app = AppCredentials {
                    client_id: app.client_id.trim().to_string(),
                    client_secret: app.client_secret.trim().to_string(),
                };
                // A new application invalidates the previous authorization.
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
                self.api.disconnect(true).await;
                self.app_connected = false;
                self.send_app_status();
            }
            Command::LoadPlaylists => self.load_playlists(),
            Command::Open { view, force } => self.open(view, force),
            Command::Play { tracks, index } => {
                if let Some(track) = self.queue.play_context(tracks, index) {
                    self.load(track, true, 0);
                }
            }
            Command::PlayPause => self.play_pause(),
            Command::Next => self.skip(false),
            Command::Previous => self.previous(),
            Command::Seek(ms) => self.seek_to(ms),
            Command::SetVolume(volume) => {
                self.settings.volume = volume;
                if let Some(audio) = &self.audio {
                    audio.mixer.set_volume(volume_to_u16(volume));
                }
                if let Some(engine) = &self.official {
                    engine.send(EngineCommand::Volume(volume));
                }
            }
            Command::SetShuffle(on) => {
                self.queue.set_shuffle(on);
                self.send_queue();
            }
            Command::SetRepeat(mode) => {
                self.queue.set_repeat(mode);
                self.send_queue();
            }
            Command::Enqueue(track) => {
                let name = track.name.clone();
                self.queue.enqueue(track);
                self.send_queue();
                self.ui.send(Event::Info(format!("« {name} » ajouté à la file")));
            }
            Command::ClearQueue => {
                self.queue.clear_manual();
                self.send_queue();
            }
            Command::SetLiked { track, liked } => self.set_liked(track, liked),
            Command::FetchImage(url) => self.fetch_image(url),
            Command::ApplySettings(settings) => self.apply_settings(*settings).await,
            Command::ClearCache => self.clear_cache(),
            Command::ClearRefused => {
                self.clear_refused();
                self.ui.send(Event::Info("Les titres refusés seront retentés.".into()));
            }
            Command::Shutdown => {}
        }
    }

    async fn handle_internal(&mut self, msg: Internal) {
        match msg {
            Internal::StreamingToken(Ok(token)) => {
                self.login_cancel = None;
                self.connect(Credentials::with_access_token(token), true);
            }
            // A cancelled flow was replaced or abandoned on purpose: its handle is
            // already gone and must not clear the one of a newer flow.
            Internal::StreamingToken(Err(e)) if e.contains("annulée") => {}
            Internal::StreamingToken(Err(e)) => {
                self.login_cancel = None;
                self.ui.error(format!("Connexion impossible : {e}"));
                if self.session.is_none() {
                    self.ui.send(Event::NeedLogin);
                }
            }
            Internal::Connected(Ok(session)) => self.on_connected(session).await,
            Internal::Connected(Err((message, auth))) => {
                self.connecting = false;
                log::warn!("connection failed: {message}");
                if auth {
                    let _ = std::fs::remove_file(self.paths.config.join("credentials.json"));
                    self.cache = None;
                    self.ui.error("Session expirée : reconnectez-vous.");
                    self.ui.send(Event::NeedLogin);
                } else {
                    self.ui.error(format!(
                        "Connexion à Spotify impossible ({message}). Nouvel essai dans 15 s."
                    ));
                    self.retry_at = Some(Instant::now() + Duration::from_secs(15));
                    if self.session.is_none() {
                        self.ui.send(Event::Offline);
                    }
                }
            }
            Internal::AppToken(Err(e)) if e.contains("annulée") => {}
            Internal::AppToken(result) => {
                self.app_login = None;
                match result {
                    Ok((app, token)) => {
                        let missing = token.missing_scopes(auth::OFFICIAL_ENGINE_SCOPES);
                        let resume = self.needs_playback_auth && missing.is_empty() && self.official_mode();
                        self.needs_playback_auth = !missing.is_empty();
                        self.api.connect(app, token).await;
                        self.app_connected = true;
                        self.refreshed.clear();
                        self.ui.send(Event::Info("Application Spotify connectée.".into()));
                        self.load_playlists();
                        self.fetch_display_name();
                        if resume && let Some(track) = self.queue.current().cloned() {
                            // The play that needed the new authorization.
                            let position = self.position_ms;
                            self.load(track, true, position);
                        }
                    }
                    Err(e) => self.ui.error(app_error_hint(&e)),
                }
                self.send_app_status();
            }
            Internal::SkipAfterUnavailable(generation) => {
                if generation == self.load_generation {
                    self.skip(false);
                }
            }
            Internal::Official(id, event) => {
                if id == self.official_id && self.official.is_some() {
                    self.on_official(event);
                }
            }
            Internal::OfficialPlayFailed(generation, error) => {
                if generation == self.load_generation {
                    self.on_official_play_failed(error);
                }
            }
            Internal::OfficialScopes(missing, interactive) => {
                if missing.is_empty() {
                    if self.needs_playback_auth {
                        self.needs_playback_auth = false;
                        self.send_app_status();
                    }
                } else {
                    log::info!("authorization lacks {missing:?} for the official engine");
                    self.ask_playback_auth(interactive);
                }
            }
            Internal::OfficialTokenFailed(e) => {
                self.ui.error(format!("Moteur officiel : jeton d'accès indisponible ({e})."));
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
        // The connection to Spotify dropped (sleep, network change…): reconnect quietly.
        if self.session.as_ref().is_some_and(Session::is_invalid)
            && !self.connecting
            && self.retry_at.is_none()
        {
            self.reconnect_if_needed();
        }
        if self.retry_at.is_some_and(|t| Instant::now() >= t) {
            self.retry_at = None;
            if let Some(credentials) = self.librespot_cache().and_then(|c| c.credentials()) {
                self.connect(credentials, false);
            }
        }
        if self.api.take_lost() {
            self.app_connected = false;
            self.ui
                .error("L'autorisation de votre application Spotify a expiré : cliquez sur « Reconnecter ».");
            self.send_app_status();
        }
        if self.audio.as_ref().is_some_and(|a| a.player.is_invalid()) && !self.audio_failed {
            self.audio_failed = true;
            self.audio = None;
            self.playing = false;
            self.loaded = false;
            self.ui.error(
                "Sortie audio indisponible. Vérifiez votre périphérique audio puis relancez la lecture.",
            );
            self.send_playback(false);
        }
        if self.official_mode() && self.official.is_some() {
            if self.tracker.overdue(Instant::now()) {
                log::info!("official engine: end of track not reported");
                self.flush_listened();
                self.playing = false;
                self.skip(true);
            } else if self.loaded
                && !self.tracker.started()
                && self.load_started.elapsed() > Duration::from_secs(25)
                && self.official_stalled != self.load_generation
            {
                self.official_stalled = self.load_generation;
                self.loaded = false;
                self.ui.error(
                    "Le moteur officiel ne démarre pas la lecture. Réessayez ; si cela persiste, \
                     repassez au moteur SpotiLite (Réglages → Lecture).",
                );
                self.send_playback(false);
            }
        }
        self.send_usage();
    }

    fn send_usage(&mut self) {
        let current = if self.playing { self.position_at.elapsed().as_millis() as u64 } else { 0 };
        let (native_ms, official_ms) = if self.official_mode() {
            (self.listened_ms, self.official_listened_ms + current)
        } else {
            (self.listened_ms + current, self.official_listened_ms)
        };
        // What WebView2 reports, or an estimate if its audio requests are not seen.
        let official = self.official_bytes.max(official_ms * OFFICIAL_KBPS / 8);
        let audio = self.audio_bytes + native_ms * u64::from(self.quality.kbps()) / 8 + official;
        let api = self.api_bytes.load(Ordering::Relaxed);
        if (api, audio) != self.last_usage {
            self.last_usage = (api, audio);
            self.ui.send(Event::DataUsage { api_bytes: api, audio_bytes: audio });
        }
    }

    async fn apply_settings(&mut self, new: Settings) {
        if new.quality != self.settings.quality {
            // Refusals may depend on the file format: give every track a new chance.
            self.clear_refused();
        }
        let engine_changed = new.engine != self.settings.engine;
        let (position, playing) = (self.current_position(), self.playing);
        if engine_changed {
            // Counted for the engine that played it.
            self.flush_listened();
        }
        self.settings = new;
        self.quality = self.settings.quality;
        if engine_changed {
            return self.switch_engine(position, playing);
        }
        let audio_changed = self
            .audio
            .as_ref()
            .is_some_and(|a| a.quality != self.quality || a.normalisation != self.settings.normalisation);
        if audio_changed {
            // The bitrate is fixed when the player is created: rebuild it and resume.
            self.flush_listened();
            if let Some(audio) = self.audio.take() {
                audio.player.stop();
                tokio::task::spawn_blocking(move || drop(audio));
            }
            if let Some(track) = self.queue.current().cloned() {
                let position = self.current_position();
                let playing = self.playing;
                self.load(track, playing, position);
            }
        }
        if !self.settings.show_covers {
            self.ui.send(Event::Info("Pochettes désactivées : plus aucune image téléchargée.".into()));
        }
    }

    fn clear_cache(&mut self) {
        let before = self.store.size() + store::dir_size(&self.paths.audio_cache());
        self.store.clear();
        let audio = self.paths.audio_cache();
        if let Ok(entries) = std::fs::read_dir(&audio) {
            for entry in entries.flatten() {
                let path = entry.path();
                let _ =
                    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            }
        }
        self.refreshed.clear();
        self.ui.send(Event::Info(format!("Cache vidé ({}).", human_bytes(before))));
    }

    // ----------------------------------------------------------------------
    // Playback

    fn ensure_audio(&mut self) -> Option<&Audio> {
        if self.audio.as_ref().is_some_and(|a| a.player.is_invalid()) {
            self.audio = None;
        }
        if self.audio.is_none() {
            let session = self.session.clone()?;
            let mixer = match SoftMixer::open(MixerConfig::default()) {
                Ok(m) => m,
                Err(e) => {
                    self.ui.error(format!("Mixeur audio indisponible : {e}"));
                    return None;
                }
            };
            mixer.set_volume(volume_to_u16(self.settings.volume));
            let config = PlayerConfig {
                bitrate: bitrate(self.quality),
                normalisation: self.settings.normalisation,
                gapless: true,
                ..PlayerConfig::default()
            };
            let backend = audio_backend::find(None)?;
            let player = Player::new(config, session, mixer.get_soft_volume(), move || {
                backend(None, AudioFormat::default())
            });
            let events = player.get_player_event_channel();
            self.audio_failed = false;
            self.loaded = false;
            self.pending_loads = 0;
            self.audio = Some(Audio {
                player,
                mixer,
                events,
                quality: self.quality,
                normalisation: self.settings.normalisation,
            });
        }
        self.audio.as_ref()
    }

    fn load(&mut self, track: Track, play: bool, position_ms: u32) {
        self.attempt_quality = None;
        self.load_generation += 1;
        self.load_started = Instant::now();
        self.flush_listened();
        self.ui.send(Event::NowPlaying(Some(track.clone())));
        self.position_ms = position_ms;
        self.position_at = Instant::now();
        self.playing = false;
        if self.official_mode() {
            return self.load_official(track, play, position_ms);
        }
        if self.reconnect_if_needed() {
            self.pending_play = play;
            self.send_playback(true);
            self.send_queue();
            return;
        }
        let Ok(id) = SpotifyId::from_base62(&track.id) else {
            return self.ui.error(format!("Identifiant de titre invalide : {}", track.id));
        };
        let Some(audio) = self.ensure_audio() else {
            return self.ui.error("Lecteur audio indisponible.");
        };
        audio.player.load(SpotifyUri::Track { id }, play, position_ms);
        self.pending_loads += 1;
        self.loaded = true;
        self.send_playback(play);
        self.send_queue();
        self.refresh_liked_state(&track);
    }

    fn play_pause(&mut self) {
        let Some(track) = self.queue.current().cloned() else { return };
        if self.official_mode() {
            return self.official_play_pause(track);
        }
        if self.playing {
            if let Some(audio) = &self.audio {
                audio.player.pause();
            }
        } else if self.loaded
            && self.pending_loads == 0
            && self.audio.as_ref().is_some_and(|a| !a.player.is_invalid())
            && self.session.as_ref().is_some_and(|s| !s.is_invalid())
        {
            if let Some(audio) = &self.audio {
                audio.player.play();
            }
        } else {
            let position = self.position_ms;
            self.load(track, true, position);
        }
    }

    fn skip(&mut self, auto: bool) {
        match self.queue.advance(auto) {
            Some(track) => self.load(track, true, 0),
            None => {
                if let Some(audio) = &self.audio {
                    audio.player.stop();
                }
                if let Some(engine) = &self.official {
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
        match self.queue.back() {
            Some(track) => self.load(track, true, 0),
            None => self.seek_to(0),
        }
    }

    fn seek_to(&mut self, ms: u32) {
        if self.official_mode() {
            if self.loaded
                && let Some(engine) = &self.official
            {
                engine.send(EngineCommand::Seek(ms));
                self.tracker.seeked(ms, Instant::now());
            }
        } else if let Some(audio) = &self.audio {
            audio.player.seek(ms);
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
            let elapsed = self.position_at.elapsed().as_millis() as u64;
            if self.official_mode() {
                self.official_listened_ms += elapsed;
            } else {
                self.listened_ms += elapsed;
            }
            self.position_at = Instant::now();
        }
    }

    fn send_playback(&self, buffering: bool) {
        self.ui.send(Event::Playback { playing: self.playing, buffering, position_ms: self.position_ms });
    }

    fn send_queue(&self) {
        self.ui.send(Event::Queue {
            upcoming: self.queue.upcoming(100),
            shuffle: self.queue.shuffle(),
            repeat: self.queue.repeat(),
        });
    }

    /// A track could not be played: explain why, and avoid hammering Spotify.
    fn on_unavailable(&mut self) {
        self.playing = false;
        self.loaded = false;
        self.unavailable_streak += 1;
        let current = self.queue.current().cloned();
        let name = current.as_ref().map(|t| t.name.clone()).unwrap_or_default();
        let messages = crate::logger::playback_messages_since(self.load_started);
        let reason = PlaybackFailure::from_messages(&messages);
        log::warn!("track unavailable ({reason:?}): {messages:?}");

        if reason == PlaybackFailure::KeyDenied {
            // Spotify may refuse the key of one file format and accept another:
            // try the same track again at the next higher quality.
            if let (Some(track), Some(next)) = (current.clone(), next_quality(self.quality)) {
                let first = self.attempt_quality.unwrap_or(self.quality);
                self.unavailable_streak -= 1;
                self.ui.send(Event::Notice {
                    key: "quality",
                    text: format!(
                        "Clé refusée en {} kbit/s : nouvel essai en {} kbit/s…",
                        self.quality.kbps(),
                        next.kbps()
                    ),
                    error: false,
                });
                self.quality = next;
                if let Some(audio) = self.audio.take() {
                    audio.player.stop();
                    tokio::task::spawn_blocking(move || drop(audio));
                }
                self.load(track, true, 0);
                self.attempt_quality = Some(first);
                return;
            }
            // Refused in every quality. Remember the track so it is not retried
            // (no wasted data) and keep going with the others, back at the
            // quality the attempts started from.
            if let Some(first) = self.attempt_quality.take()
                && first != self.quality
            {
                self.quality = first;
                if let Some(audio) = self.audio.take() {
                    audio.player.stop();
                    tokio::task::spawn_blocking(move || drop(audio));
                }
            }
            if let Some(track) = &current {
                self.remember_refused(track.id.clone());
            }
            self.skipped_refused += 1;
            if self.unavailable_streak >= MAX_REFUSED_IN_A_ROW {
                self.unavailable_streak = 0;
                self.ui.send(Event::Notice {
                    key: "refused",
                    text: format!(
                        "Lecture arrêtée : Spotify a refusé les {MAX_REFUSED_IN_A_ROW} derniers titres à librespot. \
                         Le moteur officiel (Réglages → Lecture) les lit avec le DRM de Windows."
                    ),
                    error: true,
                });
                self.send_playback(false);
                return;
            }
            let count = self.skipped_refused;
            self.ui.send(Event::Notice {
                key: "refused",
                text: format!(
                    "{count} titre{} ignoré{} : Spotify refuse {} clé de déchiffrement à librespot (code 0x0001). \
                     Le moteur officiel (Réglages → Lecture) peut {} lire.",
                    if count > 1 { "s" } else { "" },
                    if count > 1 { "s" } else { "" },
                    if count > 1 { "leur" } else { "sa" },
                    if count > 1 { "les" } else { "le" },
                ),
                error: false,
            });
            self.send_playback(false);
            self.schedule_skip(Duration::from_millis(700));
            return;
        }

        if self.unavailable_streak >= 3 {
            if matches!(reason, PlaybackFailure::KeyDeniedTemporarily | PlaybackFailure::KeyTimeout)
                && !self.reconnect_tried
            {
                // Keys time out or are refused temporarily over this access point:
                // try a fresh connection once before giving up.
                self.reconnect_tried = true;
                self.unavailable_streak = 0;
                self.ui.error("Spotify ne délivre pas les clés audio : reconnexion puis nouvel essai…");
                if let Some(session) = self.session.take() {
                    session.shutdown();
                }
                if let Some(track) = current {
                    self.load(track, true, 0);
                }
                return;
            }
            self.unavailable_streak = 0;
            self.reconnect_tried = false;
            self.ui.error(reason.stop_message());
            self.send_playback(false);
            return;
        }
        self.ui.error(format!("« {name} » : {} Titre suivant…", reason.short()));
        self.send_playback(false);
        // A pause before the next attempt: skipping at full speed only triggers
        // Spotify's rate limits.
        self.schedule_skip(Duration::from_millis(1500));
    }

    fn schedule_skip(&self, delay: Duration) {
        let (internal, generation) = (self.internal.clone(), self.load_generation);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            let _ = internal.send(Internal::SkipAfterUnavailable(generation));
        });
    }

    /// Forgets the refused tracks: they will be tried again.
    fn clear_refused(&mut self) {
        self.refused.clear();
        self.store.save("refused", &self.refused);
        self.queue.set_blocked(HashSet::new());
        self.ui.send(Event::Refused(HashSet::new()));
    }

    fn remember_refused(&mut self, id: String) {
        self.refused.insert(id.clone(), now_secs());
        self.store.save("refused", &self.refused);
        self.queue.block(id.clone());
        self.ui.send(Event::TrackRefused(id));
    }

    // ----------------------------------------------------------------------
    // Official engine (Spotify's Web Playback SDK in WebView2)

    fn official_mode(&self) -> bool {
        self.settings.engine == Engine::Official
    }

    /// Refused tracks are only skipped (and greyed out) with librespot: the
    /// official engine plays them.
    fn sync_blocked(&mut self) {
        let ids: HashSet<String> =
            if self.official_mode() { HashSet::new() } else { self.refused.keys().cloned().collect() };
        self.queue.set_blocked(ids.clone());
        self.ui.send(Event::Refused(ids));
    }

    fn switch_engine(&mut self, position: u32, playing: bool) {
        log::info!("playback engine: {:?}", self.settings.engine);
        self.playing = false;
        self.loaded = false;
        self.sync_blocked();
        if self.official_mode() {
            if let Some(audio) = self.audio.take() {
                audio.player.stop();
                tokio::task::spawn_blocking(move || drop(audio));
            }
            self.check_playback_scopes(true);
            // Started right away: it is ready by the time a track is chosen.
            self.ensure_official();
        } else {
            self.stop_official();
            self.ui.send(Event::EngineStatus("Arrêté".into()));
        }
        match self.queue.current().cloned() {
            Some(track) => self.load(track, playing, position),
            None => self.send_playback(false),
        }
    }

    /// Checks that the authorization allows the official engine to play.
    fn check_playback_scopes(&self, interactive: bool) {
        if !self.app_connected {
            return;
        }
        let (api, internal) = (self.api.clone(), self.internal.clone());
        tokio::spawn(async move {
            if let Ok(missing) = api.missing_scopes(auth::OFFICIAL_ENGINE_SCOPES).await {
                let _ = internal.send(Internal::OfficialScopes(missing, interactive));
            }
        });
    }

    /// The authorization of the user's application predates the official engine:
    /// it must be accepted again with the playback scopes.
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
                key: "engine",
                text:
                    "Le moteur officiel a besoin de l'autorisation de lecture de votre application Spotify : \
                       acceptez-la dans le navigateur qui vient de s'ouvrir."
                        .into(),
                error: false,
            });
            return self.start_app_login(app);
        }
        self.send_app_status();
        self.ui.send(Event::Notice {
            key: "engine",
            text: "Moteur officiel : autorisez la lecture dans Réglages → Application Spotify → « Autoriser la lecture ». \
                   Si Spotify refuse encore, cochez « Web Playback SDK » dans les réglages de votre application \
                   (tableau de bord Spotify → Settings → Edit)."
                .into(),
            error: true,
        });
    }

    fn ensure_official(&mut self) -> bool {
        if self.official.is_some() {
            return true;
        }
        if !self.app_connected {
            self.ui.error(
                "Le moteur officiel lit avec votre application Spotify : connectez-la d'abord \
                 (Réglages → Application Spotify).",
            );
            return false;
        }
        self.official_id += 1;
        let (id, internal) = (self.official_id, self.internal.clone());
        log::info!("starting the official engine");
        self.device_id = None;
        self.drm.clear();
        self.ui.send(Event::EngineStatus("Démarrage…".into()));
        self.official =
            Some(official::Engine::start(self.paths.webview(), self.settings.volume, move |event| {
                let _ = internal.send(Internal::Official(id, event));
            }));
        true
    }

    fn stop_official(&mut self) {
        if self.official.take().is_some() {
            log::info!("official engine stopped");
            self.ui.send(Event::EngineMemory(0));
        }
        self.device_id = None;
        self.official_pending = None;
        self.tracker.clear();
    }

    fn load_official(&mut self, track: Track, play: bool, position_ms: u32) {
        self.tracker.clear();
        self.official_pending = None;
        self.loaded = false;
        self.send_queue();
        self.refresh_liked_state(&track);
        if !play || !self.ensure_official() {
            if let Some(engine) = &self.official {
                engine.send(EngineCommand::Pause);
            }
            return self.send_playback(false);
        }
        self.tracker.expect(track.id.clone(), track.duration_ms);
        self.loaded = true;
        self.send_playback(true);
        self.official_play(self.load_generation, track.id, position_ms);
    }

    /// Asks Spotify to play the track on the official engine (one Web API call).
    fn official_play(&mut self, generation: u64, track_id: String, position_ms: u32) {
        let Some(device) = self.device_id.clone() else {
            // Started when the engine announces itself.
            self.official_pending = Some((generation, track_id, position_ms));
            return;
        };
        let (api, internal) = (self.api.clone(), self.internal.clone());
        tokio::spawn(async move {
            let mut attempt = 0;
            let result = loop {
                match api.play_on_device(&device, &track_id, position_ms).await {
                    // Spotify's servers learn about a new device a moment after it is ready.
                    Err(ApiError::NotFound) if attempt < 3 => {
                        attempt += 1;
                        tokio::time::sleep(Duration::from_millis(700 * attempt)).await;
                    }
                    other => break other,
                }
            };
            if let Err(e) = result {
                let _ = internal.send(Internal::OfficialPlayFailed(generation, e));
            }
        });
    }

    fn official_play_pause(&mut self, track: Track) {
        if self.playing {
            if let Some(engine) = &self.official {
                engine.send(EngineCommand::Pause);
            }
            let position = self.current_position();
            self.flush_listened();
            self.playing = false;
            self.position_ms = position;
            self.position_at = Instant::now();
            self.tracker.paused(Instant::now());
            self.send_playback(false);
        } else if self.loaded && self.tracker.started() && self.device_id.is_some() {
            if let Some(engine) = &self.official {
                engine.send(EngineCommand::Resume);
            }
        } else {
            let position = self.position_ms;
            self.load(track, true, position);
        }
    }

    fn on_official(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Ready(device) => {
                log::info!("official engine ready");
                self.device_id = Some(device);
                self.send_engine_status();
                if let Some((generation, track_id, position)) = self.official_pending.take()
                    && generation == self.load_generation
                {
                    self.official_play(generation, track_id, position);
                }
            }
            EngineEvent::NotReady => {
                self.device_id = None;
                self.ui.send(Event::EngineStatus("Reconnexion à Spotify…".into()));
            }
            EngineEvent::NeedToken => {
                let Some(sender) = self.official.as_ref().map(official::Engine::sender) else { return };
                let (api, internal) = (self.api.clone(), self.internal.clone());
                tokio::spawn(async move {
                    match api.access_token().await {
                        Ok(token) => sender.send(EngineCommand::Token(token)),
                        Err(e) => {
                            let _ = internal.send(Internal::OfficialTokenFailed(e.to_string()));
                        }
                    }
                });
            }
            EngineEvent::State(Some(state)) => match self.tracker.on_state(&state, Instant::now()) {
                Progress::Ignore => {}
                Progress::Started => {
                    self.unavailable_streak = 0;
                    self.official_progress(&state);
                }
                Progress::Update => self.official_progress(&state),
                Progress::Ended => {
                    self.flush_listened();
                    self.playing = false;
                    self.skip(true);
                }
            },
            EngineEvent::State(None) => {
                // Playback moved to another device (phone, other computer…).
                if self.tracker.started() {
                    let position = self.current_position();
                    self.flush_listened();
                    self.playing = false;
                    self.loaded = false;
                    self.position_ms = position;
                    self.position_at = Instant::now();
                    self.tracker.clear();
                    self.send_playback(false);
                    self.ui.send(Event::Info("La lecture continue sur un autre appareil Spotify.".into()));
                }
            }
            EngineEvent::Error(kind, message) => self.on_official_error(kind, message),
            EngineEvent::Drm(systems) => {
                log::info!("official engine DRM systems: {systems:?}");
                self.drm = if systems.iter().any(|s| s.contains("playready")) {
                    "PlayReady".into()
                } else if systems.iter().any(|s| s.contains("widevine")) {
                    "Widevine".into()
                } else {
                    String::new()
                };
                if systems.is_empty() {
                    self.ui.error(
                        "WebView2 ne propose aucun DRM (PlayReady) sur ce PC : le lecteur officiel risque de ne rien lire. \
                         Mettez Windows et Microsoft Edge WebView2 à jour.",
                    );
                }
                self.send_engine_status();
            }
            EngineEvent::Log(message) => log::info!("official engine: {message}"),
            EngineEvent::Bytes(bytes) => self.official_bytes += bytes,
            EngineEvent::Memory(bytes) => self.ui.send(Event::EngineMemory(bytes)),
            EngineEvent::Failed(message) => {
                self.stop_official();
                self.playing = false;
                self.loaded = false;
                self.send_playback(false);
                self.ui.error(format!("Moteur officiel : {message}"));
                self.ui.send(Event::EngineStatus(format!("Arrêté : {message}")));
            }
        }
    }

    fn send_engine_status(&self) {
        let mut text =
            if self.device_id.is_some() { "Prêt".to_string() } else { "Démarrage…".to_string() };
        if !self.drm.is_empty() {
            text.push_str(&format!(" · DRM {}", self.drm));
        }
        self.ui.send(Event::EngineStatus(text));
    }

    fn official_progress(&mut self, state: &PlayerState) {
        self.flush_listened();
        self.playing = !state.paused && !state.loading;
        self.position_ms = state.position_ms;
        self.position_at = Instant::now();
        self.send_playback(state.loading);
    }

    fn on_official_error(&mut self, kind: ErrorKind, message: String) {
        log::warn!("official engine error {kind:?}: {message}");
        let fatal = match kind {
            ErrorKind::Playback => return self.official_track_failed(message),
            ErrorKind::Authentication => {
                self.stop_official();
                self.ask_playback_auth(true);
                true
            }
            ErrorKind::Account => {
                self.ui.error(
                    "Spotify refuse ce compte au lecteur officiel : un abonnement Premium est nécessaire.",
                );
                true
            }
            ErrorKind::Initialization => {
                self.ui.error(format!(
                    "Le lecteur officiel de Spotify ne peut pas démarrer dans WebView2 ({message}). \
                     Mettez Windows et Microsoft Edge WebView2 à jour."
                ));
                true
            }
            ErrorKind::Load => {
                self.ui.error("Lecteur officiel injoignable (sdk.scdn.co) : vérifiez la connexion Internet.");
                true
            }
            ErrorKind::Autoplay => {
                self.ui.error("WebView2 a bloqué le démarrage du son : relancez la lecture.");
                false
            }
            ErrorKind::Other => {
                self.ui.error(format!("Moteur officiel : {message}"));
                false
            }
        };
        if fatal {
            self.stop_official();
            self.ui.send(Event::EngineStatus("Arrêté (erreur)".into()));
        }
        self.playing = false;
        self.loaded = false;
        self.send_playback(false);
    }

    /// One track could not be played by the official engine.
    fn official_track_failed(&mut self, message: String) {
        self.playing = false;
        self.loaded = false;
        self.tracker.clear();
        self.unavailable_streak += 1;
        self.send_playback(false);
        if self.unavailable_streak >= 3 {
            self.unavailable_streak = 0;
            return self.ui.error(format!(
                "Plusieurs titres d'affilée impossibles à lire avec le moteur officiel ({message}) : lecture arrêtée."
            ));
        }
        let name = self.queue.current().map(|t| t.name.clone()).unwrap_or_default();
        self.ui.error(format!("« {name} » : lecture impossible ({message}). Titre suivant…"));
        self.schedule_skip(Duration::from_millis(1500));
    }

    fn on_official_play_failed(&mut self, error: ApiError) {
        log::warn!("official engine play request failed: {error}");
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
                self.stop_official();
                self.ui.error("Spotify ne trouve pas le moteur officiel : relancez la lecture.");
            }
            ApiError::NotConnected => self.ui.error(
                "Connectez votre application Spotify (Réglages → Application Spotify) pour utiliser le moteur officiel.",
            ),
            other => self.ui.error(format!("Lecture impossible : {other}")),
        }
    }

    fn handle_player(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::PlayRequestIdChanged { .. } => {
                self.pending_loads = self.pending_loads.saturating_sub(1);
            }
            _ if self.pending_loads > 0 => {}
            PlayerEvent::Playing { position_ms, .. } => {
                self.flush_listened();
                self.unavailable_streak = 0;
                self.reconnect_tried = false;
                if let Some(first) = self.attempt_quality.take() {
                    log::warn!(
                        "key refused at {} kbit/s, accepted at {} kbit/s",
                        first.kbps(),
                        self.quality.kbps()
                    );
                    self.ui.send(Event::Notice {
                        key: "quality",
                        text: format!(
                            "Lecture en {} kbit/s : Spotify refuse les clés en {} kbit/s. Cette qualité est gardée jusqu'à la fermeture.",
                            self.quality.kbps(),
                            first.kbps()
                        ),
                        error: false,
                    });
                }
                self.playing = true;
                self.position_ms = position_ms;
                self.position_at = Instant::now();
                self.send_playback(false);
            }
            PlayerEvent::Paused { position_ms, .. } => {
                self.flush_listened();
                self.playing = false;
                self.position_ms = position_ms;
                self.position_at = Instant::now();
                self.send_playback(false);
            }
            PlayerEvent::Seeked { position_ms, .. } | PlayerEvent::PositionCorrection { position_ms, .. } => {
                self.flush_listened();
                self.position_ms = position_ms;
                self.position_at = Instant::now();
                self.send_playback(false);
            }
            PlayerEvent::Loading { position_ms, .. } => {
                self.position_ms = position_ms;
                self.send_playback(true);
            }
            PlayerEvent::Stopped { .. } => {
                self.flush_listened();
                self.playing = false;
                self.loaded = false;
                self.send_playback(false);
            }
            PlayerEvent::TimeToPreloadNextTrack { .. } => {
                if let (Some(audio), Some(next)) = (&self.audio, self.queue.peek_next())
                    && let Ok(id) = SpotifyId::from_base62(&next.id)
                {
                    audio.player.preload(SpotifyUri::Track { id });
                }
            }
            PlayerEvent::EndOfTrack { .. } => {
                self.flush_listened();
                self.playing = false;
                self.skip(true);
            }
            PlayerEvent::Unavailable { .. } => self.on_unavailable(),
            _ => {}
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
            ViewKey::Playlist(id) => {
                let summary = self.playlists.get(&id).cloned();
                let title = summary.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Playlist".into());
                let subtitle = summary.as_ref().map(|p| p.owner.clone()).unwrap_or_default();
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
                        tracks: Arc::new(c.tracks.clone()),
                    });
                    if cache_valid && !force {
                        return;
                    }
                } else {
                    ui.send(Event::Loading(view.clone()));
                }
                let session = self.session.clone();
                let snapshot = summary.map(|s| s.snapshot_id).unwrap_or_default();
                tokio::spawn(async move {
                    let result = match api.playlist_tracks(&id).await {
                        Ok(tracks) => Ok(tracks),
                        Err(e @ (ApiError::Forbidden(_) | ApiError::NotFound | ApiError::RateLimited(_))) => {
                            log::info!("web api refused playlist {id} ({e}), using streaming protocol");
                            match session {
                                Some(s) => {
                                    playlist_via_session(&s, &id).await.map_err(|m| format!("{e} / {m}"))
                                }
                                None => Err(e.to_string()),
                            }
                        }
                        Err(e) => Err(e.to_string()),
                    };
                    match result {
                        Ok(tracks) => {
                            store.save(
                                &key,
                                &CachedTracks { snapshot, saved_at: now_secs(), tracks: tracks.clone() },
                            );
                            ui.send(Event::Tracks { view, title, subtitle, tracks: Arc::new(tracks) });
                        }
                        Err(message) => ui.send(Event::ViewFailed { view, message }),
                    }
                });
            }
            ViewKey::Album(id) => {
                let key = format!("album-{id}");
                if !force && let Some(c) = store.load::<CachedAlbum>(&key) {
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
                if !force
                    && let Some(c) = store.load::<CachedArtist>(&key)
                    && now_secs().saturating_sub(c.saved_at) < 86_400
                {
                    return ui.send(Event::Artist {
                        id,
                        name: c.name,
                        top: Arc::new(c.top),
                        albums: c.albums,
                    });
                }
                ui.send(Event::Loading(view.clone()));
                let session = self.session.clone();
                tokio::spawn(async move {
                    match api.artist(&id).await {
                        Ok((name, mut top, albums)) => {
                            if top.is_empty()
                                && let Some(session) = &session
                            {
                                top = artist_top_via_session(session, &id).await;
                            }
                            store.save(
                                &key,
                                &CachedArtist {
                                    saved_at: now_secs(),
                                    name: name.clone(),
                                    top: top.clone(),
                                    albums: albums.clone(),
                                },
                            );
                            ui.send(Event::Artist { id, name, top: Arc::new(top), albums });
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
            ViewKey::Queue | ViewKey::Settings | ViewKey::Welcome => {}
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
                    tracks: Arc::new(c.tracks),
                }),
                ViewKey::SavedAlbums => store
                    .load::<Vec<AlbumSummary>>("albums")
                    .map(|albums| Event::Albums { view: view.clone(), title: "Albums".into(), albums }),
                ViewKey::Playlist(id) => store.load::<CachedTracks>(&format!("playlist-{id}")).map(|c| {
                    let summary = self.playlists.get(id);
                    Event::Tracks {
                        view: view.clone(),
                        title: summary.map(|p| p.name.clone()).unwrap_or_default(),
                        subtitle: summary.map(|p| p.owner.clone()).unwrap_or_default(),
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
                message: if self.app_connected {
                    "Hors ligne : contenu non disponible dans le cache.".into()
                } else {
                    "Connectez votre application Spotify pour charger ce contenu.".into()
                },
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

async fn next_player_event(audio: &mut Option<Audio>) -> Option<PlayerEvent> {
    match audio {
        Some(audio) => audio.events.recv().await,
        None => std::future::pending().await,
    }
}

/// Why librespot gave up on a track, deduced from what it logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackFailure {
    /// Spotify refused the decryption key for good (code 0x0001).
    KeyDenied,
    /// Spotify refused the key temporarily (0x0002), even after retries.
    KeyDeniedTemporarily,
    KeyTimeout,
    Region,
    Format,
    Download,
    Unknown,
}

impl PlaybackFailure {
    fn from_messages(messages: &[String]) -> Self {
        let has = |needle: &str| messages.iter().any(|m| m.contains(needle));
        if has("audio key error 0x0001") || has("error audio key 0 1") {
            Self::KeyDenied
        } else if has("audio key error 0x0002") || has("error audio key 0 2") {
            Self::KeyDeniedTemporarily
        } else if has("Audio key response timeout") || has("audio key response timeout") {
            Self::KeyTimeout
        } else if has("in any supported format") {
            Self::Format
        } else if has("Track is unavailable") || has("is not available") {
            Self::Region
        } else if has("Unable to load encrypted file") || has("Unable to load audio item") {
            Self::Download
        } else {
            Self::Unknown
        }
    }

    fn short(self) -> &'static str {
        match self {
            Self::KeyDenied => "Spotify refuse la clé de déchiffrement (code 0x0001).",
            Self::KeyDeniedTemporarily => "Spotify refuse temporairement la clé de déchiffrement (0x0002).",
            Self::KeyTimeout => "Spotify ne répond pas à la demande de clé audio.",
            Self::Region => "titre non disponible pour ce compte ou ce pays.",
            Self::Format => "aucun format audio compatible.",
            Self::Download => "téléchargement impossible.",
            Self::Unknown => "lecture impossible.",
        }
    }

    fn stop_message(self) -> String {
        match self {
            Self::KeyDenied | Self::KeyDeniedTemporarily | Self::KeyTimeout => format!(
                "Lecture arrêtée : {} Ce refus vient des serveurs de Spotify et touche en ce moment \
                 d'autres lecteurs basés sur librespot. Réessayez un peu plus tard.",
                self.short()
            ),
            _ => {
                format!("Plusieurs titres d'affilée impossibles à lire ({}) : lecture arrêtée.", self.short())
            }
        }
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
    Event::Tracks { view, title: cached.album.name, subtitle, tracks: Arc::new(cached.tracks) }
}

/// Reads a playlist through Spotify's streaming protocol (used when the Web API
/// refuses it: development mode applications only read the user's own playlists).
async fn playlist_via_session(session: &Session, id: &str) -> Result<Vec<Track>, String> {
    use librespot_metadata::{Metadata, Playlist};
    let uri = SpotifyUri::from_uri(&format!("spotify:playlist:{id}")).map_err(|e| e.to_string())?;
    let playlist = Playlist::get(session, &uri).await.map_err(|e| e.to_string())?;
    let uris: Vec<SpotifyUri> =
        playlist.tracks().filter(|u| matches!(u, SpotifyUri::Track { .. })).cloned().collect();
    Ok(tracks_via_session(session, uris).await)
}

/// Most popular tracks of an artist in the user's country, read through the
/// streaming protocol (the Web API endpoint is closed to development mode apps).
async fn artist_top_via_session(session: &Session, id: &str) -> Vec<Track> {
    use librespot_metadata::{Artist, Metadata};
    let Ok(uri) = SpotifyUri::from_uri(&format!("spotify:artist:{id}")) else { return Vec::new() };
    let Ok(artist) = Artist::get(session, &uri).await else { return Vec::new() };
    let top = artist.top_tracks.for_country(&session.country());
    tracks_via_session(session, top.0.into_iter().take(10).collect()).await
}

/// Track metadata through the streaming protocol, 8 requests at a time.
async fn tracks_via_session(session: &Session, uris: Vec<SpotifyUri>) -> Vec<Track> {
    use librespot_metadata::Metadata;
    futures_util::stream::iter(uris)
        .map(|uri| {
            let session = session.clone();
            async move {
                let t = librespot_metadata::Track::get(&session, &uri).await.ok()?;
                Some(Track {
                    id: uri.to_id().ok()?,
                    name: t.name,
                    artists: t
                        .artists
                        .iter()
                        .map(|a| ArtistRef { id: a.id.to_id().unwrap_or_default(), name: a.name.clone() })
                        .collect(),
                    album: t.album.name.clone(),
                    album_id: t.album.id.to_id().unwrap_or_default(),
                    duration_ms: t.duration.max(0) as u32,
                    image: t
                        .album
                        .covers
                        .iter()
                        .min_by_key(|c| c.width)
                        .and_then(|c| c.id.to_base16().ok())
                        .map(|hex| format!("https://i.scdn.co/image/{hex}")),
                    playable: true,
                })
            }
        })
        .buffered(8)
        .filter_map(std::future::ready)
        .collect::<Vec<_>>()
        .await
}

fn decode_image(bytes: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory(bytes).ok()?;
    // Covers are displayed at 56 px at most: never keep more pixels than needed.
    let image = if image.width() > 128 { image.thumbnail(128, 128) } else { image };
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

fn volume_to_u16(volume: f32) -> u16 {
    (volume.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16
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
    fn explains_playback_failures() {
        let from = |lines: &[&str]| {
            PlaybackFailure::from_messages(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
        };
        assert_eq!(
            from(&["error audio key 0 1", "Unable to load key, continuing without decryption: x"]),
            PlaybackFailure::KeyDenied
        );
        assert_eq!(
            from(&["Unable to load key: Service unavailable { audio key error 0x0002 }"]),
            PlaybackFailure::KeyDeniedTemporarily
        );
        assert_eq!(from(&["Audio key response timeout"]), PlaybackFailure::KeyTimeout);
        assert_eq!(from(&["spotify:track:<abc> is not available"]), PlaybackFailure::Region);
        assert_eq!(from(&["<x> is not available in any supported format"]), PlaybackFailure::Format);
        assert_eq!(from(&[]), PlaybackFailure::Unknown);
        crate::logger::remember_for_test("error audio key 0 1");
        let recent = crate::logger::playback_messages_since(Instant::now() - Duration::from_secs(5));
        assert_eq!(PlaybackFailure::from_messages(&recent), PlaybackFailure::KeyDenied);
    }

    #[test]
    fn refused_keys_climb_through_qualities() {
        assert_eq!(next_quality(Quality::Eco), Some(Quality::Normal));
        assert_eq!(next_quality(Quality::Normal), Some(Quality::High));
        assert_eq!(next_quality(Quality::High), None);
    }

    #[test]
    fn volume_mapping() {
        assert_eq!(volume_to_u16(0.0), 0);
        assert_eq!(volume_to_u16(1.0), u16::MAX);
        assert_eq!(volume_to_u16(2.0), u16::MAX);
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
