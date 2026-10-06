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

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The artists of `tracks`, those with the most tracks first (at most `max`,
/// leaving out `skip`). Needs no request: it works from the liked tracks cache.
pub fn artists_by_count(tracks: &[Track], skip: &[ArtistSummary], max: usize) -> Vec<ArtistSummary> {
    let mut counts: std::collections::HashMap<&str, (u32, &str)> = std::collections::HashMap::new();
    for artist in tracks.iter().flat_map(|t| &t.artists) {
        if !artist.id.is_empty() {
            counts.entry(&artist.id).or_insert((0, &artist.name)).0 += 1;
        }
    }
    let mut artists: Vec<_> = counts
        .into_iter()
        .filter(|(id, _)| !skip.iter().any(|s| s.id == *id))
        .map(|(id, (liked, name))| ArtistSummary { id: id.into(), name: name.into(), image: None, liked })
        .collect();
    artists.sort_by(|a, b| {
        b.liked.cmp(&a.liked).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    artists.truncate(max);
    artists
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
    /// Larger cover (≈300 px) for the page banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Number of the user's liked tracks by this artist (0 when not counted).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub liked: u32,
}

/// What the Artists page shows.
#[derive(Clone, Debug, Default)]
pub struct ArtistsPage {
    pub followed: Vec<ArtistSummary>,
    /// Artists of the liked tracks that are not followed, most liked first.
    pub library: Vec<ArtistSummary>,
    /// Why followed artists could not be read, if they could not.
    pub problem: Option<String>,
    /// The authorization must be renewed to read followed artists.
    pub needs_auth: bool,
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
    /// Cover (≈300 px) for the page banner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
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
    /// Artists the user follows.
    Artists,
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

    #[test]
    fn counts_artists_of_liked_tracks() {
        let track = |ids: &[&str]| Track {
            artists: ids.iter().map(|id| ArtistRef { id: id.to_string(), name: id.to_uppercase() }).collect(),
            ..Default::default()
        };
        let tracks =
            [track(&["b"]), track(&["a", "b"]), track(&["c"]), track(&["a"]), track(&["b"]), track(&[""])];
        let followed = [ArtistSummary { id: "c".into(), ..Default::default() }];
        let artists = artists_by_count(&tracks, &followed, 10);
        let summary: Vec<_> = artists.iter().map(|a| (a.name.as_str(), a.liked)).collect();
        assert_eq!(summary, [("B", 3), ("A", 2)]);
        assert_eq!(artists_by_count(&tracks, &[], 1).len(), 1);
    }
}
