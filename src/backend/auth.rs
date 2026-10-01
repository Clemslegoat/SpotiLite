//! OAuth 2.0 authorization code flow, completed in the user's browser.
//!
//! No password ever goes through SpotiLite: Spotify's own login page is opened, and
//! a tiny local HTTP listener on 127.0.0.1 receives the authorization code. With a
//! client secret the classic flow is used (HTTP Basic authentication of the app);
//! without one, PKCE replaces the secret.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

/// Scopes requested for the user's own Spotify application: library and search,
/// plus what the playback engine (Web Playback SDK) needs.
pub const WEB_API_SCOPES: &[&str] = &[
    "user-read-private",
    "playlist-read-private",
    "playlist-read-collaborative",
    "user-library-read",
    "user-library-modify",
    "streaming",
    "user-read-email",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-follow-read",
];

/// Scopes added since version 0.1 (playback, followed artists): an older
/// authorization lacking them is renewed once.
pub const REQUIRED_SCOPES: &[&str] =
    &["streaming", "user-read-email", "user-read-private", "user-modify-playback-state", "user-follow-read"];

const AUTHORIZE_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub enum AuthError {
    Bind(String),
    Cancelled,
    TimedOut,
    Denied(String),
    Exchange(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Bind(e) => write!(f, "impossible d'écouter le retour de connexion ({e})"),
            AuthError::Cancelled => write!(f, "connexion annulée"),
            AuthError::TimedOut => write!(f, "délai de connexion dépassé"),
            AuthError::Denied(e) => write!(f, "autorisation refusée ({e})"),
            AuthError::Exchange(e) => write!(f, "échange du jeton impossible ({e})"),
        }
    }
}

/// The application created by the user on developer.spotify.com.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCredentials {
    pub client_id: String,
    /// May be empty: the PKCE flow is then used.
    #[serde(default)]
    pub client_secret: String,
}

impl AppCredentials {
    fn secret(&self) -> Option<&str> {
        Some(self.client_secret.as_str()).filter(|s| !s.is_empty())
    }

    /// Spotify ids and secrets are 32 hexadecimal characters.
    pub fn looks_valid(value: &str) -> bool {
        value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit())
    }
}

#[derive(Clone, Debug)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Instant,
    /// Granted scopes, space separated (empty when unknown).
    pub scope: String,
}

impl OAuthToken {
    /// Scopes of `wanted` that were not granted (none when the grant is unknown).
    pub fn missing_scopes<'a>(&self, wanted: &[&'a str]) -> Vec<&'a str> {
        if self.scope.trim().is_empty() {
            return Vec::new();
        }
        let granted: Vec<&str> = self.scope.split_whitespace().collect();
        wanted.iter().copied().filter(|s| !granted.contains(s)).collect()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default = "default_expiry")]
    expires_in: u64,
}

fn default_expiry() -> u64 {
    3600
}

/// A pending browser authorization.
pub struct AuthFlow {
    app: AppCredentials,
    pub redirect_uri: String,
    pub auth_url: String,
    verifier: String,
    state: String,
    listener: TcpListener,
}

fn random_string(len: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::rng();
    (0..len).map(|_| CHARS[rng.random_range(0..CHARS.len())] as char).collect()
}

pub fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

impl AuthFlow {
    /// Prepares the flow on the redirect URI registered by the application (`port = 0`
    /// picks a free port, for tests).
    pub fn start(app: AppCredentials, port: u16, scopes: &[&str]) -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| AuthError::Bind(format!("127.0.0.1:{port} : {e}")))?;
        let port = listener.local_addr().map_err(|e| AuthError::Bind(e.to_string()))?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/login");
        let verifier = random_string(64);
        let state = random_string(16);
        let challenge = code_challenge(&verifier);
        let scope = scopes.join(" ");
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", app.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state.as_str()),
            ("scope", scope.as_str()),
        ];
        if app.secret().is_none() {
            params.push(("code_challenge_method", "S256"));
            params.push(("code_challenge", challenge.as_str()));
        }
        let auth_url = Url::parse_with_params(AUTHORIZE_URL, &params)
            .map_err(|e| AuthError::Exchange(e.to_string()))?
            .to_string();
        Ok(Self { app, redirect_uri, auth_url, verifier, state, listener })
    }

    /// Blocks until the browser comes back (run it on a blocking thread).
    pub fn wait_for_code(&self, cancel: &Arc<AtomicBool>) -> Result<String, AuthError> {
        self.listener.set_nonblocking(true).map_err(|e| AuthError::Bind(e.to_string()))?;
        let deadline = Instant::now() + LOGIN_TIMEOUT;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AuthError::Cancelled);
            }
            if Instant::now() > deadline {
                return Err(AuthError::TimedOut);
            }
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(result) = self.handle_connection(stream) {
                        return result;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => return Err(AuthError::Bind(e.to_string())),
            }
        }
    }

    /// Returns `None` for unrelated requests (favicon, browser pre-connections…).
    fn handle_connection(&self, mut stream: TcpStream) -> Option<Result<String, AuthError>> {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let target = line.split_whitespace().nth(1)?;
        let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
        if url.path() != "/login" {
            let _ =
                stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
            return None;
        }
        let param = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
        let result = if let Some(error) = param("error") {
            Err(AuthError::Denied(error))
        } else if param("state").as_deref() != Some(self.state.as_str()) {
            Err(AuthError::Denied("state invalide".into()))
        } else if let Some(code) = param("code") {
            Ok(code)
        } else {
            Err(AuthError::Denied("code absent".into()))
        };
        let (title, text) = match &result {
            Ok(_) => ("Connecté", "Vous pouvez fermer cet onglet et revenir à SpotiLite."),
            Err(_) => ("Échec de la connexion", "Revenez à SpotiLite pour réessayer."),
        };
        let page = callback_page(title, text);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(response.as_bytes());
        Some(result)
    }

    pub async fn exchange(&self, http: &reqwest::Client, code: &str) -> Result<OAuthToken, AuthError> {
        // Built in its own scope: the serializer is not `Send` and must not live
        // across the `.await`.
        let body = {
            let mut body = url::form_urlencoded::Serializer::new(String::new());
            body.append_pair("grant_type", "authorization_code")
                .append_pair("code", code)
                .append_pair("redirect_uri", &self.redirect_uri);
            if self.app.secret().is_none() {
                body.append_pair("client_id", &self.app.client_id)
                    .append_pair("code_verifier", &self.verifier);
            }
            body.finish()
        };
        post_token(http, &self.app, body, None).await
    }
}

/// Gets a fresh access token. Spotify no longer always returns a new refresh token,
/// in which case the previous one stays valid and is kept.
pub async fn refresh(
    http: &reqwest::Client,
    app: &AppCredentials,
    refresh_token: &str,
) -> Result<OAuthToken, AuthError> {
    let body = {
        let mut body = url::form_urlencoded::Serializer::new(String::new());
        body.append_pair("grant_type", "refresh_token").append_pair("refresh_token", refresh_token);
        if app.secret().is_none() {
            body.append_pair("client_id", &app.client_id);
        }
        body.finish()
    };
    post_token(http, app, body, Some(refresh_token)).await
}

/// `Authorization: Basic base64(client_id:client_secret)`, as documented by Spotify.
fn basic_auth(app: &AppCredentials) -> Option<String> {
    let secret = app.secret()?;
    let raw = format!("{}:{secret}", app.client_id);
    Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw)))
}

async fn post_token(
    http: &reqwest::Client,
    app: &AppCredentials,
    body: String,
    previous_refresh: Option<&str>,
) -> Result<OAuthToken, AuthError> {
    let mut request = http
        .post(TOKEN_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body);
    if let Some(header) = basic_auth(app) {
        request = request.header(reqwest::header::AUTHORIZATION, header);
    }
    let resp = request.send().await.map_err(|e| AuthError::Exchange(e.to_string()))?;
    let status = resp.status();
    let bytes = resp.bytes().await.map_err(|e| AuthError::Exchange(e.to_string()))?;
    if !status.is_success() {
        let detail = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| {
                v.get("error_description")
                    .or_else(|| v.get("error"))
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).into_owned());
        return Err(AuthError::Exchange(format!("{} : {detail}", status.as_u16())));
    }
    let parsed: TokenResponse =
        serde_json::from_slice(&bytes).map_err(|e| AuthError::Exchange(e.to_string()))?;
    Ok(OAuthToken {
        access_token: parsed.access_token,
        refresh_token: parsed
            .refresh_token
            .filter(|t| !t.is_empty())
            .or_else(|| previous_refresh.map(str::to_string)),
        expires_at: Instant::now() + Duration::from_secs(parsed.expires_in.saturating_sub(60)),
        scope: parsed.scope.unwrap_or_default(),
    })
}

fn callback_page(title: &str, text: &str) -> String {
    format!(
        "<!doctype html><html lang=\"fr\"><head><meta charset=\"utf-8\"><title>SpotiLite</title>\
<style>body{{margin:0;height:100vh;display:grid;place-items:center;background:#000;color:#fff;\
font:16px 'Segoe UI',system-ui,sans-serif}}h1{{font-weight:600;font-size:22px;margin:0 0 8px}}\
p{{margin:0;color:#8a8a8a}}</style></head><body><div><h1>{title}</h1><p>{text}</p></div></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        // Appendix B of RFC 7636.
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    fn app(id: &str, secret: &str) -> AppCredentials {
        AppCredentials { client_id: id.into(), client_secret: secret.into() }
    }

    #[test]
    fn auth_url_contains_pkce_parameters_without_secret() {
        let flow = AuthFlow::start(app("abc", ""), 0, &["streaming", "user-read-private"]).unwrap();
        let url = Url::parse(&flow.auth_url).unwrap();
        let get = |k: &str| url.query_pairs().find(|(q, _)| q == k).map(|(_, v)| v.into_owned());
        assert_eq!(get("client_id").as_deref(), Some("abc"));
        assert_eq!(get("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(get("scope").as_deref(), Some("streaming user-read-private"));
        assert!(get("redirect_uri").unwrap().starts_with("http://127.0.0.1:"));
        assert_eq!(flow.verifier.len(), 64);
    }

    #[test]
    fn secret_uses_basic_auth_and_no_pkce() {
        let flow = AuthFlow::start(app("abc", "s3cr3t"), 0, &["user-read-private"]).unwrap();
        let url = Url::parse(&flow.auth_url).unwrap();
        assert!(url.query_pairs().all(|(k, _)| k != "code_challenge"));
        // base64("abc:s3cr3t")
        assert_eq!(basic_auth(&app("abc", "s3cr3t")).as_deref(), Some("Basic YWJjOnMzY3IzdA=="));
        assert_eq!(basic_auth(&app("abc", "")), None);
    }

    #[test]
    fn reports_missing_scopes() {
        let token = |scope: &str| OAuthToken {
            access_token: String::new(),
            refresh_token: None,
            expires_at: Instant::now(),
            scope: scope.into(),
        };
        assert_eq!(
            token("user-read-private user-library-read").missing_scopes(REQUIRED_SCOPES),
            vec!["streaming", "user-read-email", "user-modify-playback-state", "user-follow-read"]
        );
        assert!(token(&WEB_API_SCOPES.join(" ")).missing_scopes(REQUIRED_SCOPES).is_empty());
        // Unknown grant: assume it is fine and let Spotify say otherwise.
        assert!(token("").missing_scopes(REQUIRED_SCOPES).is_empty());
    }

    #[test]
    fn validates_credential_format() {
        assert!(AppCredentials::looks_valid("0123456789abcdef0123456789ABCDEF"));
        assert!(!AppCredentials::looks_valid("0123456789abcdef"));
        assert!(!AppCredentials::looks_valid("0123456789abcdef0123456789abcdeg"));
    }

    #[test]
    fn callback_listener_ignores_noise_and_returns_code() {
        let flow = AuthFlow::start(app("abc", ""), 0, &["streaming"]).unwrap();
        let port = flow.listener.local_addr().unwrap().port();
        let state = flow.state.clone();
        let client = std::thread::spawn(move || {
            let mut favicon = TcpStream::connect(("127.0.0.1", port)).unwrap();
            favicon.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut ok = TcpStream::connect(("127.0.0.1", port)).unwrap();
            ok.write_all(format!("GET /login?code=xyz&state={state} HTTP/1.1\r\n\r\n").as_bytes()).unwrap();
            let mut response = String::new();
            let _ = std::io::Read::read_to_string(&mut ok, &mut response);
            response
        });
        let code = flow.wait_for_code(&Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(code, "xyz");
        assert!(client.join().unwrap().contains("Connecté"));
    }
}
