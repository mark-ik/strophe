//! S2: the app state behind the serval UI — a real `strophe_model::Session`
//! plus its `History`. Every user gesture that touches session data goes
//! through `History::commit` with a real `Edit`, so undo/redo and (later)
//! sync see exactly what the UI did. State the model does not yet carry
//! (live capture flag, click audibility, solo) lives here as app-local
//! fields, each marked with where it graduates.

use std::collections::BTreeSet;

use strophe_model::{Edit, History, Layer, MediaRef, Phrase, Session, TrackId};

/// Owner labels for the pass-the-mic rail. Placeholder until the sync layer
/// brings real peers; indexed by track position like the rail's demo circle.
pub const OWNERS: [&str; 4] = ["you", "jonah", "mara", "eli"];

pub struct AppState {
    pub session: Session,
    pub history: History,
    /// Live capture on the armed track. App-local until the engine slice:
    /// toggling it off commits an `AppendLayer` (a placeholder phrase) so the
    /// record gesture exercises the real edit spine end to end.
    pub recording: bool,
    /// Click audibility. App-local: the model's `master_clock_enabled` covers
    /// clock + count-in semantics; a separate audible-click flag is a
    /// session-config candidate once the engine consumes it.
    pub click: bool,
    /// Solo set. App-local: the model has no solo yet (mix-bus concern, not
    /// session data) — graduates if solo becomes persistent session state.
    pub solo: BTreeSet<TrackId>,
}

impl AppState {
    /// The demo session: the looper-pedal default seeded through real edits
    /// (renames, colors, arm, appended layers with placeholder media) so the
    /// history graph is populated exactly as live use would populate it.
    pub fn demo() -> Self {
        let mut session = Session::new_default();
        let mut history = History::new();
        let mut t = 0u64;
        let mut commit = |edit: Edit, session: &mut Session, t: &mut u64| {
            *t += 1;
            history.commit(edit, session, *t);
        };

        let names = ["Guitar", "Bass", "Drums", "Keys"];
        // The approved palette: amber / teal / coral / sage.
        let colors = [
            strophe_model::TrackColor::rgb(0xe0, 0xa6, 0x4b),
            strophe_model::TrackColor::rgb(0x56, 0xb3, 0xa8),
            strophe_model::TrackColor::rgb(0xe0, 0x79, 0x6a),
            strophe_model::TrackColor::rgb(0xa9, 0xb9, 0x6b),
        ];
        for i in 0..session.tracks.len().min(4) {
            let track = &session.tracks[i];
            let (id, from_name, from_color) = (track.id, track.name.clone(), track.color);
            commit(
                Edit::RenameTrack { track_id: id, from: from_name, to: names[i].into() },
                &mut session,
                &mut t,
            );
            commit(
                Edit::SetTrackColor { track_id: id, from: from_color, to: colors[i] },
                &mut session,
                &mut t,
            );
        }

        // Layer stacks: Guitar 3 (oldest muted), Bass 2, Drums 4 (L2 muted),
        // Keys empty — the S1 demo shape, now real session data.
        let layer_counts = [3usize, 2, 4, 0];
        for (i, &n) in layer_counts.iter().enumerate() {
            let track_id = session.tracks[i].id;
            for _ in 0..n {
                let phrase =
                    Phrase::new(MediaRef::ZERO, session.bars_per_phrase, session.bpm, t);
                let layer = Layer::new(phrase.id);
                commit(Edit::AppendLayer { track_id, phrase, layer }, &mut session, &mut t);
            }
        }
        let mutes = [(0usize, 0u16), (2, 1)];
        for (track_idx, layer_index) in mutes {
            let track_id = session.tracks[track_idx].id;
            commit(
                Edit::SetLayerMute { track_id, layer_index, from: false, to: true },
                &mut session,
                &mut t,
            );
        }

        let guitar = session.tracks[0].id;
        commit(Edit::ArmTrack { track_id: guitar, from: false, to: true }, &mut session, &mut t);

        Self {
            session,
            history,
            recording: true,
            click: true,
            solo: BTreeSet::new(),
        }
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Index of the armed track, if any.
    pub fn armed_index(&self) -> Option<usize> {
        self.session.tracks.iter().position(|t| t.armed)
    }

    /// Arm track `idx` (and un-arm the previous holder). Stops a live
    /// capture first so the recording flag never points at an un-armed track.
    pub fn arm(&mut self, idx: usize) {
        if self.session.tracks.get(idx).is_none_or(|t| t.armed) {
            return;
        }
        if self.recording {
            self.stop_record();
        }
        let now = Self::now_ms();
        if let Some(prev) = self.armed_index() {
            let track_id = self.session.tracks[prev].id;
            self.history.commit(
                Edit::ArmTrack { track_id, from: true, to: false },
                &mut self.session,
                now,
            );
        }
        let track_id = self.session.tracks[idx].id;
        self.history.commit(
            Edit::ArmTrack { track_id, from: false, to: true },
            &mut self.session,
            now,
        );
    }

    /// The record gesture: start capture, or stop it. Stopping appends a
    /// layer (placeholder media until the engine slice) to the armed track
    /// through the real edit spine.
    pub fn toggle_record(&mut self) {
        if self.recording {
            self.stop_record();
        } else if self.armed_index().is_some() {
            self.recording = true;
        }
    }

    fn stop_record(&mut self) {
        self.recording = false;
        let Some(idx) = self.armed_index() else { return };
        let now = Self::now_ms();
        let track_id = self.session.tracks[idx].id;
        let phrase = Phrase::new(
            MediaRef::ZERO,
            self.session.bars_per_phrase,
            self.session.bpm,
            now,
        );
        let layer = Layer::new(phrase.id);
        self.history.commit(
            Edit::AppendLayer { track_id, phrase, layer },
            &mut self.session,
            now,
        );
    }

    /// Toggle track-level mute (the lane's M).
    pub fn toggle_track_mute(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else { return };
        let (track_id, from) = (track.id, track.muted);
        self.history.commit(
            Edit::MuteTrack { track_id, from, to: !from },
            &mut self.session,
            Self::now_ms(),
        );
    }

    /// Toggle one layer's mute (tap a layer row).
    pub fn toggle_layer_mute(&mut self, track_idx: usize, layer_index: u16) {
        let Some(track) = self.session.tracks.get(track_idx) else { return };
        let Some(layer) = track.layers.get(layer_index as usize) else { return };
        let (track_id, from) = (track.id, layer.muted);
        self.history.commit(
            Edit::SetLayerMute { track_id, layer_index, from, to: !from },
            &mut self.session,
            Self::now_ms(),
        );
    }

    /// Nudge the tempo by `delta` BPM (clamped to a playable range).
    pub fn bpm_nudge(&mut self, delta: f32) {
        let from = self.session.bpm;
        let to = (from + delta).clamp(30.0, 300.0);
        if to == from {
            return;
        }
        self.history.commit(
            Edit::SetBpm { from, to },
            &mut self.session,
            Self::now_ms(),
        );
    }

    /// Toggle the master clock (bar-aligned capture + count-in semantics).
    pub fn toggle_master_clock(&mut self) {
        let from = self.session.master_clock_enabled;
        self.history.commit(
            Edit::SetMasterClock { from, to: !from },
            &mut self.session,
            Self::now_ms(),
        );
    }

    /// Toggle the audible click (app-local, see field note).
    pub fn toggle_click(&mut self) {
        self.click = !self.click;
    }

    /// Toggle solo membership for track `idx` (app-local, see field note).
    pub fn toggle_solo(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else { return };
        let id = track.id;
        if !self.solo.remove(&id) {
            self.solo.insert(id);
        }
    }
}
