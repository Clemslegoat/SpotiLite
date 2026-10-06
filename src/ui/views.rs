//! Screens and panels: rounded surfaces on a black window.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Layout, Margin, Rect, RichText, ScrollArea,
    Sense, Stroke, Ui, UiBuilder, pos2, vec2,
};

use super::theme::{
    self, GAP, PLAYER_HEIGHT, RADIUS_CARD, RADIUS_ROW, RADIUS_SURFACE, ROW_HEIGHT, SIDEBAR_WIDTH,
};
use super::widgets::{self, ButtonStyle, Icon, format_duration, format_total, paint_text};
use super::{Ambient, App, Page};
use crate::backend::{AppCredentials, AppState, Command, human_bytes};
use crate::model::{AlbumSummary, ArtistSummary, ArtistsPage, PlaylistSummary, Repeat, Track, ViewKey};

fn surface(p: &theme::Palette, outer: Margin, inner: Margin) -> Frame {
    Frame::new()
        .fill(p.surface)
        .corner_radius(CornerRadius::same(RADIUS_SURFACE))
        .outer_margin(outer)
        .inner_margin(inner)
}

/// The application mark (feather and sound waves), in white.
pub(super) fn logo(ui: &Ui, center: egui::Pos2, size: f32, texture: egui::TextureId) {
    let rect = Rect::from_center_size(center, vec2(size, size));
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture, rect, uv, Color32::WHITE);
}

// ----------------------------------------------------------------------------
// Splash (while the saved authorization is read)

pub fn splash(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg)).show(ui, |ui| {
        let center = ui.max_rect().center();
        logo(ui, center - vec2(0.0, 30.0), 64.0, app.logo.id());
        ui.painter().text(
            center + vec2(0.0, 22.0),
            Align2::CENTER_CENTER,
            "SpotiLite",
            theme::strong_font(20.0),
            p.text,
        );
    });
}

// ----------------------------------------------------------------------------
// Main layout

pub fn main_layout(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::Panel::bottom("player")
        .exact_size(PLAYER_HEIGHT + f32::from(GAP))
        .resizable(false)
        .show_separator_line(false)
        .frame(surface(
            &p,
            Margin { left: GAP, right: GAP, top: 0, bottom: GAP },
            Margin::symmetric(PLAYER_MARGIN_X, PLAYER_MARGIN_Y),
        ))
        .show(ui, |ui| player_bar(app, ui));
    egui::Panel::left("sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(surface(
            &p,
            Margin { left: GAP, right: 0, top: 0, bottom: GAP },
            Margin { left: 12, right: 12, top: 12, bottom: 10 },
        ))
        .show(ui, |ui| sidebar(app, ui));
    egui::CentralPanel::default()
        .frame(surface(
            &p,
            Margin { left: GAP, right: GAP, top: 0, bottom: GAP },
            Margin { left: 26, right: 18, top: 18, bottom: 6 },
        ))
        .show(ui, |ui| content(app, ui));
}

fn sidebar(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    // The logo and name are in the title bar.
    let search = widgets::search_field(
        ui,
        &p,
        &mut app.search_text,
        "Rechercher (Ctrl+F)",
        ui.available_width(),
        Icon::Search,
    );
    if app.focus_search {
        search.request_focus();
        app.focus_search = false;
    }
    if search.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let query = app.search_text.trim().to_string();
        if !query.is_empty() {
            app.navigate(ViewKey::Search(query));
        }
    }
    ui.add_space(12.0);

    nav_item(app, ui, Icon::Home, "Accueil", ViewKey::Home, None);
    nav_item(app, ui, Icon::Heart { filled: true }, "Titres likés", ViewKey::Liked, None);
    nav_item(app, ui, Icon::Disc, "Albums", ViewKey::SavedAlbums, None);
    nav_item(app, ui, Icon::Artist, "Artistes", ViewKey::Artists, None);
    let count = app.player.upcoming.len();
    let badge = (count > 0).then(|| count.min(99).to_string());
    nav_item(app, ui, Icon::Queue, "File d'attente", ViewKey::Queue, badge);

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("PLAYLISTS").font(theme::strong_font(11.0)).color(p.faint));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let busy = app.playlists_loading.is_some_and(|t| t.elapsed() < Duration::from_secs(15));
            if widgets::refresh_button(ui, &p, 24.0, busy, "Actualiser les playlists") {
                app.playlists_loading = Some(Instant::now());
                app.send(Command::LoadPlaylists);
            }
        });
    });
    ui.add_space(2.0);

    let footer_height = 66.0;
    let list_height = (ui.available_height() - footer_height).max(40.0);
    let playlists = app.playlists.clone();
    ScrollArea::vertical()
        .id_salt("playlists")
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, 32.0, playlists.len(), |ui, range| {
            for playlist in &playlists[range] {
                let view = ViewKey::Playlist(playlist.id.clone());
                nav_row(app, ui, None, &playlist.name, view, 32.0, None)
                    .on_hover_text(format!("{} · {} titres", playlist.owner, playlist.total));
            }
        });

    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let small = egui::FontId::proportional(11.5);
        let ram = format!("RAM : {}", human_bytes(app.total_memory()));
        ui.label(RichText::new(ram).font(small).color(p.faint)).on_hover_text(
            "Mémoire utilisée par SpotiLite, lecteur compris (valeurs du Gestionnaire des tâches).",
        );
        ui.add_space(4.0);
        nav_item(app, ui, Icon::Settings, "Réglages", ViewKey::Settings, None);
    });
}

fn nav_item(app: &mut App, ui: &mut Ui, icon: Icon, label: &str, view: ViewKey, badge: Option<String>) {
    nav_row(app, ui, Some(icon), label, view, 38.0, badge);
}

fn nav_row(
    app: &mut App,
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    view: ViewKey,
    height: f32,
    badge: Option<String>,
) -> egui::Response {
    let p = app.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let selected = app.view == view;
    let hovered = response.hovered();
    if selected || hovered {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(RADIUS_ROW),
            if selected { p.raised } else { p.hover },
        );
    }
    let color = if selected || hovered { p.text } else { p.dim };
    let mut x = rect.left() + 12.0;
    if let Some(icon) = icon {
        let icon_rect = Rect::from_center_size(pos2(x + 8.0, rect.center().y), vec2(17.0, 17.0));
        widgets::paint_icon(ui.painter(), icon_rect, icon, if selected { p.accent } else { color });
        x += 28.0;
    }
    let font = if icon.is_some() { theme::strong_font(14.0) } else { theme::body_font() };
    let badge_w = if badge.is_some() { 30.0 } else { 0.0 };
    paint_text(ui, pos2(x, rect.center().y), label, font, color, rect.right() - x - 8.0 - badge_w);
    if let Some(badge) = badge {
        let pill = Rect::from_center_size(pos2(rect.right() - 20.0, rect.center().y), vec2(26.0, 18.0));
        ui.painter().rect_filled(pill, CornerRadius::same(9), p.line);
        ui.painter().text(pill.center(), Align2::CENTER_CENTER, badge, theme::strong_font(11.0), p.text);
    }
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    if response.clicked() {
        app.navigate(view);
    }
    response
}

// ----------------------------------------------------------------------------
// Content

fn content(app: &mut App, ui: &mut Ui) {
    let view = app.view.clone();
    match &view {
        ViewKey::Settings => return settings_page(app, ui),
        ViewKey::Queue => return queue_page(app, ui),
        ViewKey::Home => return home_page(app, ui),
        _ => {}
    }
    let p = app.palette;
    if let Some(message) = app.failures.get(&view).cloned() {
        header(app, ui, "Oups", "", None);
        ui.add_space(14.0);
        note(ui, &p, &message);
        ui.add_space(10.0);
        if widgets::pill_with_icon(ui, &p, Some(Icon::Refresh), "Réessayer", true).clicked() {
            app.navigate_with(view, true);
        }
        return;
    }
    if !app.pages.contains_key(&view) {
        ui.add_space(40.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().color(p.accent).size(18.0));
            ui.label(RichText::new("Chargement…").color(p.dim));
        });
        return;
    }
    match app.pages.get(&view) {
        Some(Page::Artists(page)) => {
            let page = page.clone();
            artists_page(app, ui, &page);
        }
        Some(Page::Tracks { title, subtitle, cover, tracks }) => {
            let (title, subtitle, cover, tracks) =
                (title.clone(), subtitle.clone(), cover.clone(), tracks.clone());
            tracks_page(app, ui, &title, &subtitle, cover.as_ref(), tracks);
        }
        Some(Page::Context { title, subtitle, cover, uri, total }) => {
            let (title, subtitle, cover, uri, total) =
                (title.clone(), subtitle.clone(), cover.clone(), uri.clone(), *total);
            context_page(app, ui, &title, &subtitle, cover.as_ref(), &uri, total);
        }
        Some(Page::Albums { title, albums }) => {
            let (title, albums) = (title.clone(), albums.clone());
            header(app, ui, &title, &format!("{} albums", albums.len()), None);
            ui.add_space(12.0);
            album_list(app, ui, &albums, "albums");
        }
        Some(Page::Artist { name, image, liked, albums }) => {
            let (name, image, liked, albums) = (name.clone(), image.clone(), liked.clone(), albums.clone());
            artist_page(app, ui, &name, image.as_ref(), liked, &albums);
        }
        Some(Page::Search(results)) => {
            let results = results.clone();
            search_page(app, ui, &results);
        }
        None => {}
    }
}

fn home_page(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let hello = if app.user.is_empty() { "Bonjour".to_string() } else { format!("Bonjour {}", app.user) };
    header(app, ui, &hello, "", None);
    ui.add_space(18.0);
    ScrollArea::vertical().id_salt("home").auto_shrink([false, false]).show(ui, |ui| {
        shortcuts(app, ui);
        if !app.recent.is_empty() {
            ui.add_space(18.0);
            section_title(ui, &p, "Écoutés récemment");
            ui.add_space(4.0);
            recent_grid(app, ui);
        }
        ui.add_space(12.0);
    });
}

/// Tiles to the library and the first playlists.
fn shortcuts(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    // Icon, label, page, and the cover of a playlist.
    let mut shortcuts: Vec<(Icon, String, ViewKey, Option<String>)> = vec![
        (Icon::Heart { filled: true }, "Titres likés".into(), ViewKey::Liked, None),
        (Icon::Disc, "Albums".into(), ViewKey::SavedAlbums, None),
        (Icon::Artist, "Artistes".into(), ViewKey::Artists, None),
    ];
    shortcuts.extend(
        app.playlists
            .iter()
            .take(5)
            .map(|pl| (Icon::Library, pl.name.clone(), ViewKey::Playlist(pl.id.clone()), pl.image.clone())),
    );
    let gap = 10.0;
    let columns = ((ui.available_width() + gap) / 240.0).floor().clamp(1.0, 4.0) as usize;
    let width = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
    for row in shortcuts.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (icon, label, view, cover) in row {
                let (rect, response) = ui.allocate_exact_size(vec2(width, 58.0), Sense::click());
                let fill = if response.hovered() { p.line } else { p.raised };
                ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_CARD), fill);
                let tile = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(42.0, 42.0));
                let art = if *view == ViewKey::Liked {
                    app.liked_art(&ui.ctx().clone())
                } else if app.settings.show_covers {
                    app.cover(cover.as_ref())
                } else {
                    None
                };
                if art.is_some() {
                    widgets::cover(ui, &p, tile, art, 10);
                } else {
                    ui.painter().rect_filled(tile, CornerRadius::same(10), p.surface);
                    widgets::paint_icon(ui.painter(), tile.shrink(10.0), *icon, p.text);
                }
                paint_text(
                    ui,
                    pos2(tile.right() + 12.0, rect.center().y),
                    label,
                    theme::strong_font(14.0),
                    p.text,
                    rect.right() - tile.right() - 20.0,
                );
                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    app.navigate(view.clone());
                }
            }
        });
        ui.add_space(gap);
    }
}

/// Recently played tracks as cards: cover, title and artists; a click plays
/// from there.
fn recent_grid(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let tracks = app.recent.clone();
    let gap = 12.0;
    let width = ui.available_width();
    let columns = ((width + gap) / (170.0 + gap)).floor().max(2.0) as usize;
    let card_w = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    let art = (card_w - 20.0).min(160.0);
    let card_h = art + 64.0;
    for (row_index, row) in tracks.chunks(columns).take(3).enumerate() {
        let (row_rect, _) = ui.allocate_exact_size(vec2(width, card_h), Sense::hover());
        if !ui.is_rect_visible(row_rect) {
            ui.add_space(gap);
            continue;
        }
        for (i, track) in row.iter().enumerate() {
            let index = row_index * columns + i;
            let rect = Rect::from_min_size(
                row_rect.min + vec2(i as f32 * (card_w + gap), 0.0),
                vec2(card_w, card_h),
            );
            let response = ui.interact(rect, Id::new(("recent", index)), Sense::click());
            let hovered = response.hovered();
            if hovered {
                ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_CARD), p.hover);
            }
            let cover = Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(art, art));
            let url = track.image.as_deref().map(larger_cover);
            let texture = if app.settings.show_covers { app.cover(url.as_ref()) } else { None };
            widgets::cover(ui, &p, cover, texture, 8);
            let playing = app.player.now.as_ref().is_some_and(|t| t.id == track.id) && app.player.playing;
            if hovered || playing {
                // Round play button over the cover, like the banners'.
                let center = cover.right_bottom() - vec2(26.0, 26.0);
                ui.painter().circle_filled(center, 20.0, p.accent);
                let icon = if playing { Icon::Pause } else { Icon::Play };
                let glyph = Rect::from_center_size(center, vec2(22.0, 22.0));
                widgets::paint_icon(ui.painter(), glyph, icon, p.on_accent);
            }
            paint_text(
                ui,
                pos2(cover.left(), cover.bottom() + 16.0),
                &track.name,
                theme::strong_font(14.0),
                p.text,
                art,
            );
            let artists = track.artists_joined();
            paint_text(
                ui,
                pos2(cover.left(), cover.bottom() + 36.0),
                &artists,
                theme::small_font(),
                p.dim,
                art,
            );
            if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                if playing {
                    app.send(Command::PlayPause);
                } else if track.playable {
                    app.play(tracks.clone(), index);
                }
            }
        }
        ui.add_space(gap);
    }
}

/// Spotify serves each cover in 64, 300 and 640 px under the same name with a
/// size prefix: the cards use the 300 px one (other links are kept as they are).
fn larger_cover(url: &str) -> String {
    url.replacen("ab67616d00004851", "ab67616d00001e02", 1)
}

/// Title block with a back button and optional refresh action.
fn header(app: &mut App, ui: &mut Ui, title: &str, subtitle: &str, refresh: Option<&ViewKey>) {
    let p = app.palette;
    ui.horizontal(|ui| {
        if !app.history.is_empty()
            && widgets::round_button(ui, &p, Icon::Back, 32.0, ButtonStyle::Raised, false)
                .on_hover_text("Retour (Alt+←)")
                .clicked()
        {
            app.back();
        }
        let width = ui.available_width() - if refresh.is_some() { 44.0 } else { 0.0 };
        let galley = widgets::truncated(ui, title, theme::heading_font(), p.text, width);
        ui.label(galley);
        if let Some(view) = refresh {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let busy = app.loading.contains(view);
                if widgets::refresh_button(ui, &p, 30.0, busy, "Actualiser depuis Spotify") {
                    let view = view.clone();
                    app.navigate_with(view, true);
                }
            });
        }
    });
    if !subtitle.is_empty() {
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).color(p.dim));
    }
}

/// Rounded card with a message and an optional button; true when it is clicked.
fn notice(ui: &mut Ui, p: &theme::Palette, text: &str, action: Option<&str>) -> bool {
    let mut clicked = false;
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(680.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(p.text));
                if let Some(label) = action {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        clicked = widgets::pill(ui, p, label, true).clicked();
                    });
                }
            });
        });
    clicked
}

/// Rounded explanatory card.
fn note(ui: &mut Ui, p: &theme::Palette, text: &str) {
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(620.0));
            ui.label(RichText::new(text).color(p.dim));
        });
}

fn tracks_page(
    app: &mut App,
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    cover: Option<&String>,
    tracks: Arc<Vec<Track>>,
) {
    let view = app.view.clone();
    let total: u64 = tracks.iter().map(|t| u64::from(t.duration_ms)).sum();
    let mut detail = format!("{} · {}", plural(tracks.len(), "titre", "titres"), format_total(total));
    if !subtitle.is_empty() {
        detail = format!("{subtitle} · {detail}");
    }
    back_row(app, ui, Some(&view), true);
    ui.add_space(10.0);
    let liked = view == ViewKey::Liked;
    let liked_key = super::ambient::LIKED.to_string();
    let art = if liked { app.liked_art(&ui.ctx().clone()) } else { None };
    let (play, shuffle) = banner(
        app,
        ui,
        &Banner {
            kind: if matches!(view, ViewKey::Album(_)) { "ALBUM" } else { "PLAYLIST" },
            title,
            detail: &detail,
            image: if liked { Some(&liked_key) } else { cover },
            picture: art.map_or(Picture::Cover, Picture::Art),
            play_hint: "Lecture",
        },
    );
    let visible = app.visible_tracks().unwrap_or_else(|| tracks.clone());
    if play && !visible.is_empty() {
        if app.player.shuffle {
            app.player.shuffle = false;
            app.settings.shuffle = false;
            app.send(Command::SetShuffle(false));
        }
        app.play(visible.clone(), 0);
    }
    if shuffle && !visible.is_empty() {
        app.player.shuffle = true;
        app.settings.shuffle = true;
        app.send(Command::SetShuffle(true));
        let start = rand::random_range(0..visible.len());
        app.play(visible.clone(), start);
    }
    ui.add_space(12.0);
    track_table(app, ui, visible, "tracks");
}

fn context_page(
    app: &mut App,
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    cover: Option<&String>,
    uri: &str,
    total: u32,
) {
    let p = app.palette;
    let view = app.view.clone();
    let detail = match (subtitle.is_empty(), total) {
        (true, 0) => String::new(),
        (true, n) => plural(n as usize, "titre", "titres"),
        (false, 0) => subtitle.to_string(),
        (false, n) => format!("{subtitle} · {}", plural(n as usize, "titre", "titres")),
    };
    back_row(app, ui, Some(&view), false);
    ui.add_space(10.0);
    let banner_info = Banner {
        kind: "PLAYLIST",
        title,
        detail: &detail,
        image: cover,
        picture: Picture::Cover,
        play_hint: "Lecture",
    };
    let (play, shuffle) = banner(app, ui, &banner_info);
    if play || shuffle {
        app.play_context(uri.to_string(), title.to_string(), shuffle, total);
    }
    ui.add_space(14.0);
    note(
        ui,
        &p,
        "Spotify ne communique pas le contenu de cette playlist aux applications en mode développement \
         (seulement celui de vos propres playlists). Elle est donc lue telle quelle par Spotify : les titres \
         s'affichent au fil de la lecture, et la file d'attente montre les suivants.",
    );
}

/// What the tile of a banner shows.
#[derive(Clone, Copy, PartialEq)]
enum Picture {
    /// The image, square with rounded corners.
    Cover,
    /// The image in a circle (artists).
    Portrait,
    /// A picture of the application (the liked tracks one).
    Art(egui::TextureId),
}

struct Banner<'a> {
    kind: &'a str,
    title: &'a str,
    detail: &'a str,
    /// Picture, whose colors also paint the banner.
    image: Option<&'a String>,
    picture: Picture,
    play_hint: &'a str,
}

const BANNER_HEIGHT: f32 = 164.0;

/// Page header in the colors of its picture: picture, kind, title, details,
/// and the play and shuffle buttons. Returns which button was clicked.
fn banner(app: &mut App, ui: &mut Ui, info: &Banner) -> (bool, bool) {
    let p = app.palette;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BANNER_HEIGHT), Sense::hover());
    let ctx = ui.ctx().clone();
    let ambient = info.image.and_then(|url| app.ambient(&ctx, url, Ambient::Banner));
    let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    match ambient {
        Some(texture) => {
            ui.painter().add(
                egui::epaint::RectShape::filled(rect, CornerRadius::same(RADIUS_CARD), Color32::WHITE)
                    .with_texture(texture, full_uv),
            );
        }
        None => {
            ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_CARD), p.raised);
        }
    }

    let tile = Rect::from_min_size(rect.min + vec2(20.0, 20.0), vec2(124.0, 124.0));
    match info.picture {
        Picture::Portrait => {
            let texture = if app.settings.show_covers { app.cover(info.image) } else { None };
            widgets::avatar(ui, &p, tile, texture, info.title);
        }
        Picture::Cover => {
            let texture = if app.settings.show_covers { app.cover(info.image) } else { None };
            widgets::cover(ui, &p, tile, texture, 10);
        }
        Picture::Art(texture) => widgets::cover(ui, &p, tile, Some(texture), 10),
    }

    // Buttons on the right, text in between.
    let play_rect = Rect::from_center_size(pos2(rect.right() - 52.0, rect.center().y), vec2(52.0, 52.0));
    let shuffle_rect =
        Rect::from_center_size(pos2(play_rect.left() - 30.0, rect.center().y), vec2(40.0, 40.0));
    let x = tile.right() + 24.0;
    let width = (shuffle_rect.left() - 16.0 - x).max(40.0);
    let light = Color32::from_gray(0xd8);
    let cy = rect.center().y;
    paint_text(ui, pos2(x, cy - 32.0), info.kind, theme::strong_font(11.5), light, width);
    paint_text(ui, pos2(x, cy), info.title, theme::strong_font(34.0), p.text, width);
    paint_text(ui, pos2(x, cy + 32.0), info.detail, theme::body_font(), light, width);

    // Child areas that do not move the page layout (the list starts under the banner).
    let play = widgets::round_button(
        &mut ui.new_child(UiBuilder::new().max_rect(play_rect)),
        &p,
        Icon::Play,
        52.0,
        ButtonStyle::Accent,
        false,
    )
    .on_hover_text(info.play_hint)
    .clicked();
    let shuffle = widgets::icon_button(
        &mut ui.new_child(UiBuilder::new().max_rect(shuffle_rect)),
        &p,
        Icon::Shuffle,
        40.0,
        28.0,
        ButtonStyle::Bright,
        false,
    )
    .on_hover_text("Lecture aléatoire")
    .clicked();
    (play, shuffle)
}

fn artist_page(
    app: &mut App,
    ui: &mut Ui,
    name: &str,
    image: Option<&String>,
    liked: Arc<Vec<Track>>,
    albums: &[AlbumSummary],
) {
    let p = app.palette;
    let view = app.view.clone();
    let ViewKey::Artist(id) = &view else { return };
    back_row(app, ui, Some(&view), false);
    ui.add_space(10.0);
    let mut stats = Vec::new();
    if !liked.is_empty() {
        stats.push(plural(liked.len(), "titre liké", "titres likés"));
    }
    if !albums.is_empty() {
        stats.push(plural(albums.len(), "sortie", "sorties"));
    }
    let detail = stats.join(" · ");
    let banner_info = Banner {
        kind: "ARTISTE",
        title: name,
        detail: &detail,
        image,
        picture: Picture::Portrait,
        play_hint: "Lecture de ses titres populaires, choisis par Spotify",
    };
    let (play, shuffle) = banner(app, ui, &banner_info);
    if play || shuffle {
        app.play_context(format!("spotify:artist:{id}"), name.to_string(), shuffle, 0);
    }
    ui.add_space(16.0);
    ScrollArea::vertical().id_salt("artist").auto_shrink([false, false]).show(ui, |ui| {
        if !liked.is_empty() {
            section_title(ui, &p, "Dans vos titres likés");
            track_rows_plain(app, ui, liked.clone(), "artist-liked");
            ui.add_space(18.0);
        }
        if !albums.is_empty() {
            section_title(ui, &p, "Discographie");
            for album in albums {
                album_row(app, ui, album);
            }
        }
        if liked.is_empty() && albums.is_empty() {
            note(ui, &p, "▶ fait jouer ses titres populaires par Spotify.");
        }
        ui.add_space(12.0);
    });
}

fn artists_page(app: &mut App, ui: &mut Ui, page: &ArtistsPage) {
    let p = app.palette;
    let mut subtitle = match page.followed.len() {
        0 => "Aucun artiste suivi".to_string(),
        n => plural(n, "artiste suivi", "artistes suivis"),
    };
    if !page.library.is_empty() {
        subtitle.push_str(&format!(" · {} dans vos titres likés", page.library.len()));
    }
    header(app, ui, "Artistes", &subtitle, Some(&ViewKey::Artists));
    ui.add_space(14.0);
    if let Some(problem) = &page.problem {
        let action = if page.needs_auth { Some("Autoriser") } else { None };
        if notice(ui, &p, problem, action) {
            app.send(Command::ReconnectApp);
        }
        ui.add_space(14.0);
    }
    ScrollArea::vertical().id_salt("artists").auto_shrink([false, false]).show(ui, |ui| {
        if !page.followed.is_empty() {
            section_title(ui, &p, "Suivis");
            artist_grid(app, ui, &page.followed, "followed");
            ui.add_space(14.0);
        }
        if !page.library.is_empty() {
            section_title(ui, &p, "Dans vos titres likés");
            artist_grid(app, ui, &page.library, "library");
        }
        if page.followed.is_empty() && page.library.is_empty() && page.problem.is_none() {
            note(
                ui,
                &p,
                "Les artistes que vous suivez dans Spotify et ceux de vos titres likés apparaîtront ici.",
            );
        }
        ui.add_space(12.0);
    });
}

/// Round portraits in a grid, name and detail centered underneath.
fn artist_grid(app: &mut App, ui: &mut Ui, artists: &[ArtistSummary], salt: &str) {
    let p = app.palette;
    let gap = 10.0;
    let width = ui.available_width();
    let columns = ((width + gap) / (150.0 + gap)).floor().max(2.0) as usize;
    let tile_w = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    let portrait = if tile_w >= 140.0 { 112.0 } else { 80.0 };
    let tile_h = portrait + 70.0;
    for row in artists.chunks(columns) {
        let (row_rect, _) = ui.allocate_exact_size(vec2(width, tile_h), Sense::hover());
        if ui.is_rect_visible(row_rect) {
            for (i, artist) in row.iter().enumerate() {
                let rect = Rect::from_min_size(
                    row_rect.min + vec2(i as f32 * (tile_w + gap), 0.0),
                    vec2(tile_w, tile_h),
                );
                let response = ui.interact(rect, Id::new((salt, &artist.id)), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_CARD), p.hover);
                }
                let circle = Rect::from_center_size(
                    pos2(rect.center().x, rect.top() + 14.0 + portrait * 0.5),
                    vec2(portrait, portrait),
                );
                let texture = if app.settings.show_covers { app.cover(artist.image.as_ref()) } else { None };
                widgets::avatar(ui, &p, circle, texture, &artist.name);
                let name =
                    widgets::truncated(ui, &artist.name, theme::strong_font(14.0), p.text, tile_w - 16.0);
                let name_pos = pos2(rect.center().x - name.size().x * 0.5, circle.bottom() + 12.0);
                ui.painter().galley(name_pos, name, p.text);
                let detail = if artist.liked > 0 {
                    plural(artist.liked as usize, "titre liké", "titres likés")
                } else {
                    "Artiste".to_string()
                };
                ui.painter().text(
                    pos2(rect.center().x, circle.bottom() + 40.0),
                    Align2::CENTER_CENTER,
                    detail,
                    theme::small_font(),
                    p.dim,
                );
                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                    app.navigate(ViewKey::Artist(artist.id.clone()));
                }
            }
        }
        ui.add_space(gap);
    }
}

/// "1 titre liké", "3 titres likés".
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// Back button and, on the right, the refresh button and the list filter.
fn back_row(app: &mut App, ui: &mut Ui, refresh: Option<&ViewKey>, filter: bool) {
    let p = app.palette;
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        if !app.history.is_empty()
            && widgets::round_button(ui, &p, Icon::Back, 32.0, ButtonStyle::Raised, false)
                .on_hover_text("Retour (Alt+←)")
                .clicked()
        {
            app.back();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(view) = refresh {
                let busy = app.loading.contains(view);
                if widgets::refresh_button(ui, &p, 30.0, busy, "Actualiser depuis Spotify") {
                    let view = view.clone();
                    app.navigate_with(view, true);
                }
            }
            if filter {
                ui.add_space(6.0);
                widgets::search_field(ui, &p, &mut app.filter, "Filtrer", 210.0, Icon::Search);
            }
        });
    });
}

fn search_page(app: &mut App, ui: &mut Ui, results: &crate::model::SearchResults) {
    let p = app.palette;
    header(app, ui, &format!("« {} »", results.query), "Résultats de recherche", None);
    ui.add_space(12.0);
    ScrollArea::vertical().id_salt("search").auto_shrink([false, false]).show(ui, |ui| {
        let empty = results.tracks.is_empty()
            && results.albums.is_empty()
            && results.artists.is_empty()
            && results.playlists.is_empty();
        if empty {
            note(ui, &p, "Aucun résultat.");
        }
        if !results.tracks.is_empty() {
            section_title(ui, &p, "Titres");
            track_rows_plain(app, ui, results.tracks.clone(), "search-tracks");
            ui.add_space(16.0);
        }
        if !results.artists.is_empty() {
            section_title(ui, &p, "Artistes");
            artist_grid(app, ui, &results.artists, "search-artists");
            ui.add_space(8.0);
        }
        if !results.albums.is_empty() {
            section_title(ui, &p, "Albums");
            for album in &results.albums {
                album_row(app, ui, album);
            }
            ui.add_space(16.0);
        }
        if !results.playlists.is_empty() {
            section_title(ui, &p, "Playlists");
            for playlist in &results.playlists {
                playlist_row(app, ui, playlist);
            }
        }
        ui.add_space(12.0);
    });
}

fn queue_page(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    header(app, ui, "File d'attente", "", None);
    ui.add_space(12.0);
    if let Some(context) = app.player.context.clone() {
        note(ui, &p, &format!("Spotify lit « {context} » : il choisit lui-même les titres suivants."));
        ui.add_space(12.0);
    }
    if let Some(now) = app.player.now.clone() {
        section_title(ui, &p, "En cours");
        track_rows_plain(app, ui, Arc::new(vec![now]), "queue-now");
        ui.add_space(14.0);
    }
    ui.horizontal(|ui| {
        section_title(ui, &p, "À suivre");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if app.player.context.is_none()
                && !app.player.upcoming.is_empty()
                && widgets::pill(ui, &p, "Vider les ajouts", false).clicked()
            {
                app.send(Command::ClearQueue);
            }
        });
    });
    let upcoming = Arc::new(app.player.upcoming.clone());
    if upcoming.is_empty() {
        ui.label(
            RichText::new("Rien à suivre. Clic droit sur un titre → « Ajouter à la file ».").color(p.dim),
        );
    } else {
        track_table(app, ui, upcoming, "queue");
    }
}

fn section_title(ui: &mut Ui, p: &theme::Palette, text: &str) {
    ui.label(RichText::new(text).font(theme::strong_font(17.0)).color(p.text));
    ui.add_space(6.0);
}

// ----------------------------------------------------------------------------
// Track rows

struct Columns {
    index: f32,
    title: f32,
    album: f32,
}

fn columns(width: f32) -> Columns {
    let index = 44.0;
    let duration = 60.0;
    let rest = (width - index - duration).max(80.0);
    if width > 680.0 {
        Columns { index, title: rest * 0.6, album: rest * 0.4 }
    } else {
        Columns { index, title: rest, album: 0.0 }
    }
}

/// Virtualized table: only the visible rows are laid out, so a 10 000 track list
/// costs the same as a 20 track one.
fn track_table(app: &mut App, ui: &mut Ui, tracks: Arc<Vec<Track>>, salt: &str) {
    let p = app.palette;
    let width = ui.available_width();
    let cols = columns(width);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    let font = theme::strong_font(11.0);
    let y = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + cols.index * 0.5, y),
        Align2::CENTER_CENTER,
        "#",
        font.clone(),
        p.faint,
    );
    paint_text(ui, pos2(rect.left() + cols.index, y), "TITRE", font.clone(), p.faint, cols.title);
    if cols.album > 0.0 {
        paint_text(
            ui,
            pos2(rect.left() + cols.index + cols.title, y),
            "ALBUM",
            font.clone(),
            p.faint,
            cols.album,
        );
    }
    ui.painter().text(pos2(rect.right() - 14.0, y), Align2::RIGHT_CENTER, "DURÉE", font, p.faint);
    ui.painter().line_segment(
        [pos2(rect.left() + 8.0, rect.bottom()), pos2(rect.right() - 8.0, rect.bottom())],
        Stroke::new(1.0, p.line),
    );
    ui.add_space(4.0);

    // One scroll position per list: switching playlists starts at the top.
    let scroll_id = Id::new(salt).with(&app.view);
    let mut scroll = ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false, false]);
    if app.reveal_selected {
        if let Some(sel) = app.selected {
            let spacing = ui.spacing().item_spacing.y;
            let target = sel as f32 * (ROW_HEIGHT + spacing);
            let viewport = ui.available_height();
            let offset: f32 = ui.ctx().data(|d| d.get_temp::<f32>(scroll_id.with("offset"))).unwrap_or(0.0);
            if target < offset || target + ROW_HEIGHT > offset + viewport {
                scroll = scroll.vertical_scroll_offset((target - viewport / 2.0).max(0.0));
            }
        }
        app.reveal_selected = false;
    }
    let output = scroll.show_rows(ui, ROW_HEIGHT, tracks.len(), |ui, range| {
        for index in range {
            track_row(app, ui, &tracks, index, &cols, salt);
        }
    });
    let offset = output.state.offset.y;
    ui.ctx().data_mut(|d| d.insert_temp(scroll_id.with("offset"), offset));
}

/// Non-virtualized rows for short lists embedded in a scroll area.
fn track_rows_plain(app: &mut App, ui: &mut Ui, tracks: Arc<Vec<Track>>, salt: &str) {
    let cols = columns(ui.available_width());
    for index in 0..tracks.len() {
        track_row(app, ui, &tracks, index, &cols, salt);
    }
}

fn track_row(app: &mut App, ui: &mut Ui, tracks: &Arc<Vec<Track>>, index: usize, cols: &Columns, salt: &str) {
    let p = app.palette;
    let track = &tracks[index];
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let is_current = app.player.now.as_ref().is_some_and(|t| t.id == track.id);
    let selected = app.selected == Some(index) && salt == "tracks";
    let hovered = response.hovered();
    if hovered || selected || is_current {
        let fill = if selected || is_current { p.raised } else { p.hover };
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_ROW), fill);
    }
    let y = rect.center().y;
    let text_color = if !track.playable { p.faint } else { p.text };
    let dim = if track.playable { p.dim } else { p.faint };

    // Index / state column.
    let index_rect = Rect::from_min_size(rect.min, vec2(cols.index, rect.height()));
    if hovered && track.playable {
        widgets::paint_icon(
            ui.painter(),
            Rect::from_center_size(index_rect.center(), vec2(14.0, 14.0)),
            Icon::Play,
            p.text,
        );
    } else if is_current && app.player.playing {
        equalizer(ui, index_rect.center(), p.accent);
    } else {
        ui.painter().text(
            index_rect.center(),
            Align2::CENTER_CENTER,
            (index + 1).to_string(),
            egui::FontId::proportional(12.5),
            p.faint,
        );
    }

    // Small cover, then title over artists, like a two-line card.
    let mut x = rect.left() + cols.index;
    let mut text_w = cols.title - 14.0;
    if app.settings.show_covers && !matches!(app.view, ViewKey::Album(_)) {
        let art = Rect::from_min_size(pos2(x, y - 19.0), vec2(38.0, 38.0));
        let texture = app.cover(track.image.as_ref());
        widgets::cover(ui, &p, art, texture, 6);
        x += 50.0;
        text_w -= 50.0;
    }
    let title_font = if is_current { theme::strong_font(14.5) } else { theme::body_font() };
    paint_text(ui, pos2(x, y - 9.0), &track.name, title_font, text_color, text_w);
    let artist_text = track.artists_joined();
    let artist_rect = paint_text(ui, pos2(x, y + 10.0), &artist_text, theme::small_font(), dim, text_w);
    let mut link_clicked = false;
    if let Some(id) = track.artists.first().map(|a| a.id.clone()).filter(|id| !id.is_empty())
        && widgets::text_link(ui, Id::new((salt, "artist", index)), artist_rect, &p).clicked()
    {
        app.navigate(ViewKey::Artist(id));
        link_clicked = true;
    }
    if cols.album > 0.0 {
        let album_rect = paint_text(
            ui,
            pos2(rect.left() + cols.index + cols.title, y),
            &track.album,
            theme::body_font(),
            dim,
            cols.album - 14.0,
        );
        if !track.album_id.is_empty()
            && widgets::text_link(ui, Id::new((salt, "album", index)), album_rect, &p).clicked()
        {
            app.navigate(ViewKey::Album(track.album_id.clone()));
            link_clicked = true;
        }
    }
    ui.painter().text(
        pos2(rect.right() - 14.0, y),
        Align2::RIGHT_CENTER,
        format_duration(track.duration_ms),
        theme::small_font(),
        p.faint,
    );

    let response =
        response.on_hover_cursor(if track.playable { CursorIcon::Default } else { CursorIcon::NotAllowed });
    if link_clicked {
        return;
    }
    let clicked_index = response.clicked()
        && response.interact_pointer_pos().is_some_and(|pos| pos.x < rect.left() + cols.index);
    if response.double_clicked() || clicked_index {
        if track.playable {
            app.play(tracks.clone(), index);
        }
    } else if response.clicked() {
        app.selected = Some(index);
    }
    let track = track.clone();
    response.context_menu(|ui| {
        ui.set_min_width(200.0);
        if ui.button("Lire").clicked() {
            app.play(tracks.clone(), index);
            ui.close();
        }
        if ui.button("Ajouter à la file").clicked() {
            app.send(Command::Enqueue(track.clone()));
            ui.close();
        }
        ui.separator();
        let liked = app.player.liked.get(&track.id).copied();
        let like_label =
            if liked == Some(true) { "Retirer des titres likés" } else { "Ajouter aux titres likés" };
        if ui.button(like_label).clicked() {
            app.send(Command::SetLiked { track: track.clone(), liked: liked != Some(true) });
            ui.close();
        }
        if !track.album_id.is_empty() && ui.button("Aller à l'album").clicked() {
            app.navigate(ViewKey::Album(track.album_id.clone()));
            ui.close();
        }
        for artist in track.artists.iter().filter(|a| !a.id.is_empty()) {
            if ui.button(format!("Aller à {}", artist.name)).clicked() {
                app.navigate(ViewKey::Artist(artist.id.clone()));
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Copier le lien").clicked() {
            ui.ctx().copy_text(format!("https://open.spotify.com/track/{}", track.id));
            ui.close();
        }
    });
}

/// Three static bars marking the playing row (static on purpose: no animation,
/// no continuous repaint).
fn equalizer(ui: &Ui, center: egui::Pos2, color: Color32) {
    for (i, h) in [8.0, 13.0, 6.0].iter().enumerate() {
        let x = center.x - 5.0 + i as f32 * 5.0;
        let bar = Rect::from_min_max(pos2(x - 1.5, center.y + 6.5 - h), pos2(x + 1.5, center.y + 6.5));
        ui.painter().rect_filled(bar, CornerRadius::same(2), color);
    }
}

fn album_list(app: &mut App, ui: &mut Ui, albums: &[AlbumSummary], salt: &str) {
    let albums = albums.to_vec();
    ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show_rows(
        ui,
        58.0,
        albums.len(),
        |ui, range| {
            for album in &albums[range] {
                album_row(app, ui, album);
            }
        },
    );
}

fn album_row(app: &mut App, ui: &mut Ui, album: &AlbumSummary) {
    let p = app.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_ROW), p.hover);
    }
    let art = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(44.0, 44.0));
    let texture = if app.settings.show_covers { app.cover(album.image.as_ref()) } else { None };
    widgets::cover(ui, &p, art, texture, 8);
    let x = art.right() + 14.0;
    let width = rect.right() - x - 10.0;
    paint_text(ui, pos2(x, rect.center().y - 9.0), &album.name, theme::strong_font(14.0), p.text, width);
    let mut detail = album.artists.clone();
    if !album.year.is_empty() {
        detail.push_str(&format!(" · {}", album.year));
    }
    if album.total_tracks > 0 {
        detail.push_str(&format!(" · {} titres", album.total_tracks));
    }
    paint_text(ui, pos2(x, rect.center().y + 10.0), &detail, theme::small_font(), p.dim, width);
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        app.navigate(ViewKey::Album(album.id.clone()));
    }
}

fn playlist_row(app: &mut App, ui: &mut Ui, playlist: &PlaylistSummary) {
    let p = app.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(RADIUS_ROW), p.hover);
    }
    let art = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(44.0, 44.0));
    let texture = if app.settings.show_covers { app.cover(playlist.image.as_ref()) } else { None };
    if texture.is_some() {
        widgets::cover(ui, &p, art, texture, 8);
    } else {
        ui.painter().rect_filled(art, CornerRadius::same(8), p.raised);
        widgets::paint_icon(ui.painter(), art.shrink(13.0), Icon::Library, p.dim);
    }
    let x = art.right() + 14.0;
    let width = rect.right() - x - 10.0;
    paint_text(ui, pos2(x, rect.center().y - 9.0), &playlist.name, theme::strong_font(14.0), p.text, width);
    let detail = format!("{} · {} titres", playlist.owner, playlist.total);
    paint_text(ui, pos2(x, rect.center().y + 10.0), &detail, theme::small_font(), p.dim, width);
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        app.navigate(ViewKey::Playlist(playlist.id.clone()));
    }
}

// ----------------------------------------------------------------------------
// Player bar

/// Inner margins of the player bar.
const PLAYER_MARGIN_X: i8 = 20;
const PLAYER_MARGIN_Y: i8 = 10;

fn player_bar(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    // Background: a blurred gradient made from the cover's colors.
    let frame = full.expand2(vec2(f32::from(PLAYER_MARGIN_X), f32::from(PLAYER_MARGIN_Y)));
    if let Some(texture) = app.player_ambient(&ui.ctx().clone()) {
        let painter = ui.ctx().layer_painter(ui.layer_id()).with_clip_rect(frame);
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        painter.add(
            egui::epaint::RectShape::filled(frame, CornerRadius::same(RADIUS_SURFACE), Color32::WHITE)
                .with_texture(texture, uv),
        );
    }
    let left_w = (full.width() * 0.3).clamp(210.0, 380.0);
    let right_w = (full.width() * 0.25).clamp(190.0, 300.0);
    let left = Rect::from_min_size(full.min, vec2(left_w, full.height()));
    let right = Rect::from_min_max(pos2(full.right() - right_w, full.top()), full.max);
    let center =
        Rect::from_min_max(pos2(left.right() + 12.0, full.top()), pos2(right.left() - 12.0, full.bottom()));
    ui.scope_builder(UiBuilder::new().max_rect(left), |ui| now_playing(app, ui, left));
    ui.scope_builder(UiBuilder::new().max_rect(center), |ui| controls(app, ui, center));
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
        volume_and_queue(app, ui, right_w)
    });
}

fn volume_and_queue(app: &mut App, ui: &mut Ui, width: f32) {
    let p = app.palette;
    ui.spacing_mut().item_spacing.x = 6.0;
    let (response, committed, _) =
        widgets::bar(ui, &p, (width - 100.0).clamp(70.0, 140.0), app.settings.volume, true);
    let _ = response.on_hover_text(format!("Volume {:.0} % (Ctrl+↑/↓)", app.settings.volume * 100.0));
    if let Some(v) = committed {
        app.player.volume_before_mute = None;
        app.set_volume(v);
    }
    let level = match app.settings.volume {
        v if v <= 0.0 => 0,
        v if v < 0.5 => 1,
        _ => 2,
    };
    if widgets::icon_button(ui, &p, Icon::Volume { level }, 38.0, 26.0, ButtonStyle::Bright, false)
        .on_hover_text("Couper / rétablir le son")
        .clicked()
    {
        match app.player.volume_before_mute.take() {
            Some(previous) => app.set_volume(previous),
            None => {
                app.player.volume_before_mute = Some(app.settings.volume);
                app.set_volume(0.0);
            }
        }
    }
    let in_queue = app.view == ViewKey::Queue;
    if widgets::icon_button(ui, &p, Icon::Queue, 38.0, 26.0, ButtonStyle::Bright, in_queue)
        .on_hover_text("File d'attente")
        .clicked()
    {
        if in_queue {
            app.back();
        } else {
            app.navigate(ViewKey::Queue);
        }
    }
}

fn now_playing(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = app.palette;
    let cy = rect.center().y;
    let Some(track) = app.player.now.clone() else {
        let text = match &app.player.context {
            Some(context) if app.player.buffering => format!("Lancement de « {context} »…"),
            _ => "Rien en lecture".to_string(),
        };
        paint_text(ui, pos2(rect.left(), cy), &text, theme::body_font(), p.faint, rect.width());
        return;
    };
    let mut x = rect.left();
    if app.settings.show_covers {
        let art = Rect::from_min_size(pos2(x, cy - 28.0), vec2(56.0, 56.0));
        let texture = app.cover(track.image.as_ref());
        widgets::cover(ui, &p, art, texture, 10);
        x += 70.0;
    }
    let text_w = (rect.right() - x - 48.0).max(40.0);
    let title = paint_text(ui, pos2(x, cy - 10.0), &track.name, theme::strong_font(15.0), p.text, text_w);
    let artists = paint_text(
        ui,
        pos2(x, cy + 11.0),
        &track.artists_joined(),
        egui::FontId::proportional(13.0),
        Color32::from_gray(0xc8),
        text_w,
    );
    if !track.album_id.is_empty() && widgets::text_link(ui, Id::new("np-title"), title, &p).clicked() {
        app.navigate(ViewKey::Album(track.album_id.clone()));
    }
    if let Some(artist) = track.artists.first().filter(|a| !a.id.is_empty())
        && widgets::text_link(ui, Id::new("np-artist"), artists, &p).clicked()
    {
        app.navigate(ViewKey::Artist(artist.id.clone()));
    }
    let heart_x = (x + title.width().max(artists.width()) + 26.0).min(rect.right() - 20.0);
    let heart_rect = Rect::from_center_size(pos2(heart_x, cy), vec2(38.0, 38.0));
    let liked = app.player.is_liked() == Some(true);
    let clicked = ui
        .scope_builder(UiBuilder::new().max_rect(heart_rect), |ui| {
            widgets::icon_button(
                ui,
                &p,
                Icon::Heart { filled: liked },
                38.0,
                26.0,
                ButtonStyle::Bright,
                false,
            )
            .on_hover_text("J'aime (Ctrl+L)")
            .clicked()
        })
        .inner;
    if clicked {
        app.toggle_like_current();
    }
}

fn controls(app: &mut App, ui: &mut Ui, rect: Rect) {
    let p = app.palette;
    let has_track = app.player.now.is_some() || app.player.context.is_some();
    // Large buttons with large icons, spread over the middle zone; scaled down
    // only when the window is too narrow for them.
    let (toggle, skip, play) = (36.0, 38.0, 44.0);
    let buttons = toggle * 2.0 + skip * 2.0 + play;
    let k = (rect.width() / (buttons + 4.0 * 12.0)).clamp(0.7, 1.0);
    let (toggle, skip, play) = (toggle * k, skip * k, play * k);
    let gap = ((rect.width() - buttons * k) / 4.0).clamp(6.0, 18.0);
    let row_w = buttons * k + gap * 4.0;
    // Buttons, 4 px, then the 16 px progress row: centered as one block.
    let top = rect.center().y - (play + 4.0 + 16.0) * 0.5;
    let row = Rect::from_center_size(pos2(rect.center().x, top + play * 0.5), vec2(row_w, play));
    ui.scope_builder(UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)), |ui| {
        ui.spacing_mut().item_spacing.x = gap;
        let shuffle = app.player.shuffle;
        if widgets::icon_button(ui, &p, Icon::Shuffle, toggle, toggle * 0.72, ButtonStyle::Plain, shuffle)
            .on_hover_text("Lecture aléatoire")
            .clicked()
        {
            app.player.shuffle = !app.player.shuffle;
            app.settings.shuffle = app.player.shuffle;
            app.send(Command::SetShuffle(app.player.shuffle));
        }
        if widgets::icon_button(ui, &p, Icon::Prev, skip, skip * 0.78, ButtonStyle::Bright, false)
            .on_hover_text("Précédent (Ctrl+←)")
            .clicked()
        {
            app.send(Command::Previous);
        }
        let icon = if app.player.playing { Icon::Pause } else { Icon::Play };
        if widgets::icon_button(ui, &p, icon, play, play * 0.66, ButtonStyle::Accent, false)
            .on_hover_text("Lecture / pause (Espace)")
            .clicked()
            && has_track
        {
            app.send(Command::PlayPause);
        }
        if widgets::icon_button(ui, &p, Icon::Next, skip, skip * 0.78, ButtonStyle::Bright, false)
            .on_hover_text("Suivant (Ctrl+→)")
            .clicked()
        {
            app.send(Command::Next);
        }
        let repeat = app.player.repeat;
        let icon = Icon::Repeat { one: repeat == Repeat::One };
        let on = repeat != Repeat::Off;
        if widgets::icon_button(ui, &p, icon, toggle, toggle * 0.72, ButtonStyle::Plain, on)
            .on_hover_text("Répéter : non / tout / ce titre")
            .clicked()
        {
            let next = repeat.cycle();
            app.player.repeat = next;
            app.settings.repeat = next;
            app.send(Command::SetRepeat(next));
        }
    });

    // Progress row, as wide as the middle zone allows.
    let duration = app.player.now.as_ref().map_or(0, |t| t.duration_ms);
    let position = app.player.position();
    let bar_w = (rect.width() - 2.0 * 58.0).clamp(120.0, 720.0);
    let cy = top + play + 4.0 + 8.0;
    let bar_rect = Rect::from_center_size(pos2(rect.center().x, cy), vec2(bar_w, 18.0));
    let fraction = if duration > 0 { position as f32 / duration as f32 } else { 0.0 };
    let (committed, shown) = ui
        .scope_builder(UiBuilder::new().max_rect(bar_rect), |ui| {
            let (_, committed, shown) = widgets::bar(ui, &p, bar_w, fraction, has_track && duration > 0);
            (committed, shown)
        })
        .inner;
    // While dragging, the left clock follows the pointer.
    let label = if app.player.buffering && !app.player.playing && has_track {
        "…".to_string()
    } else {
        format_duration((shown * duration as f32) as u32)
    };
    let font = egui::FontId::proportional(12.0);
    let clock = Color32::from_gray(0xb8);
    ui.painter().text(pos2(bar_rect.left() - 12.0, cy), Align2::RIGHT_CENTER, label, font.clone(), clock);
    ui.painter().text(
        pos2(bar_rect.right() + 12.0, cy),
        Align2::LEFT_CENTER,
        format_duration(duration),
        font,
        clock,
    );
    if let Some(v) = committed {
        let ms = (v * duration as f32) as u32;
        app.player.position_ms = ms;
        app.player.at = Some(std::time::Instant::now());
        app.send(Command::Seek(ms));
    }
}

// ----------------------------------------------------------------------------
// Settings

fn settings_page(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let ctx = ui.ctx().clone();
    header(app, ui, "Réglages", "", None);
    ui.add_space(12.0);
    let mut changed = false;
    ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(620.0);

        card(ui, &p, "Compte", |ui| {
            let status = app.app_status.clone();
            let state = match status.state {
                AppState::Connected if !app.user.is_empty() => app.user.clone(),
                AppState::Connected => "Connecté".to_string(),
                AppState::Authorizing => "Autorisation en cours…".to_string(),
                AppState::Disconnected => "Déconnecté".to_string(),
                _ => "Aucune application".to_string(),
            };
            option_row(ui, &p, "Spotify", |ui| {
                ui.label(RichText::new(state).color(p.text));
            });
            option_row(ui, &p, "Client ID", |ui| {
                ui.label(RichText::new(mask_id(&status.client_id)).monospace().color(p.dim));
            });
            option_row(ui, &p, "Port de redirection", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.port_draft)
                        .desired_width(64.0)
                        .margin(Margin::symmetric(8, 5)),
                );
                let draft = app.port_draft.trim().parse::<u16>().ok().filter(|port| *port >= 1024);
                if let Some(port) = draft.filter(|port| *port != app.settings.redirect_port)
                    && widgets::pill(ui, &p, "Enregistrer", true).clicked()
                {
                    app.settings.redirect_port = port;
                    changed = true;
                    app.toast(
                        format!("Déclarez http://127.0.0.1:{port}/login dans votre application Spotify."),
                        false,
                    );
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if status.needs_playback_auth && widgets::pill(ui, &p, "Autoriser", true).clicked() {
                    app.send(Command::ReconnectApp);
                }
                if status.state == AppState::Disconnected
                    && widgets::pill(ui, &p, "Reconnecter", true).clicked()
                {
                    app.send(Command::ReconnectApp);
                }
                if widgets::pill(ui, &p, "Modifier l'application", false).clicked() {
                    app.editing_app = true;
                    app.setup_id = status.client_id.clone();
                    app.setup_secret.clear();
                }
                if !status.client_id.is_empty()
                    && widgets::pill(ui, &p, "Oublier l'application", false).clicked()
                {
                    app.send(Command::ForgetApp);
                    app.setup_id.clear();
                    app.setup_secret.clear();
                }
                if widgets::pill(ui, &p, "Se déconnecter", false).clicked() {
                    app.forget_played();
                    app.send(Command::Logout);
                }
            });
        });

        card(ui, &p, "Lecture", |ui| {
            option_row(ui, &p, "Lecteur", |ui| {
                ui.label(RichText::new(&app.engine_status).color(p.text));
            });
            option_row(ui, &p, "Mettre en veille le lecteur", |ui| {
                changed |= widgets::toggle(ui, &p, &mut app.settings.engine_sleep);
            });
        });

        card(ui, &p, "Affichage", |ui| {
            option_row(ui, &p, "Taille du texte", |ui| {
                let choices = [0.9, 1.0, 1.15, 1.3];
                let current = choices.iter().position(|s| (*s - app.settings.ui_scale).abs() < 0.01);
                if let Some(i) = widgets::segmented(ui, &p, &["90 %", "100 %", "115 %", "130 %"], current) {
                    app.settings.ui_scale = choices[i];
                    changed = true;
                }
            });
            option_row(ui, &p, "Pochettes", |ui| {
                changed |= widgets::toggle(ui, &p, &mut app.settings.show_covers);
            });
        });

        card(ui, &p, "Mémoire et données", |ui| {
            option_row(ui, &p, "Utilisée", |ui| {
                ui.label(RichText::new(human_bytes(app.total_memory())).color(p.text));
            });
            option_row(ui, &p, "Libérer quand réduite", |ui| {
                changed |= widgets::toggle(ui, &p, &mut app.settings.trim_when_minimized);
            });
            option_row(ui, &p, "Données de la session", |ui| {
                ui.label(RichText::new(human_bytes(app.usage.0 + app.usage.1)).color(p.text));
            });
            option_row(ui, &p, "Cache", |ui| {
                if widgets::pill(ui, &p, "Vider", false).clicked() {
                    app.send(Command::ClearCache);
                }
            });
        });

        ui.label(RichText::new(format!("SpotiLite {}", env!("CARGO_PKG_VERSION"))).small().color(p.faint));
        ui.add_space(16.0);
    });
    if changed {
        app.apply_settings(&ctx);
    }
}

/// One setting: its name on the left, the control on the right.
fn option_row(ui: &mut Ui, p: &theme::Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(40.0);
        let (rect, _) = ui.allocate_exact_size(vec2(190.0, 40.0), Sense::hover());
        paint_text(
            ui,
            pos2(rect.left(), rect.center().y),
            label,
            theme::body_font(),
            p.dim,
            rect.width() - 8.0,
        );
        add(ui);
    });
}

fn card(ui: &mut Ui, p: &theme::Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(p.raised)
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::strong_font(16.0)).color(p.text));
            ui.add_space(8.0);
            add(ui);
        });
    ui.add_space(12.0);
}

// ----------------------------------------------------------------------------
// Setup of the user's Spotify application

const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

/// Full-page setup: a micro guide to create the application on Spotify's
/// dashboard, then the Client ID / Client Secret form.
pub fn setup_screen(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(GAP))).show(
        ui,
        |ui| {
            Frame::new().fill(p.surface).corner_radius(CornerRadius::same(RADIUS_SURFACE)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    // Two columns on a wide window so the form stays visible without scrolling.
                    let wide = ui.available_width() >= 900.0;
                    let width = if wide {
                        (ui.available_width() - 80.0).min(1060.0)
                    } else {
                        (ui.available_width() - 40.0).clamp(280.0, 600.0)
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - width) / 2.0).max(16.0));
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            setup_header(app, ui);
                            if wide {
                                ui.columns(2, |columns| {
                                    setup_guide(app, &mut columns[0]);
                                    setup_form(app, &mut columns[1]);
                                });
                            } else {
                                setup_guide(app, ui);
                                setup_form(app, ui);
                                ui.add_space(24.0);
                            }
                        });
                    });
                });
            });
        },
    );
}

fn setup_header(app: &App, ui: &mut Ui) {
    let p = app.palette;
    ui.add_space(30.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    logo(ui, pos2(rect.left() + 22.0, rect.center().y), 44.0, app.logo.id());
    paint_text(
        ui,
        pos2(rect.left() + 58.0, rect.center().y),
        "Bienvenue dans SpotiLite",
        theme::heading_font(),
        p.text,
        rect.width() - 60.0,
    );
    ui.add_space(10.0);
    ui.label(
        RichText::new(
            "SpotiLite se connecte à Spotify avec votre propre application Spotify : un quota rien que pour \
             vous, et la lecture par le lecteur officiel. Compte Premium requis.",
        )
        .color(p.dim),
    );
    ui.add_space(18.0);
}

fn setup_guide(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let redirect = format!("http://127.0.0.1:{}/login", app.settings.redirect_port);
    card(ui, &p, "Créer l'application (2 minutes)", |ui| {
        step(ui, &p, 1, |ui| {
            ui.label(
                RichText::new("Ouvrez le tableau de bord Spotify et cliquez sur « Create app ».")
                    .color(p.text),
            );
            ui.add_space(6.0);
            if widgets::pill(ui, &p, "Ouvrir le tableau de bord", false).clicked() {
                let _ = open::that_detached(DASHBOARD_URL);
            }
        });
        step(ui, &p, 2, |ui| {
            ui.label(
                RichText::new("Nom et description : au choix, par exemple « SpotiLite ».").color(p.text),
            );
        });
        step(ui, &p, 3, |ui| {
            ui.label(
                RichText::new(
                    "Dans « Redirect URIs », collez exactement cette adresse puis cliquez sur « Add » :",
                )
                .color(p.text),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                Frame::new()
                    .fill(p.surface)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 7))
                    .show(ui, |ui| {
                        ui.label(RichText::new(&redirect).monospace().color(p.accent));
                    });
                if widgets::pill(ui, &p, "Copier", false).clicked() {
                    ui.ctx().copy_text(redirect.clone());
                    app.toast("Adresse copiée.".into(), false);
                }
            });
        });
        step(ui, &p, 4, |ui| {
            ui.label(
                RichText::new(
                    "Cochez « Web API » et « Web Playback SDK », acceptez les conditions puis cliquez sur « Save ».",
                )
                .color(p.text),
            );
        });
        step(ui, &p, 5, |ui| {
            ui.label(
                RichText::new(
                    "Ouvrez « Settings » : copiez le Client ID, puis le Client Secret (« View client secret »), \
                     et collez-les dans « Identifiants ».",
                )
                .color(p.text),
            );
        });
    });
}

fn setup_form(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let status = app.app_status.clone();
    card(ui, &p, "Identifiants", |ui| {
        let keeps_secret =
            status.has_secret && app.setup_id.trim() == status.client_id && !status.client_id.is_empty();
        ui.label(RichText::new("Client ID").color(p.dim));
        ui.add(
            egui::TextEdit::singleline(&mut app.setup_id)
                .hint_text("32 caractères")
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .margin(Margin::symmetric(10, 8)),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Client Secret").color(p.dim));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if app.show_secret { "Masquer" } else { "Afficher" };
                if ui
                    .add(egui::Label::new(RichText::new(label).small().color(p.dim)).sense(Sense::click()))
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .clicked()
                {
                    app.show_secret = !app.show_secret;
                }
            });
        });
        let hint =
            if keeps_secret { "enregistré — laissez vide pour le conserver" } else { "32 caractères" };
        ui.add(
            egui::TextEdit::singleline(&mut app.setup_secret)
                .password(!app.show_secret)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .font(egui::TextStyle::Monospace)
                .margin(Margin::symmetric(10, 8)),
        );
        let id = app.setup_id.trim().to_string();
        let secret = app.setup_secret.trim().to_string();
        let id_ok = AppCredentials::looks_valid(&id);
        let secret_ok = AppCredentials::looks_valid(&secret) || (secret.is_empty() && keeps_secret);
        if !id.is_empty() && !id_ok {
            ui.label(
                RichText::new("Le Client ID fait 32 caractères (chiffres et lettres a à f).")
                    .small()
                    .color(p.dim),
            );
        }
        if !secret.is_empty() && !AppCredentials::looks_valid(&secret) {
            ui.label(
                RichText::new("Le Client Secret fait 32 caractères (chiffres et lettres a à f).")
                    .small()
                    .color(p.dim),
            );
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if status.state == AppState::Authorizing {
                ui.add(egui::Spinner::new().color(p.accent));
                ui.label(RichText::new("Acceptez l'accès dans votre navigateur…").color(p.text));
                if widgets::pill(ui, &p, "Annuler", false).clicked() {
                    app.send(Command::CancelAppLogin);
                }
            } else {
                let ready = id_ok && secret_ok;
                let response = ui.add_enabled_ui(ready, |ui| widgets::pill(ui, &p, "Connecter", true)).inner;
                if response.clicked() && ready {
                    if secret.is_empty() && keeps_secret {
                        app.send(Command::ReconnectApp);
                    } else {
                        app.send(Command::SetupApp(AppCredentials { client_id: id, client_secret: secret }));
                    }
                }
                if app.editing_app
                    && status.state == AppState::Connected
                    && widgets::pill(ui, &p, "Annuler", false).clicked()
                {
                    app.editing_app = false;
                }
            }
        });
    });
}

/// One numbered step of the guide.
fn step(ui: &mut Ui, p: &theme::Palette, number: u32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        ui.painter().circle_filled(rect.center(), 13.0, p.surface);
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            number.to_string(),
            theme::strong_font(13.0),
            p.text,
        );
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.add_space(4.0);
            add(ui);
        });
    });
    ui.add_space(12.0);
}

/// "1a2b…9f0e": enough to recognise an id without displaying it in full.
fn mask_id(id: &str) -> String {
    if id.len() > 8 { format!("{}…{}", &id[..4], &id[id.len() - 4..]) } else { id.to_string() }
}

// ----------------------------------------------------------------------------
// Toasts

pub fn toasts(app: &App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let p = app.palette;
    egui::Area::new(Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -(PLAYER_HEIGHT + 24.0)))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for toast in &app.toasts {
                Frame::new()
                    .fill(p.raised)
                    .stroke(if toast.error { Stroke::new(1.5, p.danger) } else { Stroke::new(1.0, p.line) })
                    .corner_radius(CornerRadius::same(RADIUS_CARD))
                    .inner_margin(Margin::symmetric(16, 11))
                    .show(ui, |ui| {
                        ui.set_max_width(380.0);
                        let color = if toast.error { p.danger } else { p.text };
                        ui.label(RichText::new(&toast.text).color(color));
                    });
                ui.add_space(8.0);
            }
        });
}

/// Every icon at several sizes (`SPOTILITE_DEMO_ICONS=1`, debug builds).
#[cfg(debug_assertions)]
pub fn icon_sheet(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    let icons = [
        Icon::Home,
        Icon::Play,
        Icon::Pause,
        Icon::Prev,
        Icon::Next,
        Icon::Shuffle,
        Icon::Repeat { one: false },
        Icon::Repeat { one: true },
        Icon::Heart { filled: false },
        Icon::Heart { filled: true },
        Icon::Volume { level: 0 },
        Icon::Volume { level: 1 },
        Icon::Volume { level: 2 },
        Icon::Queue,
        Icon::Back,
        Icon::Refresh,
        Icon::Search,
        Icon::Disc,
        Icon::Library,
        Icon::Artist,
        Icon::Settings,
    ];
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(16))).show(
        ui,
        |ui| {
            for size in [16.0_f32, 24.0, 36.0, 64.0] {
                ui.horizontal_wrapped(|ui| {
                    for icon in icons {
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(size.max(28.0), size.max(28.0)), Sense::hover());
                        let rect = Rect::from_center_size(rect.center(), vec2(size, size));
                        widgets::paint_icon(ui.painter(), rect, icon, p.text);
                    }
                });
                ui.add_space(10.0);
            }
        },
    );
}
