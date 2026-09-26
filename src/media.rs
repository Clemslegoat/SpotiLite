//! Windows media integration: keyboard media keys, the volume flyout and the lock
//! screen show and control SpotiLite through the System Media Transport Controls.
//! On other systems this is a no-op.

#[cfg(not(windows))]
use crate::model::Track;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum MediaAction {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
    Seek(u32),
    Raise,
}

#[cfg(windows)]
pub use windows_impl::MediaKeys;

#[cfg(not(windows))]
pub struct MediaKeys;

#[cfg(not(windows))]
impl MediaKeys {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self
    }

    pub fn poll(&mut self) -> Vec<MediaAction> {
        Vec::new()
    }

    pub fn set_metadata(&mut self, _track: Option<&Track>, _cover_file: Option<String>) {}

    pub fn set_playback(&mut self, _playing: bool, _position_ms: u32) {}
}

#[cfg(windows)]
mod windows_impl {
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use souvlaki::{
        MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
        SeekDirection,
    };

    use super::MediaAction;
    use crate::model::Track;

    pub struct MediaKeys {
        controls: Option<MediaControls>,
        rx: Receiver<MediaAction>,
        shown: Option<(String, Option<String>)>,
        position_ms: u32,
    }

    impl MediaKeys {
        pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
            let (tx, rx) = channel();
            let hwnd = cc.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(w) => Some(w.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            });
            let controls = hwnd.and_then(|hwnd| {
                let config =
                    PlatformConfig { display_name: "SpotiLite", dbus_name: "spotilite", hwnd: Some(hwnd) };
                let mut controls = MediaControls::new(config).ok()?;
                let ctx = cc.egui_ctx.clone();
                controls
                    .attach(move |event| {
                        let action = match event {
                            MediaControlEvent::Toggle => Some(MediaAction::Toggle),
                            MediaControlEvent::Play => Some(MediaAction::Play),
                            MediaControlEvent::Pause | MediaControlEvent::Stop => Some(MediaAction::Pause),
                            MediaControlEvent::Next => Some(MediaAction::Next),
                            MediaControlEvent::Previous => Some(MediaAction::Previous),
                            MediaControlEvent::SetPosition(MediaPosition(d)) => {
                                Some(MediaAction::Seek(d.as_millis() as u32))
                            }
                            MediaControlEvent::Seek(SeekDirection::Forward) => Some(MediaAction::Next),
                            MediaControlEvent::Seek(SeekDirection::Backward) => Some(MediaAction::Previous),
                            MediaControlEvent::Raise => Some(MediaAction::Raise),
                            _ => None,
                        };
                        if let Some(action) = action {
                            let _ = tx.send(action);
                            ctx.request_repaint();
                        }
                    })
                    .ok()?;
                Some(controls)
            });
            if controls.is_none() {
                log::info!("system media controls unavailable");
            }
            Self { controls, rx, shown: None, position_ms: 0 }
        }

        pub fn poll(&mut self) -> Vec<MediaAction> {
            self.rx.try_iter().collect()
        }

        pub fn set_metadata(&mut self, track: Option<&Track>, cover_file: Option<String>) {
            let Some(controls) = self.controls.as_mut() else { return };
            let Some(track) = track else {
                let _ = controls.set_playback(MediaPlayback::Stopped);
                self.shown = None;
                return;
            };
            let key = (track.id.clone(), cover_file.clone());
            if self.shown.as_ref() == Some(&key) {
                return;
            }
            let artists = track.artists_joined();
            // Only a local file is ever given as cover: Windows must not download
            // the image a second time.
            let cover_url = cover_file.map(|path| format!("file://{path}"));
            let _ = controls.set_metadata(MediaMetadata {
                title: Some(&track.name),
                artist: Some(&artists),
                album: Some(&track.album),
                cover_url: cover_url.as_deref(),
                duration: Some(Duration::from_millis(u64::from(track.duration_ms))),
            });
            self.shown = Some(key);
        }

        pub fn set_playback(&mut self, playing: bool, position_ms: u32) {
            let Some(controls) = self.controls.as_mut() else { return };
            self.position_ms = position_ms;
            let progress = Some(MediaPosition(Duration::from_millis(u64::from(position_ms))));
            let state = if playing {
                MediaPlayback::Playing { progress }
            } else {
                MediaPlayback::Paused { progress }
            };
            let _ = controls.set_playback(state);
        }
    }
}
