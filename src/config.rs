//! User settings and on-disk locations.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Repeat;

/// Streaming quality. Lower bitrates use proportionally less data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    /// 96 kbit/s Ogg Vorbis, about 43 MB per hour.
    #[default]
    Eco,
    /// 160 kbit/s, about 72 MB per hour.
    Normal,
    /// 320 kbit/s, about 144 MB per hour.
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Eco, Quality::Normal, Quality::High];

    pub fn kbps(self) -> u32 {
        match self {
            Quality::Eco => 96,
            Quality::Normal => 160,
            Quality::High => 320,
        }
    }

    /// Approximate data used per hour of listening, in megabytes.
    pub fn mb_per_hour(self) -> u32 {
        self.kbps() * 3600 / 8 / 1000
    }

    pub fn label(self) -> &'static str {
        match self {
            Quality::Eco => "Éco",
            Quality::Normal => "Normale",
            Quality::High => "Haute",
        }
    }
}

/// What plays the audio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Engine {
    /// librespot inside SpotiLite: lightest, bitrate choice, audio cache.
    #[default]
    Native,
    /// Spotify's own Web Playback SDK in an invisible WebView2 (PlayReady DRM):
    /// plays the tracks whose keys Spotify refuses to librespot, uses more memory.
    Official,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub engine: Engine,
    pub quality: Quality,
    /// Download album covers (64 px thumbnails, cached on disk).
    pub show_covers: bool,
    /// Audio cache size in MB. Replaying a cached track costs no data. 0 disables it.
    pub audio_cache_mb: u64,
    pub volume: f32,
    pub normalisation: bool,
    pub ui_scale: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Client id entered in version 0.1 (read once to migrate it, never written back).
    #[serde(rename = "client_id", skip_serializing)]
    pub legacy_client_id: String,
    /// Port of the redirect URI registered in the user's Spotify application.
    pub redirect_port: u16,
    /// Release unused memory pages when the window is minimized (Windows only).
    pub trim_when_minimized: bool,
    pub window_size: [f32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            engine: Engine::Native,
            quality: Quality::Eco,
            show_covers: true,
            audio_cache_mb: 1024,
            volume: 0.7,
            normalisation: true,
            ui_scale: 1.0,
            shuffle: false,
            repeat: Repeat::Off,
            legacy_client_id: String::new(),
            redirect_port: 8898,
            trim_when_minimized: true,
            window_size: [980.0, 640.0],
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Self {
        fs::read(paths.settings_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map(Self::sanitized)
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &Paths) {
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&paths.settings_file(), &bytes) {
                    log::warn!("could not save settings: {e}");
                }
            }
            Err(e) => log::warn!("could not serialize settings: {e}"),
        }
    }

    fn sanitized(mut self) -> Self {
        self.volume = self.volume.clamp(0.0, 1.0);
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self.legacy_client_id = self.legacy_client_id.trim().to_string();
        if self.redirect_port == 0 {
            self.redirect_port = 8898;
        }
        self.window_size[0] = self.window_size[0].clamp(480.0, 8000.0);
        self.window_size[1] = self.window_size[1].clamp(320.0, 8000.0);
        self
    }
}

/// Where SpotiLite keeps its files.
///
/// * Windows: `%APPDATA%\SpotiLite` (settings, credentials) and
///   `%LOCALAPPDATA%\SpotiLite` (caches).
/// * Portable mode: if a `spotilite-data` folder exists next to the executable,
///   everything goes there.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn detect() -> Self {
        let paths = Self::locate();
        for dir in [&paths.config, &paths.cache] {
            if let Err(e) = fs::create_dir_all(dir) {
                log::warn!("could not create {}: {e}", dir.display());
            }
        }
        paths
    }

    fn locate() -> Self {
        if let Some(dir) = std::env::var_os("SPOTILITE_HOME") {
            let dir = PathBuf::from(dir);
            return Self { config: dir.clone(), cache: dir.join("cache") };
        }
        if let Some(portable) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.join("spotilite-data")))
            .filter(|p| p.is_dir())
        {
            return Self { config: portable.clone(), cache: portable.join("cache") };
        }
        let env_dir = |key: &str| std::env::var_os(key).map(PathBuf::from);
        let home = env_dir("HOME").or_else(|| env_dir("USERPROFILE")).unwrap_or_else(std::env::temp_dir);
        if cfg!(windows) {
            let roaming = env_dir("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"));
            let local = env_dir("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
            Self { config: roaming.join("SpotiLite"), cache: local.join("SpotiLite") }
        } else {
            let config = env_dir("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
            let cache = env_dir("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
            Self { config: config.join("spotilite"), cache: cache.join("spotilite") }
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.json")
    }

    /// Client id and secret of the user's Spotify application (encrypted on Windows).
    pub fn app_file(&self) -> PathBuf {
        self.config.join("spotify-app.dat")
    }

    pub fn web_token_file(&self) -> PathBuf {
        self.config.join("webapi-token.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.cache.join("spotilite.log")
    }

    pub fn audio_cache(&self) -> PathBuf {
        self.cache.join("audio")
    }

    pub fn image_cache(&self) -> PathBuf {
        self.cache.join("covers")
    }

    pub fn data_cache(&self) -> PathBuf {
        self.cache.join("library")
    }

    pub fn tmp(&self) -> PathBuf {
        self.cache.join("tmp")
    }

    /// Profile of the official engine (WebView2).
    pub fn webview(&self) -> PathBuf {
        self.cache.join("webview")
    }
}

/// Writes through a temporary file so a crash never leaves a truncated file behind.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_estimates() {
        assert_eq!(Quality::Eco.mb_per_hour(), 43);
        assert_eq!(Quality::Normal.mb_per_hour(), 72);
        assert_eq!(Quality::High.mb_per_hour(), 144);
    }

    #[test]
    fn partial_settings_file_keeps_defaults() {
        let s: Settings = serde_json::from_str(r#"{"quality":"High","volume":3.0}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.quality, Quality::High);
        assert_eq!(s.volume, 1.0);
        assert!(s.show_covers);
        assert_eq!(s.redirect_port, 8898);
    }

    #[test]
    fn legacy_client_id_is_read_but_never_written() {
        let s: Settings = serde_json::from_str(r#"{"client_id":" abc ","theme":"Light"}"#).unwrap();
        let s = s.sanitized();
        assert_eq!(s.legacy_client_id, "abc");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("client_id"));
        assert!(!json.contains("theme"));
    }
}
