//! Local play queue: playback is driven entirely on this machine (no Spotify Connect
//! round trips), which keeps latency low and avoids any extra network request when
//! skipping tracks.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use rand::seq::SliceRandom;

use crate::model::{Repeat, Track};

#[derive(Default)]
pub struct Queue {
    context: Arc<Vec<Track>>,
    /// Play order, as indices into `context`.
    order: Vec<usize>,
    /// Position of the current context track in `order`.
    pos: usize,
    /// Tracks added with "Ajouter à la file", played before the context resumes.
    manual: VecDeque<Track>,
    current: Option<Track>,
    shuffle: bool,
    repeat: Repeat,
    /// Tracks Spotify refused to play: automatic playback skips them.
    blocked: HashSet<String>,
}

impl Queue {
    /// Remembers a track that cannot be played so that it is skipped from now on.
    pub fn block(&mut self, id: String) {
        self.blocked.insert(id);
    }

    pub fn set_blocked(&mut self, ids: HashSet<String>) {
        self.blocked = ids;
    }

    /// Can this track be reached by automatic playback (next, previous, preload)?
    fn reachable(&self, track: &Track) -> bool {
        track.playable && !self.blocked.contains(&track.id)
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref()
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// Replaces the context and starts at `start`. Returns the track to play.
    pub fn play_context(&mut self, context: Arc<Vec<Track>>, start: usize) -> Option<Track> {
        self.context = context;
        let n = self.context.len();
        let first = (start..n).chain(0..start.min(n)).find(|&i| self.context[i].playable);
        match first {
            Some(i) => {
                self.rebuild_order(i);
                self.current = Some(self.context[i].clone());
            }
            None => {
                self.order.clear();
                self.pos = 0;
                self.current = None;
            }
        }
        self.current.clone()
    }

    pub fn enqueue(&mut self, track: Track) {
        self.manual.push_back(track);
    }

    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;
        let current = self.order.get(self.pos).copied().unwrap_or(0);
        self.rebuild_order(current);
    }

    fn rebuild_order(&mut self, current: usize) {
        let n = self.context.len();
        if n == 0 {
            self.order.clear();
            self.pos = 0;
            return;
        }
        if self.shuffle {
            let mut rest: Vec<usize> = (0..n).filter(|&i| i != current).collect();
            rest.shuffle(&mut rand::rng());
            self.order = Vec::with_capacity(n);
            self.order.push(current);
            self.order.extend(rest);
            self.pos = 0;
        } else {
            self.order = (0..n).collect();
            self.pos = current;
        }
    }

    /// Moves to the next track. `auto` is true when the previous track ended by
    /// itself (repeat-one only applies in that case).
    pub fn advance(&mut self, auto: bool) -> Option<Track> {
        if auto && self.repeat == Repeat::One && self.current.is_some() {
            return self.current.clone();
        }
        if let Some(track) = self.manual.pop_front() {
            self.current = Some(track);
            return self.current.clone();
        }
        let n = self.order.len();
        for _ in 0..n {
            if self.pos + 1 < n {
                self.pos += 1;
            } else if self.repeat != Repeat::Off {
                if self.shuffle {
                    self.order.shuffle(&mut rand::rng());
                }
                self.pos = 0;
            } else {
                return None;
            }
            let track = &self.context[self.order[self.pos]];
            if self.reachable(track) {
                self.current = Some(track.clone());
                return self.current.clone();
            }
        }
        None
    }

    /// Moves to the previous context track.
    pub fn back(&mut self) -> Option<Track> {
        let n = self.order.len();
        for _ in 0..n {
            if self.pos > 0 {
                self.pos -= 1;
            } else if self.repeat != Repeat::Off {
                self.pos = n - 1;
            } else {
                return None;
            }
            let track = &self.context[self.order[self.pos]];
            if self.reachable(track) {
                self.current = Some(track.clone());
                return self.current.clone();
            }
        }
        None
    }

    /// Track that will play after the current one, used for gapless preloading.
    pub fn peek_next(&self) -> Option<&Track> {
        if self.repeat == Repeat::One {
            return self.current.as_ref();
        }
        if let Some(track) = self.manual.front() {
            return Some(track);
        }
        let n = self.order.len();
        let mut pos = self.pos;
        for _ in 0..n {
            pos += 1;
            if pos >= n {
                if self.repeat == Repeat::Off || self.shuffle {
                    // A reshuffle happens on wrap-around: the next track is unknown.
                    return None;
                }
                pos = 0;
            }
            let track = &self.context[self.order[pos]];
            if self.reachable(track) {
                return Some(track);
            }
        }
        None
    }

    /// Upcoming tracks (manual queue first), for the "File d'attente" view.
    pub fn upcoming(&self, max: usize) -> Vec<Track> {
        let mut out: Vec<Track> = self.manual.iter().take(max).cloned().collect();
        let rest = max.saturating_sub(out.len());
        out.extend(
            self.order
                .iter()
                .skip(self.pos + 1)
                .map(|&i| &self.context[i])
                .filter(|t| self.reachable(t))
                .take(rest)
                .cloned(),
        );
        out
    }

    pub fn clear_manual(&mut self) {
        self.manual.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(n: usize) -> Arc<Vec<Track>> {
        Arc::new(
            (0..n)
                .map(|i| Track {
                    id: format!("t{i}"),
                    name: format!("Track {i}"),
                    playable: true,
                    ..Default::default()
                })
                .collect(),
        )
    }

    fn id(t: Option<Track>) -> String {
        t.map(|t| t.id).unwrap_or_default()
    }

    #[test]
    fn plays_in_order_and_stops() {
        let mut q = Queue::default();
        assert_eq!(id(q.play_context(tracks(3), 1)), "t1");
        assert_eq!(q.peek_next().map(|t| t.id.as_str()), Some("t2"));
        assert_eq!(id(q.advance(true)), "t2");
        assert_eq!(q.advance(true), None);
        assert_eq!(id(q.back()), "t1");
        assert_eq!(id(q.back()), "t0");
        assert_eq!(q.back(), None);
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_sticks() {
        let mut q = Queue::default();
        q.set_repeat(Repeat::All);
        q.play_context(tracks(2), 1);
        assert_eq!(q.peek_next().map(|t| t.id.as_str()), Some("t0"));
        assert_eq!(id(q.advance(true)), "t0");
        q.set_repeat(Repeat::One);
        assert_eq!(id(q.advance(true)), "t0");
        // A manual skip still moves forward.
        assert_eq!(id(q.advance(false)), "t1");
    }

    #[test]
    fn manual_queue_has_priority() {
        let mut q = Queue::default();
        q.play_context(tracks(3), 0);
        let extra = Track { id: "x".into(), playable: true, ..Default::default() };
        q.enqueue(extra);
        assert_eq!(q.peek_next().map(|t| t.id.as_str()), Some("x"));
        assert_eq!(q.upcoming(10).len(), 3);
        assert_eq!(id(q.advance(true)), "x");
        assert_eq!(id(q.advance(true)), "t1");
    }

    #[test]
    fn skips_unplayable_tracks() {
        let mut list = (*tracks(4)).clone();
        list[1].playable = false;
        list[0].playable = false;
        let mut q = Queue::default();
        assert_eq!(id(q.play_context(Arc::new(list), 0)), "t2");
        assert_eq!(id(q.advance(true)), "t3");
    }

    #[test]
    fn blocked_tracks_are_skipped_but_can_still_be_chosen() {
        let mut q = Queue::default();
        q.block("t1".into());
        q.block("t2".into());
        // An explicit choice is honoured…
        assert_eq!(id(q.play_context(tracks(4), 1)), "t1");
        // …but automatic playback skips refused tracks.
        assert_eq!(q.peek_next().map(|t| t.id.as_str()), Some("t3"));
        assert_eq!(id(q.advance(true)), "t3");
        assert_eq!(id(q.back()), "t0");
        assert!(q.upcoming(10).iter().all(|t| t.id != "t1" && t.id != "t2"));
    }

    #[test]
    fn shuffle_keeps_current_and_visits_everything_once() {
        let mut q = Queue::default();
        q.set_shuffle(true);
        assert_eq!(id(q.play_context(tracks(20), 7)), "t7");
        let mut seen = vec!["t7".to_string()];
        while let Some(t) = q.advance(true) {
            seen.push(t.id);
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 20);
    }

    #[test]
    fn toggling_shuffle_keeps_current_track() {
        let mut q = Queue::default();
        q.play_context(tracks(10), 4);
        q.set_shuffle(true);
        assert_eq!(q.current().map(|t| t.id.as_str()), Some("t4"));
        q.set_shuffle(false);
        assert_eq!(id(q.advance(true)), "t5");
    }
}
