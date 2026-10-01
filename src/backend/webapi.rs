//! Minimal Spotify Web API client.
//!
//! Data-saving choices:
//! * responses are requested gzip-compressed and every byte received is counted;
//! * `market=from_token` strips the huge `available_markets` arrays;
//! * results are immediately reduced to the compact structs of [`crate::model`].

use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use reqwest::Method;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::auth::{self, AppCredentials, OAuthToken};
use super::vault;
use crate::config::Paths;
use crate::model::{AlbumSummary, ArtistRef, ArtistSummary, PlaylistSummary, SearchResults, Track};

const API: &str = "https://api.spotify.com/v1";
/// Safety net against endless pagination.
const MAX_ITEMS: usize = 20_000;

#[derive(Debug)]
pub enum ApiError {
    /// No authorized Spotify application yet (setup screen).
    NotConnected,
    Network(String),
    RateLimited(u64),
    Forbidden(String),
    NotFound,
    Status(u16, String),
    Parse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::NotConnected => write!(f, "application Spotify non connectée"),
            ApiError::Network(e) => write!(f, "réseau indisponible ({e})"),
            ApiError::RateLimited(s) => {
                write!(f, "Spotify limite les requêtes, réessayez dans {s} s")
            }
            ApiError::Forbidden(e) => write!(f, "accès refusé par Spotify ({e})"),
            ApiError::NotFound => write!(f, "introuvable"),
            ApiError::Status(c, e) => write!(f, "erreur Spotify {c} ({e})"),
            ApiError::Parse(e) => write!(f, "réponse inattendue ({e})"),
        }
    }
}

impl ApiError {
    /// The authorization lacks a scope (it was granted before SpotiLite asked for it).
    pub fn is_missing_scope(&self) -> bool {
        match self {
            ApiError::Forbidden(m) | ApiError::Status(401 | 403, m) => {
                let m = m.to_lowercase();
                m.contains("scope") || m.contains("permissions missing")
            }
            _ => false,
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// The user's Spotify application and its current token.
struct Connection {
    app: AppCredentials,
    token: OAuthToken,
}

#[derive(serde::Serialize, Deserialize)]
struct StoredToken {
    client_id: String,
    refresh_token: String,
    /// Granted scopes (absent in files written by version 0.1).
    #[serde(default)]
    scope: String,
}

/// Every Web API call goes through the application the user created on
/// developer.spotify.com: its quota is not shared with anybody else.
pub struct WebApi {
    http: reqwest::Client,
    connection: tokio::sync::Mutex<Option<Connection>>,
    lost: AtomicBool,
    paths: Paths,
    bytes: Arc<AtomicU64>,
}

impl WebApi {
    pub fn new(http: reqwest::Client, paths: Paths, bytes: Arc<AtomicU64>) -> Self {
        Self { http, connection: tokio::sync::Mutex::new(None), lost: AtomicBool::new(false), paths, bytes }
    }

    /// Credentials saved by the setup screen, if any.
    pub fn saved_app(&self) -> Option<AppCredentials> {
        vault::load::<AppCredentials>(&self.paths.app_file()).filter(|a| !a.client_id.is_empty())
    }

    pub fn save_app(&self, app: &AppCredentials) {
        if let Err(e) = vault::save(&self.paths.app_file(), app) {
            log::warn!("could not save app credentials: {e}");
        }
    }

    /// Restores the authorization saved by a previous run.
    pub async fn restore(&self) -> bool {
        let mut connection = self.connection.lock().await;
        *connection = None;
        let Some(app) = self.saved_app() else { return false };
        let Some(stored) =
            vault::load::<StoredToken>(&self.paths.web_token_file()).filter(|s| s.client_id == app.client_id)
        else {
            return false;
        };
        let token = OAuthToken {
            access_token: String::new(),
            refresh_token: Some(stored.refresh_token),
            expires_at: Instant::now(),
            scope: stored.scope,
        };
        *connection = Some(Connection { app, token });
        true
    }

    pub async fn connect(&self, app: AppCredentials, token: OAuthToken) {
        self.save_token(&app, &token);
        self.lost.store(false, Ordering::Relaxed);
        *self.connection.lock().await = Some(Connection { app, token });
    }

    /// Forgets the authorization (and, with `forget_app`, the credentials too).
    pub async fn disconnect(&self, forget_app: bool) {
        *self.connection.lock().await = None;
        let _ = std::fs::remove_file(self.paths.web_token_file());
        if forget_app {
            let _ = std::fs::remove_file(self.paths.app_file());
        }
    }

    /// True once if the authorization stopped working (revoked, expired…).
    pub fn take_lost(&self) -> bool {
        self.lost.swap(false, Ordering::Relaxed)
    }

    fn save_token(&self, app: &AppCredentials, token: &OAuthToken) {
        let Some(refresh_token) = token.refresh_token.clone() else { return };
        let stored =
            StoredToken { client_id: app.client_id.clone(), refresh_token, scope: token.scope.clone() };
        if let Err(e) = vault::save(&self.paths.web_token_file(), &stored) {
            log::warn!("could not save web api token: {e}");
        }
    }

    async fn token(&self) -> ApiResult<String> {
        let mut guard = self.connection.lock().await;
        let Some(connection) = guard.as_mut() else { return Err(ApiError::NotConnected) };
        if Instant::now() < connection.token.expires_at {
            return Ok(connection.token.access_token.clone());
        }
        let refresh = connection.token.refresh_token.clone().unwrap_or_default();
        match auth::refresh(&self.http, &connection.app, &refresh).await {
            Ok(mut token) => {
                if token.scope.is_empty() {
                    token.scope = std::mem::take(&mut connection.token.scope);
                }
                self.save_token(&connection.app, &token);
                connection.token = token;
                Ok(connection.token.access_token.clone())
            }
            Err(auth::AuthError::Exchange(e)) if e.starts_with("400") || e.starts_with("401") => {
                // Refresh token revoked or expired (Spotify expires them after 6 months).
                log::warn!("web api authorization lost: {e}");
                *guard = None;
                let _ = std::fs::remove_file(self.paths.web_token_file());
                self.lost.store(true, Ordering::Relaxed);
                Err(ApiError::NotConnected)
            }
            Err(e) => Err(ApiError::Network(e.to_string())),
        }
    }

    /// A valid access token of the user's application (for the playback engine).
    pub async fn access_token(&self) -> ApiResult<String> {
        self.token().await
    }

    /// Scopes of `wanted` the current authorization lacks (refreshes the token
    /// first, which tells the granted scopes).
    pub async fn missing_scopes(&self, wanted: &[&'static str]) -> ApiResult<Vec<&'static str>> {
        self.token().await?;
        let guard = self.connection.lock().await;
        let connection = guard.as_ref().ok_or(ApiError::NotConnected)?;
        Ok(connection.token.missing_scopes(wanted))
    }

    async fn invalidate_token(&self) {
        if let Some(connection) = self.connection.lock().await.as_mut() {
            connection.token.expires_at = Instant::now();
        }
    }

    async fn request(&self, method: Method, url: &str) -> ApiResult<Vec<u8>> {
        self.request_with(method, url, None).await
    }

    async fn request_with(&self, method: Method, url: &str, json: Option<&Value>) -> ApiResult<Vec<u8>> {
        let mut auth_retried = false;
        let mut retried = false;
        loop {
            let token = self.token().await?;
            let mut req =
                self.http.request(method.clone(), url).bearer_auth(&token).header(ACCEPT_ENCODING, "gzip");
            if let Some(json) = json {
                req = req.header(CONTENT_TYPE, "application/json").body(json.to_string());
            } else if method != Method::GET {
                req = req.header(CONTENT_LENGTH, "0");
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if !retried => {
                    log::info!("request failed, retrying: {e}");
                    retried = true;
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    continue;
                }
                Err(e) => return Err(ApiError::Network(e.to_string())),
            };
            let status = resp.status();
            let gzip = resp
                .headers()
                .get(CONTENT_ENCODING)
                .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"gzip"));
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let raw = resp.bytes().await.map_err(|e| ApiError::Network(e.to_string()))?;
            self.bytes.fetch_add(raw.len() as u64, Ordering::Relaxed);
            let body = if gzip { gunzip(&raw)? } else { raw.to_vec() };
            match status.as_u16() {
                200..=299 => return Ok(body),
                401 if !auth_retried => {
                    auth_retried = true;
                    self.invalidate_token().await;
                }
                429 => {
                    let wait = retry_after.unwrap_or(5);
                    if wait <= 3 && !retried {
                        retried = true;
                        tokio::time::sleep(Duration::from_secs(wait.max(1))).await;
                    } else {
                        return Err(ApiError::RateLimited(wait));
                    }
                }
                403 => return Err(ApiError::Forbidden(error_message(&body))),
                404 => return Err(ApiError::NotFound),
                500..=599 if !retried => {
                    retried = true;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                code => return Err(ApiError::Status(code, error_message(&body))),
            }
        }
    }

    async fn get<T: DeserializeOwned>(&self, url: &str) -> ApiResult<T> {
        let body = self.request(Method::GET, url).await?;
        serde_json::from_slice(&body).map_err(|e| ApiError::Parse(e.to_string()))
    }

    /// Follows `next` links, converting each raw item with `convert`.
    async fn paged<T>(
        &self,
        first_url: String,
        mut convert: impl FnMut(Value) -> Option<T>,
    ) -> ApiResult<Vec<T>> {
        let mut out = Vec::new();
        let mut url = Some(first_url);
        while let Some(current) = url.take() {
            let page: Page = self.get(&current).await?;
            out.extend(page.items.into_iter().filter_map(&mut convert));
            if out.len() >= MAX_ITEMS {
                break;
            }
            url = page.next;
        }
        Ok(out)
    }

    pub async fn me(&self) -> ApiResult<String> {
        #[derive(Deserialize)]
        struct Me {
            id: String,
            display_name: Option<String>,
        }
        let me: Me = self.get(&format!("{API}/me")).await?;
        Ok(me.display_name.filter(|n| !n.is_empty()).unwrap_or(me.id))
    }

    pub async fn playlists(&self) -> ApiResult<Vec<PlaylistSummary>> {
        self.paged(format!("{API}/me/playlists?limit=50"), playlist_from).await
    }

    /// Tracks of a playlist. Development mode applications can only read playlists
    /// owned by the user (or shared with them): the caller then falls back to the streaming protocol.
    pub async fn playlist_tracks(&self, id: &str) -> ApiResult<Vec<Track>> {
        let url = format!("{API}/playlists/{id}/items?limit=50&market=from_token&additional_types=track");
        self.paged(url, |item| {
            let mut item: PlaylistItem = serde_json::from_value(item).ok()?;
            if item.is_local {
                return None;
            }
            let track = item.item.take().or(item.track.take())?;
            track_from(track, None)
        })
        .await
    }

    /// Liked songs. With a cached copy, only the first page is downloaded when
    /// nothing changed (the list is sorted by date added, newest first).
    pub async fn liked_tracks(&self, cached: Option<&[Track]>) -> ApiResult<Vec<Track>> {
        let first = format!("{API}/me/tracks?limit=50&market=from_token");
        let page: Page = self.get(&first).await?;
        let fresh: Vec<Track> =
            page.items.iter().filter_map(|v| track_from(v.get("track")?.clone(), None)).collect();
        if let Some(cached) = cached
            && let Some(first_cached) = cached.first()
            && let Some(k) = fresh.iter().position(|t| t.id == first_cached.id)
            && k + cached.len() == page.total as usize
        {
            let mut merged = fresh[..k].to_vec();
            merged.extend_from_slice(cached);
            return Ok(merged);
        }
        let mut all = fresh;
        if let Some(next) = page.next {
            let rest = self.paged(next, |v| track_from(v.get("track")?.clone(), None)).await?;
            all.extend(rest);
        }
        Ok(all)
    }

    pub async fn saved_albums(&self) -> ApiResult<Vec<AlbumSummary>> {
        self.paged(format!("{API}/me/albums?limit=50&market=from_token"), |v| {
            album_from(v.get("album")?.clone())
        })
        .await
    }

    pub async fn album(&self, id: &str) -> ApiResult<(AlbumSummary, Vec<Track>)> {
        let mut value: Value = self.get(&format!("{API}/albums/{id}?market=from_token")).await?;
        let tracks_page = value.get_mut("tracks").map(Value::take).unwrap_or(Value::Null);
        let summary = album_from(value.clone()).ok_or_else(|| ApiError::Parse("album".into()))?;
        let album_ref: AlbumRef =
            serde_json::from_value(value).map_err(|e| ApiError::Parse(e.to_string()))?;
        let page: Page = serde_json::from_value(tracks_page).unwrap_or_default();
        let mut tracks: Vec<Track> =
            page.items.into_iter().filter_map(|v| track_from(v, Some(&album_ref))).collect();
        if let Some(next) = page.next {
            let rest = self.paged(next, |v| track_from(v, Some(&album_ref))).await?;
            tracks.extend(rest);
        }
        Ok((summary, tracks))
    }

    pub async fn artist(&self, id: &str) -> ApiResult<(String, Vec<Track>, Vec<AlbumSummary>)> {
        #[derive(Deserialize)]
        struct Artist {
            name: String,
        }
        #[derive(Deserialize)]
        struct TopTracks {
            #[serde(default)]
            tracks: Vec<Value>,
        }
        let artist: Artist = self.get(&format!("{API}/artists/{id}")).await?;
        // Not available to development mode applications: optional.
        let top =
            match self.get::<TopTracks>(&format!("{API}/artists/{id}/top-tracks?market=from_token")).await {
                Ok(top) => top.tracks.into_iter().filter_map(|v| track_from(v, None)).collect(),
                Err(e) => {
                    log::info!("top tracks unavailable: {e}");
                    Vec::new()
                }
            };
        let albums = self
            .paged(
                format!("{API}/artists/{id}/albums?include_groups=album,single&limit=50&market=from_token"),
                album_from,
            )
            .await?;
        Ok((artist.name, top, albums))
    }

    pub async fn search(&self, query: &str) -> ApiResult<SearchResults> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Response {
            tracks: Page,
            albums: Page,
            artists: Page,
            playlists: Page,
        }
        let url = reqwest::Url::parse_with_params(
            &format!("{API}/search"),
            &[
                ("q", query),
                ("type", "track,album,artist,playlist"),
                // 10 is the maximum allowed to development mode applications.
                ("limit", "10"),
                ("market", "from_token"),
            ],
        )
        .map_err(|e| ApiError::Parse(e.to_string()))?;
        let r: Response = self.get(url.as_str()).await?;
        Ok(SearchResults {
            query: query.to_string(),
            tracks: Arc::new(r.tracks.items.into_iter().filter_map(|v| track_from(v, None)).collect()),
            albums: r.albums.items.into_iter().filter_map(album_from).collect(),
            artists: r
                .artists
                .items
                .into_iter()
                .filter_map(|v| serde_json::from_value::<ArtistJson>(v).ok())
                .filter_map(|a| Some(ArtistSummary { id: a.id?, name: a.name }))
                .collect(),
            playlists: r.playlists.items.into_iter().filter_map(playlist_from).collect(),
        })
    }

    pub async fn is_liked(&self, track_id: &str) -> ApiResult<bool> {
        let uri = format!("spotify:track:{track_id}");
        let result: ApiResult<Vec<bool>> =
            match self.get(&format!("{API}/me/library/contains?uris={uri}")).await {
                Err(ApiError::NotFound | ApiError::Forbidden(_)) => {
                    self.get(&format!("{API}/me/tracks/contains?ids={track_id}")).await
                }
                other => other,
            };
        Ok(result?.first().copied().unwrap_or(false))
    }

    pub async fn set_liked(&self, track_id: &str, liked: bool) -> ApiResult<()> {
        let method = if liked { Method::PUT } else { Method::DELETE };
        let uri = format!("spotify:track:{track_id}");
        match self.request(method.clone(), &format!("{API}/me/library?uris={uri}")).await {
            Err(ApiError::NotFound | ApiError::Forbidden(_) | ApiError::Status(405, _)) => {
                self.request(method, &format!("{API}/me/tracks?ids={track_id}")).await.map(|_| ())
            }
            other => other.map(|_| ()),
        }
    }

    /// Downloads a cover image (public CDN, no token needed).
    /// Starts playback on a Spotify device (the playback engine).
    pub async fn play_on_device(&self, device_id: &str, what: &PlayRequest) -> ApiResult<()> {
        let body = match what {
            PlayRequest::Track { id, position_ms } => {
                serde_json::json!({ "uris": [format!("spotify:track:{id}")], "position_ms": position_ms })
            }
            PlayRequest::Context { uri, offset, position_ms } => {
                let mut body = serde_json::json!({ "context_uri": uri });
                match offset {
                    Some(Offset::Position(n)) => body["offset"] = serde_json::json!({ "position": n }),
                    Some(Offset::Track(id)) => {
                        body["offset"] = serde_json::json!({ "uri": format!("spotify:track:{id}") });
                    }
                    None => {}
                }
                if *position_ms > 0 {
                    body["position_ms"] = serde_json::json!(position_ms);
                }
                body
            }
        };
        let url = player_url("play", device_id, &[])?;
        self.request_with(Method::PUT, &url, Some(&body)).await.map(drop)
    }

    /// Shuffle of what Spotify plays itself (a playlist or an artist).
    pub async fn set_shuffle(&self, device_id: &str, on: bool) -> ApiResult<()> {
        let url = player_url("shuffle", device_id, &[("state", if on { "true" } else { "false" })])?;
        self.request(Method::PUT, &url).await.map(drop)
    }

    /// `state`: "off", "context" or "track".
    pub async fn set_repeat(&self, device_id: &str, state: &str) -> ApiResult<()> {
        let url = player_url("repeat", device_id, &[("state", state)])?;
        self.request(Method::PUT, &url).await.map(drop)
    }

    /// Adds a track to Spotify's own queue (while it plays a playlist or an artist).
    pub async fn add_to_queue(&self, device_id: &str, track_id: &str) -> ApiResult<()> {
        let uri = format!("spotify:track:{track_id}");
        let url = player_url("queue", device_id, &[("uri", uri.as_str())])?;
        self.request(Method::POST, &url).await.map(drop)
    }

    pub async fn download(&self, url: &str) -> ApiResult<Vec<u8>> {
        let resp = self.http.get(url).send().await.map_err(|e| ApiError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ApiError::Status(resp.status().as_u16(), String::new()));
        }
        let bytes = resp.bytes().await.map_err(|e| ApiError::Network(e.to_string()))?;
        self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        Ok(bytes.to_vec())
    }
}

/// What to start on the playback engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayRequest {
    /// One track, chosen by SpotiLite's own queue.
    Track { id: String, position_ms: u32 },
    /// A playlist or an artist that Spotify plays itself (development mode
    /// applications cannot read playlists of other users nor top tracks).
    Context { uri: String, offset: Option<Offset>, position_ms: u32 },
}

/// Where to start in a context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offset {
    Position(u32),
    Track(String),
}

fn player_url(action: &str, device_id: &str, params: &[(&str, &str)]) -> ApiResult<String> {
    let mut query = vec![("device_id", device_id)];
    query.extend_from_slice(params);
    reqwest::Url::parse_with_params(&format!("{API}/me/player/{action}"), &query)
        .map(String::from)
        .map_err(|e| ApiError::Parse(e.to_string()))
}

fn gunzip(raw: &[u8]) -> ApiResult<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len() * 6);
    flate2::read::GzDecoder::new(raw)
        .read_to_end(&mut out)
        .map_err(|e| ApiError::Parse(format!("gzip: {e}")))?;
    Ok(out)
}

fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/error_description"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned())
}

// ---------------------------------------------------------------------------
// JSON shapes (only the fields we use). Items are parsed one by one so a single
// unexpected entry (null, podcast episode, local file…) never breaks a whole list.

#[derive(Deserialize, Default)]
#[serde(default)]
struct Page {
    items: Vec<Value>,
    next: Option<String>,
    total: u32,
}

#[derive(Deserialize)]
struct PlaylistItem {
    #[serde(default)]
    item: Option<Value>,
    #[serde(default)]
    track: Option<Value>,
    #[serde(default)]
    is_local: bool,
}

#[derive(Deserialize)]
struct ImageJson {
    url: String,
    #[serde(default)]
    width: Option<u32>,
}

#[derive(Deserialize)]
struct ArtistJson {
    id: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct AlbumRef {
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    images: Vec<ImageJson>,
}

#[derive(Deserialize)]
struct TrackJson {
    id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    artists: Vec<ArtistJson>,
    album: Option<AlbumRef>,
    #[serde(default)]
    duration_ms: u32,
    is_playable: Option<bool>,
    #[serde(default)]
    is_local: bool,
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// Picks the smallest image that still looks sharp in a 48–64 px slot.
fn small_image(images: &[ImageJson]) -> Option<String> {
    images
        .iter()
        .filter(|i| i.width.unwrap_or(0) >= 60)
        .min_by_key(|i| i.width.unwrap_or(u32::MAX))
        .or_else(|| images.last())
        .map(|i| i.url.clone())
}

fn track_from(value: Value, album: Option<&AlbumRef>) -> Option<Track> {
    let t: TrackJson = serde_json::from_value(value).ok()?;
    if t.kind.as_deref().is_some_and(|k| k != "track") {
        return None;
    }
    let album = t.album.as_ref().or(album);
    Some(Track {
        id: t.id?,
        name: t.name,
        artists: t
            .artists
            .into_iter()
            .map(|a| ArtistRef { id: a.id.unwrap_or_default(), name: a.name })
            .collect(),
        album: album.map(|a| a.name.clone()).unwrap_or_default(),
        album_id: album.and_then(|a| a.id.clone()).unwrap_or_default(),
        duration_ms: t.duration_ms,
        image: album.and_then(|a| small_image(&a.images)),
        playable: t.is_playable.unwrap_or(true) && !t.is_local,
    })
}

fn album_from(value: Value) -> Option<AlbumSummary> {
    #[derive(Deserialize)]
    struct AlbumJson {
        id: Option<String>,
        #[serde(default)]
        name: String,
        #[serde(default)]
        artists: Vec<ArtistJson>,
        #[serde(default)]
        images: Vec<ImageJson>,
        #[serde(default)]
        release_date: String,
        #[serde(default)]
        total_tracks: u32,
    }
    let a: AlbumJson = serde_json::from_value(value).ok()?;
    Some(AlbumSummary {
        id: a.id?,
        name: a.name,
        artists: crate::model::join_names(a.artists.iter().map(|a| a.name.as_str())),
        year: a.release_date.chars().take(4).collect(),
        total_tracks: a.total_tracks,
        image: small_image(&a.images),
    })
}

fn playlist_from(value: Value) -> Option<PlaylistSummary> {
    #[derive(Deserialize)]
    struct Owner {
        #[serde(default)]
        display_name: Option<String>,
        #[serde(default)]
        id: String,
    }
    #[derive(Deserialize)]
    struct PlaylistJson {
        id: Option<String>,
        #[serde(default)]
        name: String,
        owner: Option<Owner>,
        // `tracks` was renamed `items` in 2026; accept both.
        #[serde(default)]
        tracks: Option<Value>,
        #[serde(default)]
        items: Option<Value>,
        #[serde(default)]
        snapshot_id: Option<String>,
    }
    let p: PlaylistJson = serde_json::from_value(value).ok()?;
    let total = p
        .items
        .as_ref()
        .or(p.tracks.as_ref())
        .and_then(|v| v.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    Some(PlaylistSummary {
        id: p.id?,
        name: p.name,
        owner: p.owner.map(|o| o.display_name.filter(|n| !n.is_empty()).unwrap_or(o.id)).unwrap_or_default(),
        total,
        snapshot_id: p.snapshot_id.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn player_urls_carry_the_device() {
        let url = player_url("queue", "dev 1", &[("uri", "spotify:track:abc")]).unwrap();
        assert_eq!(
            url,
            "https://api.spotify.com/v1/me/player/queue?device_id=dev+1&uri=spotify%3Atrack%3Aabc"
        );
    }

    #[tokio::test]
    async fn saves_restores_and_forgets_the_application() {
        let dir = std::env::temp_dir().join(format!("spotilite-webapi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths { config: dir.clone(), cache: dir.join("cache") };
        let api = WebApi::new(reqwest::Client::new(), paths, Arc::new(AtomicU64::new(0)));
        assert!(!api.restore().await);
        assert!(matches!(api.me().await, Err(ApiError::NotConnected)));

        let app = AppCredentials { client_id: "a".repeat(32), client_secret: "b".repeat(32) };
        api.save_app(&app);
        assert_eq!(api.saved_app(), Some(app.clone()));
        assert!(!api.restore().await, "no authorization yet");

        let token = OAuthToken {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: Instant::now() + Duration::from_secs(600),
            scope: "user-read-private streaming".into(),
        };
        api.connect(app.clone(), token).await;
        assert_eq!(
            api.missing_scopes(auth::PLAYBACK_SCOPES).await.unwrap(),
            vec!["user-read-email", "user-modify-playback-state"]
        );
        assert!(api.restore().await, "authorization survives a restart");
        assert_eq!(api.connection.lock().await.as_ref().unwrap().token.scope, "user-read-private streaming");

        api.disconnect(false).await;
        assert!(!api.restore().await);
        assert_eq!(api.saved_app(), Some(app), "credentials are kept");
        api.disconnect(true).await;
        assert_eq!(api.saved_app(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn track_json(id: &str) -> Value {
        json!({
            "id": id, "name": format!("Song {id}"), "type": "track", "duration_ms": 1000,
            "is_playable": true,
            "artists": [{"id": "a1", "name": "Artist"}],
            "album": {"id": "al", "name": "Album", "images": [
                {"url": "big", "width": 640}, {"url": "mid", "width": 300}, {"url": "small", "width": 64}
            ]}
        })
    }

    #[test]
    fn parses_track_and_picks_small_cover() {
        let t = track_from(track_json("x"), None).unwrap();
        assert_eq!(t.id, "x");
        assert_eq!(t.album, "Album");
        assert_eq!(t.image.as_deref(), Some("small"));
        assert!(t.playable);
    }

    #[test]
    fn skips_episodes_local_files_and_nulls() {
        assert!(track_from(json!({"id": "e", "name": "Ep", "type": "episode"}), None).is_none());
        assert!(track_from(Value::Null, None).is_none());
        let local = json!({"id": null, "name": "Local", "type": "track", "is_local": true});
        assert!(track_from(local, None).is_none());
    }

    #[test]
    fn playlist_items_accept_item_and_legacy_track_keys() {
        for key in ["item", "track"] {
            let entry =
                json!({ "added_at": "2026-01-01T00:00:00Z", "is_local": false, key: track_json("p") });
            let mut item: PlaylistItem = serde_json::from_value(entry).unwrap();
            let track = item.item.take().or(item.track.take()).unwrap();
            assert_eq!(track_from(track, None).unwrap().id, "p");
        }
    }

    #[test]
    fn playlist_summary_accepts_items_or_tracks_total() {
        let new = json!({"id": "1", "name": "N", "owner": {"id": "me", "display_name": ""},
                          "items": {"total": 12}, "snapshot_id": "s"});
        let old = json!({"id": "2", "name": "O", "owner": {"id": "me", "display_name": "Moi"},
                          "tracks": {"total": 3}});
        let n = playlist_from(new).unwrap();
        assert_eq!((n.total, n.owner.as_str(), n.snapshot_id.as_str()), (12, "me", "s"));
        let o = playlist_from(old).unwrap();
        assert_eq!((o.total, o.owner.as_str()), (3, "Moi"));
    }

    #[test]
    fn album_tracks_inherit_album_info() {
        let album: AlbumRef = serde_json::from_value(json!({
            "id": "al", "name": "Disque", "images": [{"url": "u", "width": 64}]
        }))
        .unwrap();
        let simplified = json!({"id": "t", "name": "Piste", "type": "track", "duration_ms": 5});
        let t = track_from(simplified, Some(&album)).unwrap();
        assert_eq!((t.album.as_str(), t.album_id.as_str()), ("Disque", "al"));
        assert_eq!(t.image.as_deref(), Some("u"));
    }

    #[test]
    fn gunzip_roundtrip() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(b"{\"ok\":true}").unwrap();
        let gz = enc.finish().unwrap();
        assert_eq!(gunzip(&gz).unwrap(), b"{\"ok\":true}");
    }

    #[test]
    fn extracts_error_message() {
        assert_eq!(error_message(br#"{"error":{"status":403,"message":"Forbidden"}}"#), "Forbidden");
        assert_eq!(error_message(b"plain"), "plain");
    }
}
