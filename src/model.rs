//! Compact data model shared by the backend and the UI.
//!
//! Only the fields the interface actually displays are kept: Web API objects are
//! reduced to these structs right after parsing so large JSON payloads never stay
//! in memory.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// Base62 Spotify id.
    pub id: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub album_id: String,
    pub duration_ms: u32,
    /// Smallest cover available (≈64 px), only downloaded when displayed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub playable: bool,
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

impl Track {
    pub fn artists_joined(&self) -> String {
        join_names(self.artists.iter().map(|a| a.name.as_str()))
    }
}

pub fn join_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for name in names {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(name);
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub id: String,
    pub name: String,
    pub artists: String,
    pub year: String,
    pub total_tracks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub total: u32,
    /// Changes whenever the playlist content changes: lets us reuse the disk cache
    /// without downloading the tracks again.
    pub snapshot_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub query: String,
    pub tracks: std::sync::Arc<Vec<Track>>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub playlists: Vec<PlaylistSummary>,
}

/// Identifies what the main panel shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ViewKey {
    Welcome,
    Liked,
    SavedAlbums,
    Playlist(String),
    Album(String),
    Artist(String),
    Search(String),
    Queue,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

impl Repeat {
    pub fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_serde_skips_defaults() {
        let t = Track { id: "abc".into(), name: "Song".into(), playable: true, ..Default::default() };
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("playable"));
        assert!(!json.contains("image"));
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn joins_artist_names() {
        let t = Track {
            artists: vec![
                ArtistRef { id: "1".into(), name: "A".into() },
                ArtistRef { id: "2".into(), name: "B".into() },
            ],
            ..Default::default()
        };
        assert_eq!(t.artists_joined(), "A, B");
    }
}
