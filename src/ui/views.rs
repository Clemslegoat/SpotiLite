//! Screens and panels.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Layout, Margin, RichText, ScrollArea,
    Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use super::theme::{self, PLAYER_HEIGHT, ROW_HEIGHT, SIDEBAR_WIDTH};
use super::widgets::{self, Icon, format_duration, format_total, paint_text};
use super::{App, Auth, Page};
use crate::backend::{Command, PersonalApi, human_bytes};
use crate::config::{Quality, ThemeChoice};
use crate::model::{AlbumSummary, PlaylistSummary, Repeat, Track, ViewKey};

// ----------------------------------------------------------------------------
// Login

pub fn login_screen(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space((ui.available_height() * 0.28).max(24.0));
                ui.label(RichText::new("SpotiLite").font(theme::strong_font(34.0)).color(p.text));
                ui.add_space(4.0);
                ui.label(RichText::new("Spotify, sans le superflu.").color(p.dim));
                ui.add_space(28.0);
                match app.auth.clone() {
                    Auth::Unknown | Auth::Connecting => {
                        ui.add(egui::Spinner::new().color(p.accent).size(22.0));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Connexion à Spotify…").color(p.dim));
                    }
                    Auth::Pending { url } => {
                        ui.label(RichText::new("Terminez la connexion dans votre navigateur.").color(p.text));
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            let width = 260.0;
                            ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                            if widgets::pill(ui, &p, "Rouvrir la page", true).clicked() {
                                let _ = open::that_detached(&url);
                            }
                            if widgets::pill(ui, &p, "Annuler", false).clicked() {
                                app.send(Command::CancelLogin);
                                app.auth = Auth::NeedLogin;
                            }
                        });
                    }
                    Auth::NeedLogin | Auth::LoggedIn | Auth::Offline => {
                        if widgets::pill(ui, &p, "Se connecter avec Spotify", true).clicked() {
                            app.send(Command::Login);
                        }
                    }
                }
                ui.add_space(36.0);
                ui.label(
                    RichText::new(
                        "Compte Spotify Premium requis. La connexion se fait sur la page officielle de Spotify :\n\
                         aucun mot de passe ne transite par SpotiLite.",
                    )
                    .small()
                    .color(p.faint),
                );
            });
        });
}

// ----------------------------------------------------------------------------
// Main layout

pub fn main_layout(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    egui::Panel::bottom("player")
        .exact_size(PLAYER_HEIGHT)
        .resizable(false)
        .frame(
            Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.line))
                .inner_margin(Margin::symmetric(14, 8)),
        )
        .show(ui, |ui| player_bar(app, ui));
    egui::Panel::left("sidebar")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(Frame::new().fill(p.panel).inner_margin(Margin { left: 10, right: 10, top: 12, bottom: 8 }))
        .show(ui, |ui| sidebar(app, ui));
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 22, right: 18, top: 16, bottom: 0 }))
        .show(ui, |ui| content(app, ui));
}

fn sidebar(app: &mut App, ui: &mut Ui) {
    let p = app.palette;
    // Search field.
    let search = ui.add(
        egui::TextEdit::singleline(&mut app.search_text)
            .hint_text("Rechercher  (Ctrl+F)")
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(8, 6)),
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
    ui.add_space(10.0);

    nav_item(app, ui, "Titres likés", ViewKey::Liked);
    nav_item(app, ui, "Albums", ViewKey::SavedAlbums);
    let queue_label = if app.player.upcoming.is_empty() {
        "File d'attente".to_string()
    } else {
        format!("File d'attente · {}", app.player.upcoming.len().min(99))
    };
    nav_item(app, ui, &queue_label, ViewKey::Queue);

    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("PLAYLISTS").font(egui::FontId::proportional(11.0)).color(p.faint));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_button(ui, &p, Icon::Refresh, 22.0, false, false)
                .on_hover_text("Actualiser les playlists")
                .clicked()
            {
                app.send(Command::LoadPlaylists);
            }
        });
    });
    ui.add_space(2.0);

    let footer_height = 80.0;
    let list_height = (ui.available_height() - footer_height).max(40.0);
    let playlists = app.playlists.clone();
    ScrollArea::vertical()
        .id_salt("playlists")
        .max_height(list_height)
        .auto_shrink([false, false])
        .show_rows(ui, 26.0, playlists.len(), |ui, range| {
            for playlist in &playlists[range] {
                let view = ViewKey::Playlist(playlist.id.clone());
                nav_row(app, ui, &playlist.name, view, 26.0)
                    .on_hover_text(format!("{} · {} titres", playlist.owner, playlist.total));
            }
        });

    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let small = egui::FontId::proportional(11.0);
        if app.auth == Auth::Offline {
            ui.label(RichText::new("Hors ligne · reconnexion automatique").font(small.clone()).color(p.danger));
        }
        let data = format!("Données reçues : {}", human_bytes(app.usage.0 + app.usage.1));
        ui.label(RichText::new(data).font(small.clone()).color(p.faint)).on_hover_text(
            "Estimation pour cette session : interface et pochettes (mesuré) + audio (débit × durée écoutée).",
        );
        let ram = format!(
            "RAM {} · {} kbit/s",
            human_bytes(app.memory.private_working_set),
            app.settings.quality.kbps()
        );
        ui.label(RichText::new(ram).font(small).color(p.faint))
            .on_hover_text("Mémoire utilisée, telle qu'affichée par le Gestionnaire des tâches · qualité audio");
        ui.add_space(2.0);
        nav_item(app, ui, "Réglages", ViewKey::Settings);
    });
}

fn nav_item(app: &mut App, ui: &mut Ui, label: &str, view: ViewKey) {
    nav_row(app, ui, label, view, 30.0);
}

fn nav_row(app: &mut App, ui: &mut Ui, label: &str, view: ViewKey, height: f32) -> egui::Response {
    let p = app.palette;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let selected = app.view == view;
    let playing_here = false;
    if response.hovered() || selected {
        ui.painter().rect_filled(rect, CornerRadius::same(5), if selected { p.raised } else { p.hover });
    }
    if selected {
        let marker = egui::Rect::from_min_size(rect.min + vec2(0.0, 7.0), vec2(3.0, rect.height() - 14.0));
        ui.painter().rect_filled(marker, CornerRadius::same(2), p.accent);
    }
    let color = if selected || playing_here { p.text } else { p.dim };
    let font = if height > 28.0 { theme::strong_font(13.5) } else { theme::body_font() };
    paint_text(ui, pos2(rect.left() + 12.0, rect.center().y), label, font, color, rect.width() - 18.0);
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
        ViewKey::Welcome => return welcome_page(app, ui),
        _ => {}
    }
    let p = app.palette;
    if let Some(message) = app.failures.get(&view).cloned() {
        header(app, ui, "Oups", "", None);
        ui.add_space(20.0);
        ui.label(RichText::new(message).color(p.danger));
        ui.add_space(10.0);
        if widgets::pill(ui, &p, "Réessayer", true).clicked() {
            app.navigate_with(view, true);
        }
        return;
    }
    if !app.pages.contains_key(&view) {
        ui.add_space(40.0);
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().color(p.accent));
            ui.label(RichText::new("Chargement…").color(p.dim));
        });
        return;
    }
    let refreshing = app.loading.contains(&view);
    match app.pages.get(&view) {
        Some(Page::Tracks { title, subtitle, tracks }) => {
            let (title, subtitle, tracks) = (title.clone(), subtitle.clone(), tracks.clone());
            tracks_page(app, ui, &title, &subtitle, tracks, refreshing);
        }
        Some(Page::Albums { title, albums }) => {
            let (title, albums) = (title.clone(), albums.clone());
            header(app, ui, &title, &format!("{} albums", albums.len()), None);
            ui.add_space(8.0);
            album_list(app, ui, &albums, "albums");
        }
        Some(Page::Artist { name, top, albums }) => {
            let (name, top, albums) = (name.clone(), top.clone(), albums.clone());
            artist_page(app, ui, &name, top, &albums);
        }
        Some(Page::Search(results)) => {
            let results = results.clone();
            search_page(app, ui, &results);
        }
        None => {}
    }
}

fn welcome_page(app: &mut App, ui: &mut Ui) {
    header(
        app,
        ui,
        &format!("Bonjour {}", app.user),
        "Choisissez une playlist ou lancez une recherche.",
        None,
    );
}

/// Title block with a back button and optional refresh action.
fn header(app: &mut App, ui: &mut Ui, title: &str, subtitle: &str, refresh: Option<&ViewKey>) {
    let p = app.palette;
    ui.horizontal(|ui| {
        if !app.history.is_empty()
            && widgets::icon_button(ui, &p, Icon::Back, 28.0, false, false)
                .on_hover_text("Retour (Alt+←)")
                .clicked()
        {
            app.back();
        }
        let width = ui.available_width() - if refresh.is_some() { 40.0 } else { 0.0 };
        let galley = widgets::truncated(ui, title, theme::heading_font(), p.text, width);
        ui.label(galley);
        if let Some(view) = refresh {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let spinning = app.loading.contains(view);
                if spinning {
                    ui.add(egui::Spinner::new().color(p.accent).size(16.0));
                } else if widgets::icon_button(ui, &p, Icon::Refresh, 26.0, false, false)
                    .on_hover_text("Actualiser depuis Spotify")
                    .clicked()
                {
                    let view = view.clone();
                    app.navigate_with(view, true);
                }
            });
        }
    });
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).color(p.dim));
    }
}

fn tracks_page(
    app: &mut App,
    ui: &mut Ui,
    title: &str,
    subtitle: &str,
    tracks: Arc<Vec<Track>>,
    _refreshing: bool,
) {
    let p = app.palette;
    let view = app.view.clone();
    let total: u64 = tracks.iter().map(|t| u64::from(t.duration_ms)).sum();
    let mut info = format!("{} titres · {}", tracks.len(), format_total(total));
    if !subtitle.is_empty() {
        info = format!("{subtitle} · {info}");
    }
    header(app, ui, title, &info, Some(&view));
    ui.add_space(10.0);
    let visible = app.visible_tracks().unwrap_or_else(|| tracks.clone());
    ui.horizontal(|ui| {
        if widgets::pill(ui, &p, "Lecture", true).clicked() && !visible.is_empty() {
            if app.player.shuffle {
                app.send(Command::SetShuffle(false));
            }
            app.play(visible.clone(), 0);
        }
        if widgets::pill(ui, &p, "Aléatoire", false).clicked() && !visible.is_empty() {
            app.send(Command::SetShuffle(true));
            let start = rand::random_range(0..visible.len());
            app.play(visible.clone(), start);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.filter)
                    .hint_text("Filtrer")
                    .desired_width(180.0)
                    .margin(Margin::symmetric(8, 5)),
            );
        });
    });
    ui.add_space(8.0);
    track_table(app, ui, visible, "tracks");
}

fn artist_page(app: &mut App, ui: &mut Ui, name: &str, top: Arc<Vec<Track>>, albums: &[AlbumSummary]) {
    let p = app.palette;
    let view = app.view.clone();
    header(app, ui, name, "Artiste", Some(&view));
    ui.add_space(8.0);
    ScrollArea::vertical().id_salt("artist").auto_shrink([false, false]).show(ui, |ui| {
        if !top.is_empty() {
            section_title(ui, &p, "Populaires");
            track_rows_plain(app, ui, top.clone(), "artist-top");
            ui.add_space(14.0);
        }
        section_title(ui, &p, "Albums et singles");
        album_rows_plain(app, ui, albums, "artist-albums");
    });
}

fn search_page(app: &mut App, ui: &mut Ui, results: &crate::model::SearchResults) {
    let p = app.palette;
    header(app, ui, &format!("« {} »", results.query), "Résultats de recherche", None);
    ui.add_space(8.0);
    ScrollArea::vertical().id_salt("search").auto_shrink([false, false]).show(ui, |ui| {
        let empty = results.tracks.is_empty()
            && results.albums.is_empty()
            && results.artists.is_empty()
            && results.playlists.is_empty();
        if empty {
            ui.label(RichText::new("Aucun résultat.").color(p.dim));
        }
        if !results.tracks.is_empty() {
            section_title(ui, &p, "Titres");
            track_rows_plain(app, ui, results.tracks.clone(), "search-tracks");
            ui.add_space(14.0);
        }
        if !results.artists.is_empty() {
            section_title(ui, &p, "Artistes");
            ui.horizontal_wrapped(|ui| {
                for artist in &results.artists {
                    if widgets::pill(ui, &p, &artist.name, false).clicked() {
                        app.navigate(ViewKey::Artist(artist.id.clone()));
                    }
                }
            });
            ui.add_space(14.0);
        }
        if !results.albums.is_empty() {
            section_title(ui, &p, "Albums");
            album_rows_plain(app, ui, &results.albums, "search-albums");
            ui.add_space(14.0);
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
    ui.add_space(8.0);
    if let Some(now) = app.player.now.clone() {
        section_title(ui, &p, "En cours");
        track_rows_plain(app, ui, Arc::new(vec![now]), "queue-now");
        ui.add_space(12.0);
    }
    ui.horizontal(|ui| {
        section_title(ui, &p, "À suivre");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if !app.player.upcoming.is_empty() && widgets::pill(ui, &p, "Vider les ajouts", false).clicked() {
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
    ui.label(RichText::new(text).font(theme::strong_font(15.0)).color(p.text));
    ui.add_space(4.0);
}

// ----------------------------------------------------------------------------
// Track rows

struct Columns {
    index: f32,
    title: f32,
    artist: f32,
    album: f32,
}

fn columns(width: f32) -> Columns {
    let index = 36.0;
    let duration = 52.0;
    let rest = (width - index - duration - 12.0).max(80.0);
    if width > 720.0 {
        Columns { index, title: rest * 0.42, artist: rest * 0.29, album: rest * 0.29 }
    } else {
        Columns { index, title: rest * 0.6, artist: rest * 0.4, album: 0.0 }
    }
}

/// Virtualized table: only the visible rows are laid out, so a 10 000 track list
/// costs the same as a 20 track one.
fn track_table(app: &mut App, ui: &mut Ui, tracks: Arc<Vec<Track>>, salt: &str) {
    let p = app.palette;
    let width = ui.available_width();
    let cols = columns(width);
    // Column header.
    let (rect, _) = ui.allocate_exact_size(vec2(width, 22.0), Sense::hover());
    let font = egui::FontId::proportional(11.0);
    let y = rect.center().y;
    let mut x = rect.left() + 10.0;
    paint_text(ui, pos2(x, y), "#", font.clone(), p.faint, cols.index);
    x = rect.left() + cols.index;
    paint_text(ui, pos2(x, y), "TITRE", font.clone(), p.faint, cols.title);
    x += cols.title;
    paint_text(ui, pos2(x, y), "ARTISTE", font.clone(), p.faint, cols.artist);
    x += cols.artist;
    if cols.album > 0.0 {
        paint_text(ui, pos2(x, y), "ALBUM", font.clone(), p.faint, cols.album);
    }
    ui.painter().line_segment(
        [pos2(rect.left(), rect.bottom()), pos2(rect.right(), rect.bottom())],
        Stroke::new(1.0, p.line),
    );

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
    if hovered || selected {
        ui.painter().rect_filled(rect, CornerRadius::same(4), if selected { p.raised } else { p.hover });
    }
    let y = rect.center().y;
    let text_color = if !track.playable {
        p.faint
    } else if is_current {
        p.accent
    } else {
        p.text
    };
    let dim = if track.playable { p.dim } else { p.faint };

    // Index / state column.
    let index_rect = egui::Rect::from_min_size(rect.min, vec2(cols.index, rect.height()));
    if hovered && track.playable {
        widgets::paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(index_rect.center(), vec2(14.0, 14.0)),
            Icon::Play,
            p.text,
        );
    } else if is_current && app.player.playing {
        equalizer(ui, index_rect.center(), p.accent);
    } else {
        let label = (index + 1).to_string();
        ui.painter().text(
            index_rect.center(),
            Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            p.faint,
        );
    }

    let mut x = rect.left() + cols.index;
    paint_text(ui, pos2(x, y), &track.name, theme::body_font(), text_color, cols.title - 12.0);
    x += cols.title;

    // Artist and album are links.
    let artist_text = track.artists_joined();
    let artist_rect = paint_text(ui, pos2(x, y), &artist_text, theme::body_font(), dim, cols.artist - 12.0);
    let artist_id = track.artists.first().map(|a| a.id.clone()).filter(|id| !id.is_empty());
    let mut link_clicked = false;
    if let Some(id) = artist_id
        && widgets::text_link(ui, Id::new((salt, "artist", index)), artist_rect, &p).clicked()
    {
        app.navigate(ViewKey::Artist(id));
        link_clicked = true;
    }
    x += cols.artist;
    if cols.album > 0.0 {
        let album_rect = paint_text(ui, pos2(x, y), &track.album, theme::body_font(), dim, cols.album - 12.0);
        if !track.album_id.is_empty()
            && widgets::text_link(ui, Id::new((salt, "album", index)), album_rect, &p).clicked()
        {
            app.navigate(ViewKey::Album(track.album_id.clone()));
            link_clicked = true;
        }
    }
    ui.painter().text(
        pos2(rect.right() - 10.0, y),
        Align2::RIGHT_CENTER,
        format_duration(track.duration_ms),
        egui::FontId::proportional(12.5),
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
        ui.set_min_width(190.0);
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
    for (i, h) in [8.0, 12.0, 6.0].iter().enumerate() {
        let x = center.x - 5.0 + i as f32 * 5.0;
        let bar = egui::Rect::from_min_max(pos2(x - 1.5, center.y + 6.0 - h), pos2(x + 1.5, center.y + 6.0));
        ui.painter().rect_filled(bar, CornerRadius::same(1), color);
    }
}

fn album_list(app: &mut App, ui: &mut Ui, albums: &[AlbumSummary], salt: &str) {
    let albums = albums.to_vec();
    ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]).show_rows(
        ui,
        ROW_HEIGHT + 4.0,
        albums.len(),
        |ui, range| {
            for album in &albums[range] {
                album_row(app, ui, album);
            }
        },
    );
}

fn album_rows_plain(app: &mut App, ui: &mut Ui, albums: &[AlbumSummary], _salt: &str) {
    for album in albums {
        album_row(app, ui, album);
    }
}

fn album_row(app: &mut App, ui: &mut Ui, album: &AlbumSummary) {
    let p = app.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 4.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
    }
    let y = rect.center().y;
    let name_width = rect.width() * 0.5;
    paint_text(ui, pos2(rect.left() + 10.0, y), &album.name, theme::body_font(), p.text, name_width - 16.0);
    let mut detail = album.artists.clone();
    if !album.year.is_empty() {
        detail.push_str(&format!(" · {}", album.year));
    }
    if album.total_tracks > 0 {
        detail.push_str(&format!(" · {} titres", album.total_tracks));
    }
    paint_text(
        ui,
        pos2(rect.left() + name_width, y),
        &detail,
        theme::body_font(),
        p.dim,
        rect.width() - name_width - 10.0,
    );
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        app.navigate(ViewKey::Album(album.id.clone()));
    }
}

fn playlist_row(app: &mut App, ui: &mut Ui, playlist: &PlaylistSummary) {
    let p = app.palette;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 4.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
    }
    let y = rect.center().y;
    let name_width = rect.width() * 0.5;
    paint_text(
        ui,
        pos2(rect.left() + 10.0, y),
        &playlist.name,
        theme::body_font(),
        p.text,
        name_width - 16.0,
    );
    let detail = format!("{} · {} titres", playlist.owner, playlist.total);
    paint_text(
        ui,
        pos2(rect.left() + name_width, y),
        &detail,
        theme::body_font(),
        p.dim,
        rect.width() - name_width - 10.0,
    );
    if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
        app.navigate(ViewKey::Playlist(playlist.id.clone()));
    }
}

// ----------------------------------------------------------------------------
// Player bar

fn player_bar(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let left_w = (full.width() * 0.3).clamp(170.0, 340.0);
    let right_w = (full.width() * 0.22).clamp(140.0, 230.0);
    let left = egui::Rect::from_min_size(full.min, vec2(left_w, full.height()));
    let right = egui::Rect::from_min_max(pos2(full.right() - right_w, full.top()), full.max);
    let center = egui::Rect::from_min_max(pos2(left.right(), full.top()), pos2(right.left(), full.bottom()));
    ui.scope_builder(UiBuilder::new().max_rect(left), |ui| now_playing(app, ui, left));
    ui.scope_builder(UiBuilder::new().max_rect(center), |ui| controls(app, ui, center));
    ui.scope_builder(UiBuilder::new().max_rect(right).layout(Layout::right_to_left(Align::Center)), |ui| {
        volume_and_queue(app, ui, right_w)
    });
}

fn volume_and_queue(app: &mut App, ui: &mut Ui, width: f32) {
    let p = app.palette;
    let (response, committed, _) =
        widgets::bar(ui, &p, (width - 76.0).clamp(60.0, 120.0), app.settings.volume, true);
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
    if widgets::icon_button(ui, &p, Icon::Volume { level }, 28.0, false, false)
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
    if widgets::icon_button(ui, &p, Icon::Queue, 28.0, in_queue, false)
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

fn now_playing(app: &mut App, ui: &mut Ui, rect: egui::Rect) {
    let p = app.palette;
    let cy = rect.center().y;
    let Some(track) = app.player.now.clone() else {
        paint_text(ui, pos2(rect.left(), cy), "Rien en lecture", theme::body_font(), p.faint, rect.width());
        return;
    };
    let mut x = rect.left();
    if app.settings.show_covers {
        let cover = egui::Rect::from_min_size(pos2(x, cy - 24.0), vec2(48.0, 48.0));
        match app.cover(track.image.as_ref()) {
            Some(texture) => {
                let uv = egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                ui.painter().image(texture, cover, uv, Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(cover, CornerRadius::same(3), p.raised);
            }
        }
        x += 58.0;
    }
    let text_w = (rect.right() - x - 36.0).max(40.0);
    let title = paint_text(ui, pos2(x, cy - 9.0), &track.name, theme::strong_font(14.0), p.text, text_w);
    let artists =
        paint_text(ui, pos2(x, cy + 10.0), &track.artists_joined(), theme::small_font(), p.dim, text_w);
    if !track.album_id.is_empty() && widgets::text_link(ui, Id::new("np-title"), title, &p).clicked() {
        app.navigate(ViewKey::Album(track.album_id.clone()));
    }
    if let Some(artist) = track.artists.first().filter(|a| !a.id.is_empty())
        && widgets::text_link(ui, Id::new("np-artist"), artists, &p).clicked()
    {
        app.navigate(ViewKey::Artist(artist.id.clone()));
    }
    let heart_x = (x + title.width().max(artists.width()) + 20.0).min(rect.right() - 14.0);
    let heart_rect = egui::Rect::from_center_size(pos2(heart_x, cy), vec2(28.0, 28.0));
    let liked = app.player.is_liked();
    let clicked = ui
        .scope_builder(UiBuilder::new().max_rect(heart_rect), |ui| {
            widgets::icon_button(
                ui,
                &p,
                Icon::Heart { filled: liked == Some(true) },
                28.0,
                liked == Some(true),
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

fn controls(app: &mut App, ui: &mut Ui, rect: egui::Rect) {
    let p = app.palette;
    let has_track = app.player.now.is_some();
    // Buttons row: toggles get a fixed width so the row never shifts.
    let toggle_w = 64.0;
    let row_w = toggle_w * 2.0 + 32.0 * 2.0 + 36.0 + 4.0 * ui.spacing().item_spacing.x;
    let row = egui::Rect::from_center_size(pos2(rect.center().x, rect.top() + 18.0), vec2(row_w, 36.0));
    ui.scope_builder(UiBuilder::new().max_rect(row).layout(Layout::left_to_right(Align::Center)), |ui| {
        if widgets::text_toggle(ui, &p, "ALÉA", app.player.shuffle, toggle_w)
            .on_hover_text("Lecture aléatoire")
            .clicked()
        {
            app.player.shuffle = !app.player.shuffle;
            app.settings.shuffle = app.player.shuffle;
            app.send(Command::SetShuffle(app.player.shuffle));
        }
        if widgets::icon_button(ui, &p, Icon::Prev, 32.0, false, false)
            .on_hover_text("Précédent (Ctrl+←)")
            .clicked()
        {
            app.send(Command::Previous);
        }
        let icon = if app.player.playing { Icon::Pause } else { Icon::Play };
        if widgets::icon_button(ui, &p, icon, 36.0, false, true)
            .on_hover_text("Lecture / pause (Espace)")
            .clicked()
            && has_track
        {
            app.send(Command::PlayPause);
        }
        if widgets::icon_button(ui, &p, Icon::Next, 32.0, false, false)
            .on_hover_text("Suivant (Ctrl+→)")
            .clicked()
        {
            app.send(Command::Next);
        }
        let (label, on) = match app.player.repeat {
            Repeat::Off => ("BOUCLE", false),
            Repeat::All => ("BOUCLE", true),
            Repeat::One => ("BOUCLE 1", true),
        };
        if widgets::text_toggle(ui, &p, label, on, toggle_w)
            .on_hover_text("Répéter : non / tout / ce titre")
            .clicked()
        {
            let next = app.player.repeat.cycle();
            app.player.repeat = next;
            app.settings.repeat = next;
            app.send(Command::SetRepeat(next));
        }
    });

    // Progress row.
    let duration = app.player.now.as_ref().map_or(0, |t| t.duration_ms);
    let position = app.player.position();
    let bar_w = (rect.width() - 2.0 * 56.0).clamp(80.0, 520.0);
    let cy = rect.bottom() - 9.0;
    let bar_rect = egui::Rect::from_center_size(pos2(rect.center().x, cy), vec2(bar_w, 16.0));
    let fraction = if duration > 0 { position as f32 / duration as f32 } else { 0.0 };
    let (committed, shown) = ui
        .scope_builder(UiBuilder::new().max_rect(bar_rect), |ui| {
            let (_, committed, shown) = widgets::bar(ui, &p, bar_w, fraction, has_track && duration > 0);
            (committed, shown)
        })
        .inner;
    // While dragging, the left clock follows the pointer.
    let label = if app.player.buffering && app.player.playing {
        "…".to_string()
    } else {
        format_duration((shown * duration as f32) as u32)
    };
    let font = egui::FontId::proportional(11.5);
    ui.painter().text(pos2(bar_rect.left() - 10.0, cy), Align2::RIGHT_CENTER, label, font.clone(), p.dim);
    ui.painter().text(
        pos2(bar_rect.right() + 10.0, cy),
        Align2::LEFT_CENTER,
        format_duration(duration),
        font,
        p.dim,
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
    ui.add_space(6.0);
    let mut changed = false;
    ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(640.0);

        card(ui, &p, "Économie de données", |ui| {
            ui.label(RichText::new("Qualité audio").color(p.text));
            for quality in Quality::ALL {
                let text = format!(
                    "{} · {} kbit/s · ≈ {} Mo par heure",
                    quality.label(),
                    quality.kbps(),
                    quality.mb_per_hour()
                );
                changed |= ui.radio_value(&mut app.settings.quality, quality, text).changed();
            }
            ui.add_space(8.0);
            changed |= ui
                .checkbox(&mut app.settings.show_covers, "Afficher les pochettes (vignettes 64 px, gardées en cache)")
                .changed();
            ui.add_space(8.0);
            ui.label(RichText::new("Cache audio : un titre déjà écouté ne consomme plus aucune donnée").color(p.text));
            ui.horizontal_wrapped(|ui| {
                for (mb, label) in [(0, "Désactivé"), (512, "512 Mo"), (1024, "1 Go"), (2048, "2 Go"), (4096, "4 Go")] {
                    changed |= ui.radio_value(&mut app.settings.audio_cache_mb, mb, label).changed();
                }
            });
            ui.label(RichText::new("La taille du cache est appliquée au prochain démarrage.").small().color(p.faint));
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!(
                    "Cette session : {} pour l'interface et les pochettes (mesuré), ≈ {} pour l'audio (estimation haute : un titre relu depuis le cache ne consomme rien).",
                    human_bytes(app.usage.0),
                    human_bytes(app.usage.1)
                ))
                .color(p.dim),
            );
            ui.add_space(6.0);
            if widgets::pill(ui, &p, "Vider le cache", false).clicked() {
                app.send(Command::ClearCache);
            }
        });

        card(ui, &p, "Lecture", |ui| {
            changed |= ui
                .checkbox(&mut app.settings.normalisation, "Normaliser le volume entre les titres")
                .changed();
        });

        card(ui, &p, "Apparence", |ui| {
            ui.horizontal(|ui| {
                changed |= ui.radio_value(&mut app.settings.theme, ThemeChoice::Dark, "Sombre").changed();
                changed |= ui.radio_value(&mut app.settings.theme, ThemeChoice::Light, "Clair").changed();
            });
            ui.horizontal(|ui| {
                ui.label("Taille du texte");
                for (scale, label) in [(0.9, "90 %"), (1.0, "100 %"), (1.15, "115 %"), (1.3, "130 %")] {
                    changed |= ui.radio_value(&mut app.settings.ui_scale, scale, label).changed();
                }
            });
        });

        card(ui, &p, "Mémoire", |ui| {
            ui.label(
                RichText::new(format!(
                    "Utilisation actuelle : {} (ensemble de travail privé, la valeur du Gestionnaire des tâches), {} en comptant les bibliothèques partagées du système.",
                    human_bytes(app.memory.private_working_set),
                    human_bytes(app.memory.working_set)
                ))
                .color(p.dim),
            );
            changed |= ui
                .checkbox(&mut app.settings.trim_when_minimized, "Libérer la mémoire quand la fenêtre est réduite")
                .changed();
        });

        card(ui, &p, "API Web Spotify (facultatif)", |ui| {
            ui.label(
                RichText::new(
                    "Par défaut, SpotiLite utilise le jeton de votre session. Spotify limite parfois ce jeton partagé \
                     (erreur « limite les requêtes »). Pour un accès dédié, créez une application gratuite sur \
                     developer.spotify.com, ajoutez l'URI de redirection ci-dessous puis collez son Client ID.",
                )
                .color(p.dim),
            );
            ui.hyperlink_to("Ouvrir le tableau de bord développeur", "https://developer.spotify.com/dashboard");
            ui.add_space(6.0);
            egui::Grid::new("api-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Client ID");
                ui.add(egui::TextEdit::singleline(&mut app.client_id_draft).desired_width(300.0).hint_text("32 caractères"));
                ui.end_row();
                ui.label("Port de redirection");
                ui.add(egui::TextEdit::singleline(&mut app.port_draft).desired_width(80.0));
                ui.end_row();
                ui.label("URI à déclarer");
                let port: u16 = app.port_draft.trim().parse().unwrap_or(app.settings.redirect_port);
                ui.label(RichText::new(format!("http://127.0.0.1:{port}/login")).monospace());
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let draft_id = app.client_id_draft.trim().to_string();
                let draft_port = app.port_draft.trim().parse::<u16>().ok().filter(|p| *p > 0);
                let dirty = draft_id != app.settings.client_id || draft_port.is_some_and(|port| port != app.settings.redirect_port);
                if dirty && widgets::pill(ui, &p, "Enregistrer", true).clicked() {
                    app.settings.client_id = draft_id;
                    if let Some(port) = draft_port {
                        app.settings.redirect_port = port;
                    }
                    changed = true;
                }
                if !dirty && !app.settings.client_id.is_empty() {
                    match app.personal {
                        PersonalApi::Connected => {
                            ui.label(RichText::new("● Connectée").color(p.accent));
                            if widgets::pill(ui, &p, "Déconnecter", false).clicked() {
                                app.send(Command::DisconnectPersonalApi);
                            }
                        }
                        _ => {
                            if widgets::pill(ui, &p, "Autoriser l'application", true).clicked() {
                                app.send(Command::ConnectPersonalApi);
                            }
                        }
                    }
                }
            });
        });

        card(ui, &p, "Compte", |ui| {
            ui.label(RichText::new(format!("Connecté : {}", app.user)).color(p.text));
            ui.add_space(4.0);
            if widgets::pill(ui, &p, "Se déconnecter", false).clicked() {
                app.send(Command::Logout);
            }
            ui.label(RichText::new("La déconnexion efface aussi les données en cache.").small().color(p.faint));
        });

        card(ui, &p, "À propos", |ui| {
            ui.label(RichText::new(format!("SpotiLite {}", env!("CARGO_PKG_VERSION"))).color(p.text));
            ui.label(RichText::new(format!("Réglages : {}", app.paths.config.display())).small().color(p.dim));
            ui.label(RichText::new(format!("Cache : {}", app.paths.cache.display())).small().color(p.dim));
            ui.label(
                RichText::new("Raccourcis : Espace lecture/pause · Ctrl+←/→ titre précédent/suivant · Ctrl+↑/↓ volume · Ctrl+F recherche · Ctrl+L j'aime · Alt+← retour · ↑/↓ + Entrée dans les listes")
                    .small()
                    .color(p.faint),
            );
        });
        ui.add_space(16.0);
    });
    if changed {
        app.apply_settings(&ctx);
    }
}

fn card(ui: &mut Ui, p: &theme::Palette, title: &str, add: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(p.panel)
        .stroke(Stroke::new(1.0, p.line))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::strong_font(15.0)).color(p.text));
            ui.add_space(6.0);
            add(ui);
        });
    ui.add_space(10.0);
}

// ----------------------------------------------------------------------------
// Toasts

pub fn toasts(app: &App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let p = app.palette;
    egui::Area::new(Id::new("toasts"))
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -(PLAYER_HEIGHT + 12.0)))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            for toast in &app.toasts {
                Frame::new()
                    .fill(p.raised)
                    .stroke(Stroke::new(1.0, if toast.error { p.danger } else { p.line }))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        let color = if toast.error { p.danger } else { p.text };
                        ui.label(RichText::new(&toast.text).color(color));
                    });
                ui.add_space(6.0);
            }
        });
    let _ = StrokeKind::Inside;
}
