use std::cmp::Ordering;
use std::sync::{Arc, RwLock};

use librespot_protocol::spirc::TrackRef;
use log::info;
#[cfg(feature = "notify")]
use notify_rust::Notification;
use strum_macros::Display;

use crate::config::{Config, PlaybackState};
use crate::library::Library;
use crate::model::playable::Playable;
use crate::spotify::PlayerEvent;
use crate::spotify::Spotify;

/// Repeat behavior for the [Queue].
#[derive(Display, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum RepeatSetting {
    #[serde(rename = "off")]
    None,
    #[serde(rename = "playlist")]
    RepeatPlaylist,
    #[serde(rename = "track")]
    RepeatTrack,
}

/// The queue determines the playback order of
/// [Playable] items, and is also used to
/// control playback itself.
pub struct Queue {
    /// The internal data, which doesn't change with shuffle or repeat. This is
    /// the raw data only.
    pub queue: Arc<RwLock<Vec<Playable>>>,
    // /// The playback order of the queue, as indices into `self.queue`.
    // random_order: RwLock<Option<Vec<usize>>>,
    context: String,
    current_track: RwLock<Option<usize>>,
    spotify: Spotify,
    cfg: Arc<Config>,
    library: Arc<Library>,
}

impl Queue {
    pub fn new(spotify: Spotify, cfg: Arc<Config>, library: Arc<Library>) -> Queue {
        let queue_state = cfg.state().queuestate.clone();
        let playback_state = cfg.state().playback_state.clone();

        let tracks: Vec<TrackRef> = queue_state
            .queue
            .iter()
            .map(Playable::as_trackref)
            .collect();

        let queue = Queue {
            queue: Arc::new(RwLock::new(queue_state.queue)),
            context: queue_state.context.clone(),
            spotify: spotify.clone(),
            current_track: RwLock::new(queue_state.current_track),
            cfg,
            library,
        };

        if let Some(index) = queue.get_current_index() {
            spotify.load(
                queue.get_context(),
                tracks,
                index,
                playback_state == PlaybackState::Playing,
                queue_state.track_progress.as_millis() as u32,
                queue.get_shuffle(),
                !matches!(queue.get_repeat(), RepeatSetting::None),
            );
        }

        queue
    }

    fn queue_as_tracks(&self) -> Vec<TrackRef> {
        self.queue
            .read()
            .unwrap()
            .iter()
            .map(Playable::as_trackref)
            .collect()
    }

    /// The index of the next item in `self.queue` that should be played. None
    /// if at the end of the queue.
    pub fn next_index(&self) -> Option<usize> {
        let index = (*self.current_track.read().ok()?)?;

        let next_index = index + 1;
        if next_index < self.queue.read().ok()?.len() {
            Some(next_index)
        } else {
            None
        }
    }

    /// The index of the previous item in `self.queue` that should be played.
    /// None if at the start of the queue.
    pub fn previous_index(&self) -> Option<usize> {
        let index = (*self.current_track.read().ok()?)?;

        if index > 0 {
            Some(index - 1)
        } else {
            None
        }
    }

    pub fn get_context(&self) -> String {
        self.context.clone()
    }

    /// The currently playing item from `self.queue`.
    pub fn get_current(&self) -> Option<Playable> {
        self.get_current_index()
            .map(|index| self.queue.read().unwrap()[index].clone())
    }

    /// The index of the currently playing item from `self.queue`.
    pub fn get_current_index(&self) -> Option<usize> {
        *self.current_track.read().unwrap()
    }

    /// Insert `track` as the item that should logically follow the currently
    /// playing item, taking into account shuffle status.
    pub fn insert_after_current(&self, track: Playable) {
        if let Some(index) = self.get_current_index() {
            let mut q = self.queue.write().unwrap();
            q.insert(index + 1, track);
        } else {
            self.append(track);
        }
    }

    /// Add `track` to the end of the queue.
    pub fn append(&self, track: Playable) {
        self.spotify.api.queue(&track, None);

        let mut q = self.queue.write().unwrap();
        q.push(track);
    }

    /// Append `tracks` after the currently playing item, taking into account
    /// shuffle status. Returns the amount of added items.
    pub fn append_next(&self, tracks: &Vec<Playable>) -> usize {
        let mut q = self.queue.write().unwrap();
        let first = match *self.current_track.read().unwrap() {
            Some(index) => index + 1,
            None => q.len(),
        };

        let mut i = first;
        for track in tracks {
            q.insert(i, track.clone());
            i += 1;
        }

        first
    }

    /// Remove the item at `index`. This doesn't take into account shuffle
    /// status, and will literally remove the item at `index` in `self.queue`.
    pub fn remove(&self, index: usize) {
        {
            let mut q = self.queue.write().unwrap();
            if q.len() == 0 {
                info!("queue is empty");
                return;
            }
            q.remove(index);
        }

        // if the queue is empty return
        let len = self.queue.read().unwrap().len();
        if len == 0 {
            return;
        }

        // if we are deleting the currently playing track, play the track with
        // the same index again, because the next track is now at the position
        // of the one we deleted
        let current = *self.current_track.read().unwrap();
        if let Some(current_track) = current {
            match current_track.cmp(&index) {
                Ordering::Equal => {
                    // if we have deleted the last item and it was playing
                    // stop playback, unless repeat playlist is on, play next
                    if current_track == len {
                        if self.get_repeat() == RepeatSetting::RepeatPlaylist {
                            self.next(false);
                        }
                    } else {
                        self.play(index, false);
                    }
                }
                Ordering::Greater => {
                    let mut current = self.current_track.write().unwrap();
                    current.replace(current_track - 1);
                }
                _ => (),
            }
        }
    }

    /// Clear all the items from the queue and stop playback.
    pub fn clear(&self) {
        self.stop();

        let mut q = self.queue.write().unwrap();
        q.clear();
    }

    /// The amount of items in `self.queue`.
    pub fn len(&self) -> usize {
        self.queue.read().unwrap().len()
    }

    /// Shift the item at `from` in `self.queue` to `to`.
    pub fn shift(&self, from: usize, to: usize) {
        let mut queue = self.queue.write().unwrap();
        let item = queue.remove(from);
        queue.insert(to, item);

        // if the currently playing track is affected by the shift, update its
        // index
        let mut current = self.current_track.write().unwrap();
        if let Some(index) = *current {
            if index == from {
                current.replace(to);
            } else if index == to && from > index {
                current.replace(to + 1);
            } else if index == to && from < index {
                current.replace(to - 1);
            }
        }
    }

    /// Play the item at `index` in `self.queue`.
    ///
    /// `shuffle`: Reshuffle the current order of the queue.
    /// chosen at random as a valid index in the queue.
    pub fn play(&self, index: usize, shuffle: bool) {
        if let Some(track) = &self.queue.read().unwrap().get(index) {
            let tracks = self.queue_as_tracks();
            let repeat = !matches!(self.get_repeat(), RepeatSetting::None);
            self.spotify.load(self.context.clone(), tracks, index, true, 0, shuffle, repeat);
            
            let mut current = self.current_track.write().unwrap();
            current.replace(index);
            self.spotify.update_track();

            #[cfg(feature = "notify")]
            if self.cfg.values().notify.unwrap_or(false) {
                std::thread::spawn({
                    // use same parser as track_format, Playable::format
                    let format = self
                        .cfg
                        .values()
                        .notification_format
                        .clone()
                        .unwrap_or_default();
                    let default_title = crate::config::NotificationFormat::default().title.unwrap();
                    let title = format.title.unwrap_or_else(|| default_title.clone());

                    let default_body = crate::config::NotificationFormat::default().body.unwrap();
                    let body = format.body.unwrap_or_else(|| default_body.clone());

                    let summary_txt = Playable::format(track, &title, &self.library);
                    let body_txt = Playable::format(track, &body, &self.library);
                    let cover_url = track.cover_url();
                    move || send_notification(&summary_txt, &body_txt, cover_url)
                });
            }
        }
    }

    /// Toggle the playback. If playback is currently stopped, this will either
    /// play the next song if one is available, or restart from the start.
    pub fn toggleplayback(&self) {
        match self.spotify.get_current_status() {
            PlayerEvent::Playing(_) | PlayerEvent::Paused(_) => {
                self.spotify.toggleplayback();
            }
            PlayerEvent::Stopped => match self.next_index() {
                Some(_) => self.next(false),
                None => self.play(0, false),
            },
            _ => (),
        }
    }

    /// Stop playback.
    pub fn stop(&self) {
        let mut current = self.current_track.write().unwrap();
        *current = None;
        self.spotify.stop();
    }

    /// Play the next song in the queue.
    ///
    /// `manual`: If this is true, normal queue logic like repeat will not be
    /// used, and the next track will actually be played. This should be used
    /// when going to the next entry in the queue is the wanted behavior.
    pub fn next(&self, manual: bool) {
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = self.get_repeat();

        if repeat == RepeatSetting::RepeatTrack && !manual {
            if let Some(index) = current {
                self.play(index, false);
            }
        } else if let Some(index) = self.next_index() {
            self.play(index, false);
            if repeat == RepeatSetting::RepeatTrack && manual {
                self.set_repeat(RepeatSetting::RepeatPlaylist);
            }
        } else if repeat == RepeatSetting::RepeatPlaylist && q.len() > 0 {
            self.play(0, false);
        } else {
            self.spotify.stop();
        }
    }

    /// Play the previous item in the queue.
    pub fn previous(&self) {
        let q = self.queue.read().unwrap();
        let current = *self.current_track.read().unwrap();
        let repeat = self.get_repeat();

        if let Some(index) = self.previous_index() {
            self.play(index, false);
        } else if repeat == RepeatSetting::RepeatPlaylist && q.len() > 0 {
            if self.get_shuffle() {
                self.play(0, false);
            } else {
                self.play(q.len() - 1, false);
            }
        } else if let Some(index) = current {
            self.play(index, false);
        }
    }

    /// Get the current repeat behavior.
    pub fn get_repeat(&self) -> RepeatSetting {
        self.cfg.state().repeat
    }

    /// Set the current repeat behavior and save it to the configuration.
    pub fn set_repeat(&self, new: RepeatSetting) {
        self.spotify.api.set_repeat(new, None);
        self.cfg.with_state_mut(|mut s| s.repeat = new);
    }

    /// Get the current shuffle behavior.
    pub fn get_shuffle(&self) -> bool {
        self.cfg.state().shuffle
    }

    /// Set the current shuffle behavior.
    pub fn set_shuffle(&self, new: bool) {
        self.spotify.api.set_shuffle(new, None);
        self.cfg.with_state_mut(|mut s| s.shuffle = new);
    }

    /// Get the spotify session.
    pub fn get_spotify(&self) -> Spotify {
        self.spotify.clone()
    }
}

/// Send a notification using the desktops default notification method.
///
/// `summary_txt`: A short title for the notification.
/// `body_txt`: The actual content of the notification.
/// `cover_url`: A URL to an image to show in the notification.
/// `notification_id`: Unique id for a notification, that can be used to operate
/// on a previous notification (for example to close it).
#[cfg(feature = "notify")]
pub fn send_notification(summary_txt: &str, body_txt: &str, cover_url: Option<String>) {
    let mut n = Notification::new();
    n.appname("ncspot").summary(summary_txt).body(body_txt);

    // album cover image
    if let Some(u) = cover_url {
        let path = crate::utils::cache_path_for_url(u.to_string());
        if !path.exists() {
            if let Err(e) = crate::utils::download(u, path.clone()) {
                log::error!("Failed to download cover: {}", e);
            }
        }
        n.icon(path.to_str().unwrap());
    }

    // XDG desktop entry hints
    #[cfg(all(unix, not(target_os = "macos")))]
    n.urgency(notify_rust::Urgency::Low)
        .hint(notify_rust::Hint::Transient(true))
        .hint(notify_rust::Hint::DesktopEntry("ncspot".into()));

    match n.show() {
        Ok(handle) => {
            // only available for XDG
            #[cfg(all(unix, not(target_os = "macos")))]
            info!("Created notification: {}", handle.id());
        }
        Err(e) => log::error!("Failed to send notification cover: {}", e),
    }
}
