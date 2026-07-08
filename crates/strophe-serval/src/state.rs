//! The app state behind the serval UI: the real `strophe_model::Session` +
//! `History`, plus the audio `strophe_engine::Engine` and its content store.
//!
//! Every data-bearing gesture commits a real `Edit`, so undo/redo + the future
//! sync layer see exactly what the UI did; the engine-driving methods mirror the
//! Masonry app's (`record` / `arm` / `stop_all` / `play_layer_from_store` / …)
//! so audio behaviour is a fixed reference across the two hosts. The `Engine` is
//! `!Send`; winit runs the app on the main thread, so it lives directly here and
//! is advanced by [`tick`](AppState::tick) from the host's ~60fps timer.
//!
//! Demo note: the initial session is seeded with silent placeholder layers
//! (`MediaRef::ZERO`) for the approved populated look — they render but do not
//! play. A real Record captures audio, appends a playable layer, and loops it.

use std::collections::BTreeSet;

use strophe_engine::media::{InMemoryStore, MediaStore};
use strophe_engine::{CapturePhase, Engine, LayerKey};
use strophe_model::{Edit, History, Layer, MediaRef, Phrase, Session, TrackId};

/// Owner labels for the pass-the-mic rail. Placeholder until the sync layer.
pub const OWNERS: [&str; 4] = ["you", "jonah", "mara", "eli"];

/// Bars captured per Record press (master-clock / bar-aligned mode).
const CAPTURE_BARS: u8 = 1;
/// Meter dB floor for the 0..1 level the output meters display.
const METER_FLOOR_DB: f32 = -60.0;

pub struct AppState {
    pub session: Session,
    pub history: History,

    // === audio ===
    engine: Result<Engine, String>,
    sample_rate: u32,
    store: InMemoryStore,
    capture_phase: CapturePhase,
    /// The track the in-flight capture targets (snapshot at Record time, so
    /// re-arming mid-capture does not redirect it).
    capturing_track: Option<usize>,
    /// Engine layer keys currently looping, so stop-all / mute can stop them.
    playing: Vec<LayerKey>,
    /// Latest output peak, dB per channel, read back each tick.
    meter_db: [f32; 2],

    // === app-local UI (graduation notes) ===
    /// Audible metronome. Drives the engine click directly; distinct from the
    /// master clock (which governs bar-aligned capture + count-in).
    pub click: bool,
    /// Solo set. App-local: the model has no solo yet (a mix-bus concern).
    pub solo: BTreeSet<TrackId>,
}

impl AppState {
    /// The demo session: the looper-pedal default, seeded through real edits so
    /// the history graph is populated exactly as live use would. Seeded layers
    /// carry placeholder media (silent); the engine is live for real captures.
    pub fn demo() -> Self {
        let session = Session::new_default();
        let (engine, sample_rate) = match Engine::new() {
            Ok(e) => {
                let sr = e.sample_rate();
                (Ok(e), sr)
            }
            Err(e) => (Err(e.to_string()), 0),
        };
        let mut state = Self {
            session,
            history: History::new(),
            engine,
            sample_rate,
            store: InMemoryStore::new(),
            capture_phase: CapturePhase::Idle,
            capturing_track: None,
            playing: Vec::new(),
            meter_db: [f32::NEG_INFINITY; 2],
            click: true,
            solo: BTreeSet::new(),
        };
        state.seed_demo();
        // Apply the initial click + tempo to the engine.
        state.resync_tempo();
        state
    }

    /// Seed the approved demo shape (Guitar/Bass/Drums/Keys, owner colours,
    /// layer stacks, Guitar armed) through real commits.
    fn seed_demo(&mut self) {
        let mut t = 0u64;
        let names = ["Guitar", "Bass", "Drums", "Keys"];
        let colors = [
            strophe_model::TrackColor::rgb(0xe0, 0xa6, 0x4b),
            strophe_model::TrackColor::rgb(0x56, 0xb3, 0xa8),
            strophe_model::TrackColor::rgb(0xe0, 0x79, 0x6a),
            strophe_model::TrackColor::rgb(0xa9, 0xb9, 0x6b),
        ];
        for i in 0..self.session.tracks.len().min(4) {
            let track = &self.session.tracks[i];
            let (id, from_name, from_color) = (track.id, track.name.clone(), track.color);
            t += 1;
            self.history.commit(
                Edit::RenameTrack { track_id: id, from: from_name, to: names[i].into() },
                &mut self.session,
                t,
            );
            t += 1;
            self.history.commit(
                Edit::SetTrackColor { track_id: id, from: from_color, to: colors[i] },
                &mut self.session,
                t,
            );
        }
        let layer_counts = [3usize, 2, 4, 0];
        for (i, &n) in layer_counts.iter().enumerate() {
            let track_id = self.session.tracks[i].id;
            for _ in 0..n {
                t += 1;
                let phrase = Phrase::new(MediaRef::ZERO, self.session.bars_per_phrase, self.session.bpm, t);
                let layer = Layer::new(phrase.id);
                self.history.commit(Edit::AppendLayer { track_id, phrase, layer }, &mut self.session, t);
            }
        }
        for (track_idx, layer_index) in [(0usize, 0u16), (2, 1)] {
            let track_id = self.session.tracks[track_idx].id;
            t += 1;
            self.history.commit(
                Edit::SetLayerMute { track_id, layer_index, from: false, to: true },
                &mut self.session,
                t,
            );
        }
        let guitar = self.session.tracks[0].id;
        t += 1;
        self.history.commit(Edit::ArmTrack { track_id: guitar, from: false, to: true }, &mut self.session, t);
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    // === derived state the view reads ===

    pub fn armed_index(&self) -> Option<usize> {
        self.session.tracks.iter().position(|t| t.armed)
    }

    /// Whether a capture is in progress (drives the record light + rail).
    pub fn is_recording(&self) -> bool {
        matches!(
            self.capture_phase,
            CapturePhase::Waiting { .. } | CapturePhase::Recording { .. } | CapturePhase::FreeRecording { .. }
        )
    }

    /// The output meter level `0..1` for channel `ch` (0 = L, 1 = R), mapped
    /// from the read-back dB against the display floor.
    pub fn meter_level(&self, ch: usize) -> f32 {
        let db = self.meter_db.get(ch).copied().unwrap_or(f32::NEG_INFINITY);
        if db <= METER_FLOOR_DB {
            0.0
        } else {
            ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
        }
    }

    // === the engine tick (host calls ~60fps via runner.update) ===

    /// Advance the engine, read back the meter + capture phase, and promote a
    /// completed capture into a model layer + looping playback.
    pub fn tick(&mut self) {
        let (meter, phase, captured) = match &mut self.engine {
            Ok(engine) => {
                let _ = engine.tick();
                (engine.peak_db(), engine.pending_capture_progress(), engine.take_bar_aligned_capture())
            }
            Err(_) => (self.meter_db, self.capture_phase.clone(), None),
        };
        self.meter_db = meter;
        self.capture_phase = phase;

        if let Some(samples) = captured {
            let Some(track_idx) = self.capturing_track.take() else { return };
            // A free capture stopped instantly yields an empty buffer — skip.
            if samples.is_empty() || track_idx >= self.session.tracks.len() {
                return;
            }
            let media = self.store.put(&samples, self.sample_rate);
            let phrase = Phrase::new(media, self.session.bars_per_phrase, self.session.bpm, Self::now_ms());
            let layer = Layer::new(phrase.id);
            let track_id = self.session.tracks[track_idx].id;
            let layer_index = self.session.tracks[track_idx].layers.len() as u16;
            self.history.commit(
                Edit::AppendLayer { track_id, phrase, layer },
                &mut self.session,
                Self::now_ms(),
            );
            let key = LayerKey::new(track_id, layer_index);
            let clocked = self.session.master_clock_enabled;
            if let Ok(engine) = &mut self.engine {
                let _ = if clocked {
                    engine.play_layer_at_next_bar(key, samples, 1.0, true)
                } else {
                    engine.play_layer(key, samples, 1.0, true)
                };
            }
            self.playing.push(key);
        }
    }

    // === gestures (commit a real Edit, then drive the engine) ===

    /// Arm track `idx` (single-arm: unarm the previous holder). Stops a live
    /// capture first so the flag never points at an un-armed track.
    pub fn arm(&mut self, idx: usize) {
        if self.session.tracks.get(idx).is_none_or(|t| t.armed) {
            return;
        }
        if self.is_recording() {
            self.stop_capture();
        }
        let now = Self::now_ms();
        if let Some(prev) = self.armed_index() {
            let track_id = self.session.tracks[prev].id;
            self.history
                .commit(Edit::ArmTrack { track_id, from: true, to: false }, &mut self.session, now);
        }
        let track_id = self.session.tracks[idx].id;
        self.history
            .commit(Edit::ArmTrack { track_id, from: false, to: true }, &mut self.session, now);
    }

    /// The Record gesture. Master clock on → a bar-aligned, count-in, fixed
    /// capture. Master clock off → toggle a free/variable-length capture.
    pub fn toggle_record(&mut self) {
        let Some(armed) = self.armed_index() else { return };
        if self.session.master_clock_enabled {
            let count_in = self.session.count_in_bars;
            if let Ok(engine) = &mut self.engine {
                if engine.arm_bar_aligned_capture(CAPTURE_BARS, count_in).is_ok() {
                    self.capturing_track = Some(armed);
                }
            }
        } else if matches!(self.capture_phase, CapturePhase::FreeRecording { .. }) {
            self.stop_capture();
        } else if let Ok(engine) = &mut self.engine {
            if engine.start_free_capture().is_ok() {
                self.capturing_track = Some(armed);
            }
        }
    }

    /// Stop an in-flight free capture (the tick picks up the Complete buffer).
    fn stop_capture(&mut self) {
        if let Ok(engine) = &mut self.engine {
            engine.stop_free_capture();
        }
    }

    /// Toggle track-level mute (the lane's M): stop / replay the track's voices.
    pub fn toggle_track_mute(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else { return };
        let (track_id, from) = (track.id, track.muted);
        self.history
            .commit(Edit::MuteTrack { track_id, from, to: !from }, &mut self.session, Self::now_ms());
        if !from {
            // Now muted: stop every voice on this track.
            let keys: Vec<LayerKey> = self
                .playing
                .iter()
                .copied()
                .filter(|k| k.track_id == track_id)
                .collect();
            for key in keys {
                self.stop_layer_key(key);
            }
        } else {
            self.reconcile_track_playback(idx);
        }
    }

    /// Toggle one layer's mute (tap a layer row): stop / replay that voice.
    pub fn toggle_layer_mute(&mut self, track_idx: usize, layer_index: u16) {
        let Some(track) = self.session.tracks.get(track_idx) else { return };
        let track_id = track.id;
        let Some(layer) = track.layers.get(layer_index as usize) else { return };
        let from = layer.muted;
        self.history.commit(
            Edit::SetLayerMute { track_id, layer_index, from, to: !from },
            &mut self.session,
            Self::now_ms(),
        );
        let key = LayerKey::new(track_id, layer_index);
        if !from {
            self.stop_layer_key(key);
        } else if !self.session.tracks[track_idx].muted {
            self.play_layer_from_store(track_idx, layer_index as usize);
        }
    }

    /// Nudge tempo (clamped) and re-sync the engine grid.
    pub fn bpm_nudge(&mut self, delta: f32) {
        let from = self.session.bpm;
        let to = (from + delta).clamp(40.0, 240.0);
        if (to - from).abs() < f32::EPSILON {
            return;
        }
        self.history
            .commit(Edit::SetBpm { from, to }, &mut self.session, Self::now_ms());
        self.resync_tempo();
    }

    /// Toggle the master clock (bar-aligned capture + count-in). Model-only;
    /// the audible metronome is the separate [`toggle_click`](Self::toggle_click).
    pub fn toggle_master_clock(&mut self) {
        let from = self.session.master_clock_enabled;
        self.history
            .commit(Edit::SetMasterClock { from, to: !from }, &mut self.session, Self::now_ms());
    }

    /// Toggle the audible metronome click on the engine.
    pub fn toggle_click(&mut self) {
        self.click = !self.click;
        if let Ok(engine) = &mut self.engine {
            engine.set_click_enabled(self.click);
        }
    }

    /// Toggle solo membership for track `idx` (app-local; no engine backing yet).
    pub fn toggle_solo(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else { return };
        let id = track.id;
        if !self.solo.remove(&id) {
            self.solo.insert(id);
        }
    }

    // === engine playback helpers (mirror the Masonry app) ===

    /// Push tempo + bar length to the click and re-apply the click enable.
    /// Stops all playing loops first — they were captured at the old grid and
    /// would drift.
    fn resync_tempo(&mut self) {
        self.stop_all();
        let bpm = self.session.bpm;
        let beats = self.session.time_signature.numerator;
        let click = self.click;
        if let Ok(engine) = &mut self.engine {
            let _ = engine.set_tempo(bpm, beats);
            engine.set_click_enabled(click);
        }
    }

    fn stop_all(&mut self) {
        let keys: Vec<LayerKey> = self.playing.drain(..).collect();
        if let Ok(engine) = &mut self.engine {
            for key in keys {
                engine.stop_layer(key);
            }
        }
    }

    fn stop_layer_key(&mut self, key: LayerKey) {
        if let Ok(engine) = &mut self.engine {
            engine.stop_layer(key);
        }
        self.playing.retain(|k| *k != key);
    }

    /// (Re)play every audible layer of track `idx` from the store.
    fn reconcile_track_playback(&mut self, idx: usize) {
        let Some(track) = self.session.tracks.get(idx) else { return };
        let audible: Vec<usize> = track
            .layers
            .iter()
            .enumerate()
            .filter(|(li, layer)| track.playback_mode.is_layer_audible(*li as u16, layer.muted))
            .map(|(li, _)| li)
            .collect();
        for li in audible {
            self.play_layer_from_store(idx, li);
        }
    }

    /// Play a layer from the store at the next bar, at its stored gain. No-op
    /// if the layer's media is absent (a silent demo placeholder).
    fn play_layer_from_store(&mut self, track_idx: usize, layer_idx: usize) {
        let Some(track) = self.session.tracks.get(track_idx) else { return };
        let Some(layer) = track.layers.get(layer_idx) else { return };
        let Some(phrase) = self.session.phrases.get(&layer.phrase_id) else { return };
        let Some(buf) = self.store.get(&phrase.media) else { return };
        let samples = buf.samples.clone();
        let (gain, key) = (layer.gain, LayerKey::new(track.id, layer_idx as u16));
        if let Ok(engine) = &mut self.engine {
            let _ = engine.play_layer_at_next_bar(key, samples, gain, true);
        }
        if !self.playing.contains(&key) {
            self.playing.push(key);
        }
    }
}
