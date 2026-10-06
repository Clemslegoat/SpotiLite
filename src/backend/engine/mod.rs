//! The playback engine: Spotify's own player (Web Playback SDK), hosted in an
//! invisible WebView2, the Edge engine built into Windows.
//!
//! Audio is decrypted by the DRM of the Edge engine (Widevine, or PlayReady, the DRM
//! of Windows), exactly as in a browser. SpotiLite only drives it (play, pause,
//! seek, volume, next) and keeps its own interface and queue.

// The page protocol is only spoken by the Windows host.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod webview;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use serde::Deserialize;

use crate::model::{ArtistRef, Track};

/// The page loaded in WebView2.
pub const PLAYER_HTML: &str = include_str!("player.html");
/// Virtual https origin of that page (EME requires a secure context).
pub const HOST_NAME: &str = "spotilite.example";
/// Where WebView2 can be installed when it is missing (Windows 10 without Edge updates).
pub const RUNTIME_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// Orders for the page.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineCommand {
    Token(String),
    Pause,
    Resume,
    /// Next and previous of what Spotify plays itself (a playlist or an artist).
    Next,
    Previous,
    Seek(u32),
    Volume(f32),
    Shutdown,
}

impl EngineCommand {
    /// The message posted to the page (`None` for commands handled by the host).
    pub fn to_json(&self) -> Option<String> {
        let value = match self {
            Self::Token(token) => serde_json::json!({ "type": "token", "token": token }),
            Self::Pause => serde_json::json!({ "type": "pause" }),
            Self::Resume => serde_json::json!({ "type": "resume" }),
            Self::Next => serde_json::json!({ "type": "next" }),
            Self::Previous => serde_json::json!({ "type": "previous" }),
            Self::Seek(ms) => serde_json::json!({ "type": "seek", "ms": ms }),
            Self::Volume(v) => serde_json::json!({ "type": "volume", "value": v.clamp(0.0, 1.0) }),
            Self::Shutdown => return None,
        };
        Some(value.to_string())
    }
}

/// What the player is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub track_id: Option<String>,
    pub paused: bool,
    pub loading: bool,
    pub position_ms: u32,
    pub duration_ms: u32,
    /// The current track as Spotify describes it (used when Spotify chooses the
    /// tracks itself) and the next ones it announces.
    pub current: Option<Track>,
    pub next: Vec<Track>,
    pub shuffle: bool,
    /// 0: off, 1: whole context, 2: this track.
    pub repeat: u8,
}

/// A track as described by the SDK.
#[derive(Deserialize)]
struct TrackInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<ArtistInfo>,
    #[serde(default)]
    album: String,
    #[serde(default)]
    album_id: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    duration: f64,
}

#[derive(Deserialize)]
struct ArtistInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

impl TrackInfo {
    fn into_track(self) -> Option<Track> {
        (!self.id.is_empty()).then(|| Track {
            id: self.id,
            name: self.name,
            artists: self.artists.into_iter().map(|a| ArtistRef { id: a.id, name: a.name }).collect(),
            album: self.album,
            album_id: self.album_id,
            duration_ms: self.duration.clamp(0.0, f64::from(u32::MAX)) as u32,
            image: self.image.filter(|i| !i.is_empty()),
            playable: true,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The player cannot start in this WebView2 (no usable DRM, typically).
    Initialization,
    /// The token was refused: missing scope or "Web Playback SDK" not enabled.
    Authentication,
    /// Not a Premium account.
    Account,
    /// One track could not be played.
    Playback,
    Autoplay,
    /// The SDK script could not be downloaded.
    Load,
    /// The SDK refused to connect (token rejected: scope, Premium, or "Web
    /// Playback SDK" not enabled for the application).
    Connect,
    Other,
}

impl ErrorKind {
    fn parse(kind: &str) -> Self {
        match kind {
            "initialization_error" => Self::Initialization,
            "authentication_error" => Self::Authentication,
            "account_error" => Self::Account,
            "playback_error" => Self::Playback,
            "autoplay_failed" => Self::Autoplay,
            "load" => Self::Load,
            "connect" => Self::Connect,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// The player is registered as a Spotify device.
    Ready(String),
    NotReady,
    /// The SDK asks for an access token.
    NeedToken,
    /// `None` when this device is no longer the active one.
    State(Option<PlayerState>),
    Error(ErrorKind, String),
    /// DRM systems available in this WebView2.
    Drm(Vec<String>),
    Log(String),
    /// A media key or media panel action that reached the page ("play",
    /// "pause", "nexttrack"…): handled by SpotiLite like its own buttons.
    Media(String),
    /// Bytes received by the WebView2 (data usage).
    Bytes(u64),
    /// Memory used by the WebView2 processes (private working set).
    Memory(u64),
    /// The engine stopped or could not start.
    Failed(String),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Ready {
        device_id: String,
    },
    NotReady {},
    NeedToken {},
    State {
        #[serde(default)]
        active: bool,
        #[serde(default)]
        paused: bool,
        #[serde(default)]
        loading: bool,
        #[serde(default)]
        position: f64,
        #[serde(default)]
        duration: f64,
        #[serde(default)]
        track: Option<String>,
        #[serde(default)]
        current: Option<TrackInfo>,
        #[serde(default)]
        next: Vec<Option<TrackInfo>>,
        #[serde(default)]
        shuffle: bool,
        #[serde(default)]
        repeat: u8,
    },
    Error {
        kind: String,
        #[serde(default)]
        message: String,
    },
    Drm {
        #[serde(default)]
        systems: Vec<String>,
    },
    Log {
        #[serde(default)]
        message: String,
    },
    Media {
        #[serde(default)]
        action: String,
    },
}

/// Decodes a message posted by the page.
pub fn parse_message(json: &str) -> Option<EngineEvent> {
    let message: Message = serde_json::from_str(json).ok()?;
    Some(match message {
        Message::Ready { device_id } => EngineEvent::Ready(device_id),
        Message::NotReady {} => EngineEvent::NotReady,
        Message::NeedToken {} => EngineEvent::NeedToken,
        Message::State { active: false, .. } => EngineEvent::State(None),
        Message::State {
            paused, loading, position, duration, track, current, next, shuffle, repeat, ..
        } => EngineEvent::State(Some(PlayerState {
            track_id: track.filter(|t| !t.is_empty()),
            paused,
            loading,
            position_ms: position.clamp(0.0, f64::from(u32::MAX)) as u32,
            duration_ms: duration.clamp(0.0, f64::from(u32::MAX)) as u32,
            current: current.and_then(TrackInfo::into_track),
            next: next.into_iter().flatten().filter_map(TrackInfo::into_track).collect(),
            shuffle,
            repeat: repeat.min(2),
        })),
        Message::Error { kind, message } => EngineEvent::Error(ErrorKind::parse(&kind), message),
        Message::Drm { systems } => EngineEvent::Drm(systems),
        Message::Log { message } => EngineEvent::Log(message),
        Message::Media { action } => EngineEvent::Media(action),
    })
}

/// Sends orders to the engine thread (cheap to clone, usable from any task).
#[derive(Clone)]
pub struct EngineSender {
    tx: std::sync::mpsc::Sender<EngineCommand>,
    /// Id of the engine thread once its message queue exists (0 before).
    thread_id: Arc<AtomicU32>,
}

impl EngineSender {
    pub fn send(&self, command: EngineCommand) {
        if self.tx.send(command).is_ok() {
            #[cfg(windows)]
            webview::wake(self.thread_id.load(std::sync::atomic::Ordering::Acquire));
            #[cfg(not(windows))]
            let _ = &self.thread_id;
        }
    }
}

/// The running engine; dropping it closes the WebView2 and its processes.
pub struct Engine {
    sender: EngineSender,
}

/// How the WebView2 processes are configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Fewer processes and no GPU process: the page is invisible and only plays audio.
    Lean,
    /// WebView2 defaults, used if the lean profile cannot play on this PC.
    Compatible,
}

impl Profile {
    /// Chromium switches of the profile.
    pub fn browser_arguments(self) -> String {
        let mut args = vec![
            // Sound must start without a click in the (invisible) page.
            "--autoplay-policy=no-user-gesture-required",
            // The window is never shown: keep the page running at full speed.
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
            "--disable-backgrounding-occluded-windows",
            // Audio segments use one-time URLs: a big HTTP cache would only fill the disk.
            "--disk-cache-size=8388608",
        ];
        let mut disabled = vec![
            // SpotiLite has its own media keys and Windows media panel entry.
            "HardwareMediaKeyHandling",
            "MediaSessionService",
        ];
        if self == Self::Lean {
            args.extend([
                // Nothing is displayed: no GPU process, no GPU driver in memory.
                "--disable-gpu",
                "--disable-gpu-compositing",
                "--disable-software-rasterizer",
                // The page and Spotify's player share one renderer process.
                "--renderer-process-limit=1",
                "--disable-site-isolation-trials",
            ]);
            disabled.extend([
                "SpareRendererForSitePerProcess",
                // Audio output in the browser process instead of a separate one.
                "AudioServiceOutOfProcess",
                "CalculateNativeWinOcclusion",
                "msSmartScreenProtection",
                "msWebOOUI",
                "msPdfOOUI",
            ]);
        }
        let mut line = args.join(" ");
        line.push_str(" --disable-features=");
        line.push_str(&disabled.join(","));
        line
    }
}

impl Engine {
    /// Starts the engine on its own thread. `dir` holds the WebView2 profile;
    /// `events` is called from that thread.
    pub fn start(
        dir: PathBuf,
        volume: f32,
        profile: Profile,
        events: impl Fn(EngineEvent) + Send + Sync + 'static,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_id = Arc::new(AtomicU32::new(0));
        let sender = EngineSender { tx, thread_id: thread_id.clone() };
        #[cfg(windows)]
        {
            let arguments =
                std::env::var("SPOTILITE_WEBVIEW_ARGS").unwrap_or_else(|_| profile.browser_arguments());
            log::info!("engine profile {profile:?}: {arguments}");
            let setup =
                webview::Setup { dir, volume, arguments, commands: rx, thread_id, events: Arc::new(events) };
            let spawned = std::thread::Builder::new()
                .name("spotilite-webview".into())
                .spawn(move || webview::run(setup));
            if let Err(e) = spawned {
                log::warn!("webview thread: {e}");
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (dir, volume, profile, rx, thread_id);
            events(EngineEvent::Failed("la lecture n'existe que sous Windows (WebView2)".into()));
        }
        Self { sender }
    }

    pub fn sender(&self) -> EngineSender {
        self.sender.clone()
    }

    pub fn send(&self, command: EngineCommand) {
        self.sender.send(command);
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.sender.send(EngineCommand::Shutdown);
    }
}

/// Outcome of a state report for the track SpotiLite asked to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Unrelated (a previous track, or nothing expected).
    Ignore,
    /// Sound started.
    Started,
    Update,
    /// The track finished (or Spotify moved on to something else by itself).
    Ended,
}

/// How close to the end a stop must happen to count as "finished" (state
/// reports can arrive a little late).
const END_MARGIN_MS: u32 = 4000;

/// Follows the official player to tell when the requested track is over. The
/// queue stays SpotiLite's: one track is sent at a time.
#[derive(Debug, Default)]
pub struct Tracker {
    expected: Option<String>,
    started: bool,
    playing: bool,
    position_ms: u32,
    at: Option<Instant>,
    duration_ms: u32,
}

impl Tracker {
    pub fn expect(&mut self, track_id: String, duration_ms: u32) {
        *self = Self { expected: Some(track_id), duration_ms, ..Self::default() };
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn started(&self) -> bool {
        self.expected.is_some() && self.started
    }

    fn estimate(&self, now: Instant) -> u32 {
        match self.at {
            Some(at) if self.playing => {
                self.position_ms.saturating_add(now.saturating_duration_since(at).as_millis() as u32)
            }
            _ => self.position_ms,
        }
    }

    /// A seek ordered by SpotiLite (so that a jump back is not taken for a restart).
    pub fn seeked(&mut self, position_ms: u32, now: Instant) {
        self.position_ms = position_ms;
        self.at = Some(now);
    }

    pub fn paused(&mut self, now: Instant) {
        self.position_ms = self.estimate(now);
        self.at = Some(now);
        self.playing = false;
    }

    pub fn on_state(&mut self, state: &PlayerState, now: Instant) -> Progress {
        let Some(expected) = &self.expected else { return Progress::Ignore };
        if state.track_id.as_deref() != Some(expected.as_str()) {
            if self.started {
                // Spotify went on by itself (autoplay of similar tracks).
                self.expected = None;
                return Progress::Ended;
            }
            return Progress::Ignore;
        }
        if state.duration_ms > 0 {
            self.duration_ms = state.duration_ms;
        }
        let estimate = self.estimate(now);
        let near_end = self.duration_ms > 0 && estimate.saturating_add(END_MARGIN_MS) >= self.duration_ms;
        // At the end Spotify reports the track paused at 0 (or restarted, if the
        // account has "repeat" enabled).
        if self.started
            && near_end
            && state.position_ms < 3000
            && (state.paused || estimate > state.position_ms + 5000)
        {
            self.expected = None;
            return Progress::Ended;
        }
        let first = !self.started && !state.paused && !state.loading;
        self.started |= first;
        self.playing = !state.paused && !state.loading;
        self.position_ms = state.position_ms;
        self.at = Some(now);
        if first { Progress::Started } else { Progress::Update }
    }

    /// Playing for longer than the track lasts: an end that was never reported.
    pub fn overdue(&self, now: Instant) -> bool {
        self.started && self.playing && self.duration_ms > 0 && self.estimate(now) > self.duration_ms + 6000
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn state(track: &str, paused: bool, position_ms: u32) -> PlayerState {
        PlayerState {
            track_id: Some(track.into()),
            paused,
            position_ms,
            duration_ms: 180_000,
            ..PlayerState::default()
        }
    }

    #[test]
    fn parses_page_messages() {
        assert_eq!(
            parse_message(r#"{"type":"ready","device_id":"abc"}"#),
            Some(EngineEvent::Ready("abc".into()))
        );
        assert_eq!(parse_message(r#"{"type":"not_ready","device_id":"abc"}"#), Some(EngineEvent::NotReady));
        assert_eq!(parse_message(r#"{"type":"need_token"}"#), Some(EngineEvent::NeedToken));
        assert_eq!(
            parse_message(r#"{"type":"media","action":"pause"}"#),
            Some(EngineEvent::Media("pause".into()))
        );
        assert_eq!(parse_message(r#"{"type":"state","active":false}"#), Some(EngineEvent::State(None)));
        assert_eq!(
            parse_message(
                r#"{"type":"state","active":true,"paused":false,"loading":false,"position":1200.7,"duration":180000,"track":"t1"}"#
            ),
            Some(EngineEvent::State(Some(PlayerState {
                track_id: Some("t1".into()),
                position_ms: 1200,
                duration_ms: 180_000,
                ..PlayerState::default()
            })))
        );
        assert_eq!(
            parse_message(
                r#"{"type":"error","kind":"authentication_error","message":"Invalid token scopes."}"#
            ),
            Some(EngineEvent::Error(ErrorKind::Authentication, "Invalid token scopes.".into()))
        );
        assert_eq!(
            parse_message(r#"{"type":"drm","systems":["com.microsoft.playready.recommendation"]}"#),
            Some(EngineEvent::Drm(vec!["com.microsoft.playready.recommendation".into()]))
        );
        // What the SDK does with a rejected token (seen on a real WebView2).
        assert_eq!(
            parse_message(r#"{"type":"error","kind":"connect","message":"connexion du lecteur refusée"}"#),
            Some(EngineEvent::Error(ErrorKind::Connect, "connexion du lecteur refusée".into()))
        );
        assert_eq!(parse_message(r#"{"type":"unknown"}"#), None);
        assert_eq!(parse_message("not json"), None);
    }

    #[test]
    fn reads_track_metadata_from_the_player() {
        let json = r#"{"type":"state","active":true,"paused":true,"position":0,"duration":200000,
            "track":"t9","shuffle":true,"repeat":1,
            "current":{"id":"t9","name":"Song","artists":[{"id":"a1","name":"Artist"}],
                       "album":"Album","album_id":"al","image":"https://i.scdn.co/image/x","duration":200000},
            "next":[{"id":"t10","name":"Next","artists":[],"album":"","album_id":"","image":null,"duration":1000},
                    null, {"id":"","name":"no id"}]}"#;
        let Some(EngineEvent::State(Some(state))) = parse_message(json) else { panic!("not a state") };
        let current = state.current.unwrap();
        assert_eq!(current.name, "Song");
        assert_eq!(current.artists[0].id, "a1");
        assert_eq!(current.album_id, "al");
        assert_eq!(current.image.as_deref(), Some("https://i.scdn.co/image/x"));
        assert_eq!(current.duration_ms, 200_000);
        assert_eq!(state.next.len(), 1, "null and id-less tracks are dropped");
        assert!(state.shuffle);
        assert_eq!(state.repeat, 1);
    }

    #[test]
    fn lean_profile_drops_the_gpu_and_extra_processes() {
        let lean = Profile::Lean.browser_arguments();
        let compatible = Profile::Compatible.browser_arguments();
        for args in [&lean, &compatible] {
            assert!(args.contains("--autoplay-policy=no-user-gesture-required"));
            // Only SpotiLite appears in the Windows media panel.
            assert!(args.contains("HardwareMediaKeyHandling"));
        }
        assert!(lean.contains("--disable-gpu ") && lean.contains("--renderer-process-limit=1"));
        assert!(!compatible.contains("--disable-gpu"));
        assert_eq!(lean.matches("--disable-features=").count(), 1);
    }

    #[test]
    fn commands_become_page_messages() {
        assert_eq!(EngineCommand::Seek(42).to_json().as_deref(), Some(r#"{"ms":42,"type":"seek"}"#));
        assert_eq!(EngineCommand::Volume(3.0).to_json().as_deref(), Some(r#"{"type":"volume","value":1.0}"#));
        assert_eq!(EngineCommand::Next.to_json().as_deref(), Some(r#"{"type":"next"}"#));
        assert_eq!(EngineCommand::Shutdown.to_json(), None);
    }

    #[test]
    fn ignores_previous_track_then_detects_the_end() {
        let t0 = Instant::now();
        let mut tracker = Tracker::default();
        tracker.expect("t1".into(), 180_000);
        // The previous track is still reported until the new one loads.
        assert_eq!(tracker.on_state(&state("t0", false, 50_000), t0), Progress::Ignore);
        assert_eq!(tracker.on_state(&state("t1", false, 0), t0), Progress::Started);
        assert!(tracker.started());
        // A pause in the middle is not an end.
        let t1 = t0 + Duration::from_secs(60);
        assert_eq!(tracker.on_state(&state("t1", true, 60_000), t1), Progress::Update);
        let t2 = t1 + Duration::from_secs(10);
        assert_eq!(tracker.on_state(&state("t1", false, 60_000), t2), Progress::Update);
        // Spotify reports the track paused at 0 when it ends.
        let t3 = t2 + Duration::from_secs(119);
        assert_eq!(tracker.on_state(&state("t1", true, 0), t3), Progress::Ended);
        assert_eq!(tracker.on_state(&state("t1", true, 0), t3), Progress::Ignore);
    }

    #[test]
    fn a_seek_back_is_not_an_end_but_a_repeat_is() {
        let t0 = Instant::now();
        let mut tracker = Tracker::default();
        tracker.expect("t1".into(), 180_000);
        tracker.on_state(&state("t1", false, 0), t0);
        let t1 = t0 + Duration::from_secs(178);
        tracker.seeked(0, t1);
        assert_eq!(tracker.on_state(&state("t1", false, 0), t1), Progress::Update);
        // Played to the end and restarted by the account's "repeat": over.
        let t2 = t1 + Duration::from_secs(179);
        assert_eq!(tracker.on_state(&state("t1", false, 400), t2), Progress::Ended);
    }

    #[test]
    fn autoplay_of_another_track_ends_the_current_one() {
        let t0 = Instant::now();
        let mut tracker = Tracker::default();
        tracker.expect("t1".into(), 180_000);
        tracker.on_state(&state("t1", false, 0), t0);
        assert_eq!(
            tracker.on_state(&state("radio", false, 0), t0 + Duration::from_secs(181)),
            Progress::Ended
        );
    }

    /// Real WebView2 + Spotify's SDK (network needed): `cargo test -- --ignored --nocapture`.
    /// Measures both profiles. An invalid token is given on purpose: the SDK must
    /// load, ask for a token and report an error through the bridge.
    #[cfg(windows)]
    #[test]
    #[ignore = "starts WebView2 and downloads Spotify's SDK"]
    fn webview2_hosts_the_player() {
        let compatible = probe(Profile::Compatible);
        let lean = probe(Profile::Lean);
        println!("SUMMARY compatible: {compatible:?}");
        println!("SUMMARY lean:       {lean:?}");
        for result in [compatible, lean] {
            assert!(result.drm.is_some(), "the player page never loaded");
            assert!(result.reported.is_some(), "the SDK reported nothing");
        }
    }

    #[cfg(windows)]
    #[derive(Debug)]
    struct Probe {
        drm: Option<Vec<String>>,
        reported: Option<ErrorKind>,
        memory_mb: u64,
    }

    #[cfg(windows)]
    fn probe(profile: Profile) -> Probe {
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = std::env::temp_dir().join(format!("spotilite-webview-{}-{profile:?}", std::process::id()));
        let engine = Engine::start(dir.clone(), 0.5, profile, move |event| {
            let _ = tx.send(event);
        });
        let sender = engine.sender();
        let deadline = Instant::now() + Duration::from_secs(90);
        let mut result = Probe { drm: None, reported: None, memory_mb: 0 };
        while Instant::now() < deadline && result.reported.is_none() {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(EngineEvent::Drm(systems)) => result.drm = Some(systems),
                Ok(EngineEvent::NeedToken) => {
                    sender.send(EngineCommand::Token("invalid-token-for-test".into()))
                }
                Ok(EngineEvent::Error(kind, message)) => {
                    println!("{profile:?}: SDK error {kind:?}: {message}");
                    result.reported = Some(kind);
                }
                Ok(EngineEvent::Failed(message)) => panic!("{profile:?}: engine failed: {message}"),
                Ok(EngineEvent::Memory(bytes)) => result.memory_mb = bytes / 1024 / 1024,
                Ok(_) | Err(_) => {}
            }
        }
        // Memory of the WebView2 processes once settled.
        let until = Instant::now() + Duration::from_secs(12);
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            if let Ok(EngineEvent::Memory(bytes)) = rx.recv_timeout(left) {
                result.memory_mb = bytes / 1024 / 1024;
            }
        }
        drop(engine);
        std::thread::sleep(Duration::from_secs(3));
        let _ = std::fs::remove_dir_all(dir);
        result
    }

    #[test]
    fn notices_a_missing_end_report() {
        let t0 = Instant::now();
        let mut tracker = Tracker::default();
        tracker.expect("t1".into(), 180_000);
        tracker.on_state(&state("t1", false, 0), t0);
        assert!(!tracker.overdue(t0 + Duration::from_secs(150)));
        assert!(tracker.overdue(t0 + Duration::from_secs(190)));
        tracker.paused(t0 + Duration::from_secs(10));
        assert!(!tracker.overdue(t0 + Duration::from_secs(400)));
    }
}
