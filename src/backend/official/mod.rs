//! « Moteur officiel » : the playback engine of Spotify itself (Web Playback SDK),
//! hosted in an invisible WebView2, the Edge engine built into Windows.
//!
//! Spotify refuses some audio keys to librespot; its own player never goes through
//! that path: audio is decrypted by the DRM of the Edge engine (Widevine, or
//! PlayReady, the DRM of Windows), exactly as in a browser. SpotiLite only drives it (play, pause, seek, volume) and keeps its own
//! interface and queue. It costs more memory than librespot (the WebView2
//! processes), which is why it is optional.

// The page protocol is only spoken by the Windows host.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod webview;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use serde::Deserialize;

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
            Self::Seek(ms) => serde_json::json!({ "type": "seek", "ms": ms }),
            Self::Volume(v) => serde_json::json!({ "type": "volume", "value": v.clamp(0.0, 1.0) }),
            Self::Shutdown => return None,
        };
        Some(value.to_string())
    }
}

/// What the official player is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerState {
    pub track_id: Option<String>,
    pub paused: bool,
    pub loading: bool,
    pub position_ms: u32,
    pub duration_ms: u32,
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
}

/// Decodes a message posted by the page.
pub fn parse_message(json: &str) -> Option<EngineEvent> {
    let message: Message = serde_json::from_str(json).ok()?;
    Some(match message {
        Message::Ready { device_id } => EngineEvent::Ready(device_id),
        Message::NotReady {} => EngineEvent::NotReady,
        Message::NeedToken {} => EngineEvent::NeedToken,
        Message::State { active: false, .. } => EngineEvent::State(None),
        Message::State { paused, loading, position, duration, track, .. } => {
            EngineEvent::State(Some(PlayerState {
                track_id: track.filter(|t| !t.is_empty()),
                paused,
                loading,
                position_ms: position.clamp(0.0, f64::from(u32::MAX)) as u32,
                duration_ms: duration.clamp(0.0, f64::from(u32::MAX)) as u32,
            }))
        }
        Message::Error { kind, message } => EngineEvent::Error(ErrorKind::parse(&kind), message),
        Message::Drm { systems } => EngineEvent::Drm(systems),
        Message::Log { message } => EngineEvent::Log(message),
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

impl Engine {
    /// Starts the engine on its own thread. `dir` holds the WebView2 profile;
    /// `events` is called from that thread.
    pub fn start(dir: PathBuf, volume: f32, events: impl Fn(EngineEvent) + Send + Sync + 'static) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread_id = Arc::new(AtomicU32::new(0));
        let sender = EngineSender { tx, thread_id: thread_id.clone() };
        #[cfg(windows)]
        {
            let setup = webview::Setup { dir, volume, commands: rx, thread_id, events: Arc::new(events) };
            let spawned = std::thread::Builder::new()
                .name("spotilite-webview".into())
                .spawn(move || webview::run(setup));
            if let Err(e) = spawned {
                log::warn!("webview thread: {e}");
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (dir, volume, rx, thread_id);
            events(EngineEvent::Failed("le moteur officiel n'existe que sous Windows (WebView2)".into()));
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
            loading: false,
            position_ms,
            duration_ms: 180_000,
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
        assert_eq!(parse_message(r#"{"type":"state","active":false}"#), Some(EngineEvent::State(None)));
        assert_eq!(
            parse_message(
                r#"{"type":"state","active":true,"paused":false,"loading":false,"position":1200.7,"duration":180000,"track":"t1"}"#
            ),
            Some(EngineEvent::State(Some(PlayerState {
                track_id: Some("t1".into()),
                paused: false,
                loading: false,
                position_ms: 1200,
                duration_ms: 180_000,
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
    fn commands_become_page_messages() {
        assert_eq!(EngineCommand::Seek(42).to_json().as_deref(), Some(r#"{"ms":42,"type":"seek"}"#));
        assert_eq!(EngineCommand::Volume(3.0).to_json().as_deref(), Some(r#"{"type":"volume","value":1.0}"#));
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
    /// An invalid token is given on purpose: the SDK must load, ask for a token
    /// and report an error through the bridge.
    #[cfg(windows)]
    #[test]
    #[ignore = "starts WebView2 and downloads Spotify's SDK"]
    fn webview2_hosts_the_official_player() {
        let (tx, rx) = std::sync::mpsc::channel();
        let dir = std::env::temp_dir().join(format!("spotilite-webview-{}", std::process::id()));
        let engine = Engine::start(dir.clone(), 0.5, move |event| {
            let _ = tx.send(event);
        });
        let sender = engine.sender();
        let deadline = Instant::now() + Duration::from_secs(90);
        let (mut drm, mut asked_token, mut reported) = (None, false, None);
        while Instant::now() < deadline && reported.is_none() {
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(EngineEvent::Drm(systems)) => {
                    println!("DRM systems offered by WebView2: {systems:?}");
                    drm = Some(systems);
                }
                Ok(EngineEvent::NeedToken) => {
                    println!("the SDK asks for a token");
                    asked_token = true;
                    sender.send(EngineCommand::Token("invalid-token-for-test".into()));
                }
                Ok(EngineEvent::Error(kind, message)) => {
                    println!("SDK error {kind:?}: {message}");
                    reported = Some(kind);
                }
                Ok(EngineEvent::Failed(message)) => panic!("engine failed: {message}"),
                Ok(other) => println!("{other:?}"),
                Err(_) => {}
            }
        }
        // Memory of the WebView2 processes, for the README figures.
        let until = Instant::now() + Duration::from_secs(6);
        while let Some(left) = until.checked_duration_since(Instant::now()) {
            if let Ok(EngineEvent::Memory(bytes)) = rx.recv_timeout(left) {
                println!("WebView2 processes: {} MB", bytes / 1024 / 1024);
            }
        }
        drop(engine);
        std::thread::sleep(Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(dir);
        assert!(drm.is_some(), "the player page never loaded");
        assert!(reported.is_some(), "the SDK reported nothing (token asked: {asked_token})");
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
